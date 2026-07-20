[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Uri,

    [Parameter(Mandatory = $true)]
    [string]$TokenEnvVar,

    [Parameter(Mandatory = $true)]
    [string]$LogPath,

    [string]$ResumeThreadId,

    [Parameter(Mandatory = $true)]
    [string]$ReadyPath,

    [Parameter(Mandatory = $true)]
    [string]$ResumeMarkerPath,

    [Parameter(Mandatory = $true)]
    [string]$ServerRequestMarkerPath,

    [Parameter(Mandatory = $true)]
    [string]$SafetyExitIntentPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-NestedValue {
    param(
        [object]$Value,
        [string[]]$Path
    )

    $current = $Value
    foreach ($part in $Path) {
        if ($null -eq $current) {
            return $null
        }

        if ($current -is [Collections.IDictionary]) {
            if (-not $current.Contains($part)) {
                return $null
            }
            $current = $current[$part]
        } else {
            $property = $current.PSObject.Properties[$part]
            if ($null -eq $property) {
                return $null
            }
            $current = $property.Value
        }
    }
    return $current
}

function Write-ReadyMarker {
    [IO.File]::WriteAllText(
        $ReadyPath,
        [DateTimeOffset]::UtcNow.ToString("o"),
        [Text.Encoding]::ASCII
    )
}

function Write-SanitizedRecord {
    param(
        [string]$Direction,
        [object]$Message,
        [bool]$RequiresResponse = $false
    )

    $method = Get-NestedValue -Value $Message -Path @("method")
    $id = Get-NestedValue -Value $Message -Path @("id")
    $status = Get-NestedValue -Value $Message -Path @("params", "turn", "status")
    if ($null -eq $status) {
        $status = Get-NestedValue -Value $Message -Path @("params", "status")
    }

    $threadId = Get-NestedValue -Value $Message -Path @("params", "thread", "id")
    if ($null -eq $threadId) {
        $threadId = Get-NestedValue -Value $Message -Path @("params", "threadId")
    }
    if ($null -eq $threadId) {
        $threadId = Get-NestedValue -Value $Message -Path @("result", "thread", "id")
    }

    $turnId = Get-NestedValue -Value $Message -Path @("params", "turn", "id")
    if ($null -eq $turnId) {
        $turnId = Get-NestedValue -Value $Message -Path @("params", "turnId")
    }

    $kind = if ($RequiresResponse) {
        "server_request"
    } elseif ($null -ne $method -and $null -eq $id) {
        "notification"
    } elseif ($null -ne $method) {
        "request"
    } else {
        "response"
    }

    $record = [ordered]@{
        timestamp = [DateTimeOffset]::UtcNow.ToString("o")
        direction = $Direction
        kind = $kind
        method = $method
        id = $id
        threadId = $threadId
        turnId = $turnId
        status = $status
        requiresResponse = $RequiresResponse
        hasError = ($null -ne (Get-NestedValue -Value $Message -Path @("error")))
    }

    $record | ConvertTo-Json -Compress | Add-Content -LiteralPath $LogPath -Encoding UTF8
}

function Send-JsonMessage {
    param(
        [System.Net.WebSockets.ClientWebSocket]$Socket,
        [object]$Message
    )

    $json = $Message | ConvertTo-Json -Depth 12 -Compress
    $bytes = [Text.Encoding]::UTF8.GetBytes($json)
    $segment = [ArraySegment[byte]]::new($bytes)
    [void]$Socket.SendAsync(
        $segment,
        [System.Net.WebSockets.WebSocketMessageType]::Text,
        $true,
        [Threading.CancellationToken]::None
    ).GetAwaiter().GetResult()
    Write-SanitizedRecord -Direction "outbound" -Message $Message
}

function Receive-JsonMessage {
    param([System.Net.WebSockets.ClientWebSocket]$Socket)

    $buffer = New-Object byte[] 16384
    $stream = [IO.MemoryStream]::new()
    try {
        do {
            $segment = [ArraySegment[byte]]::new($buffer)
            $result = $Socket.ReceiveAsync(
                $segment,
                [Threading.CancellationToken]::None
            ).GetAwaiter().GetResult()

            if ($result.MessageType -eq [System.Net.WebSockets.WebSocketMessageType]::Close) {
                return $null
            }
            if ($result.MessageType -ne [System.Net.WebSockets.WebSocketMessageType]::Text) {
                throw "Observer received a non-text WebSocket frame."
            }
            $stream.Write($buffer, 0, $result.Count)
        } while (-not $result.EndOfMessage)

        $json = [Text.Encoding]::UTF8.GetString($stream.ToArray())
        return $json | ConvertFrom-Json
    } finally {
        $stream.Dispose()
    }
}

$token = [Environment]::GetEnvironmentVariable($TokenEnvVar, "Process")
if ([string]::IsNullOrWhiteSpace($token)) {
    throw "Environment variable '$TokenEnvVar' is missing."
}

$logDirectory = Split-Path -Parent $LogPath
if (-not [string]::IsNullOrWhiteSpace($logDirectory)) {
    New-Item -ItemType Directory -Path $logDirectory -Force | Out-Null
}

$socket = [System.Net.WebSockets.ClientWebSocket]::new()
$socket.Options.SetRequestHeader("Authorization", "Bearer $token")
$safetyStopRequested = $false

try {
    [void]$socket.ConnectAsync([Uri]$Uri, [Threading.CancellationToken]::None).GetAwaiter().GetResult()

    $initialize = [ordered]@{
        method = "initialize"
        id = 1
        params = [ordered]@{
            clientInfo = [ordered]@{
                name = "keylink_studio_gate_a_observer"
                title = "Keylink Studio Gate A Observer"
                version = "0.1.0"
            }
            capabilities = [ordered]@{
                experimentalApi = $true
            }
        }
    }
    Send-JsonMessage -Socket $socket -Message $initialize

    $initialized = $false
    while ($socket.State -eq [System.Net.WebSockets.WebSocketState]::Open) {
        $message = Receive-JsonMessage -Socket $socket
        if ($null -eq $message) {
            break
        }

        $method = Get-NestedValue -Value $message -Path @("method")
        $id = Get-NestedValue -Value $message -Path @("id")
        $isServerRequest = ($null -ne $method -and $null -ne $id)
        Write-SanitizedRecord -Direction "inbound" -Message $message -RequiresResponse $isServerRequest

        if ($isServerRequest) {
            [IO.File]::WriteAllText(
                $ServerRequestMarkerPath,
                [string]$method,
                [Text.Encoding]::ASCII
            )
            [Console]::Error.WriteLine("Observer received response-required server request '$method'. No response was sent.")
            $safetyStopRequested = $true
            break
        }

        if (-not $initialized -and $id -eq 1) {
            if ($null -ne (Get-NestedValue -Value $message -Path @("error"))) {
                throw "Observer initialize failed. See the sanitized log."
            }

            Send-JsonMessage -Socket $socket -Message ([ordered]@{
                method = "initialized"
                params = [ordered]@{}
            })
            $initialized = $true

            if (-not [string]::IsNullOrWhiteSpace($ResumeThreadId)) {
                Send-JsonMessage -Socket $socket -Message ([ordered]@{
                    method = "thread/resume"
                    id = 2
                    params = [ordered]@{
                        threadId = $ResumeThreadId
                    }
                })
            } else {
                Write-ReadyMarker
            }
        } elseif ($initialized -and $id -eq 2 -and -not [string]::IsNullOrWhiteSpace($ResumeThreadId)) {
            if ($null -ne (Get-NestedValue -Value $message -Path @("error"))) {
                throw "Observer thread/resume failed. See the sanitized log."
            }
            $actualThreadId = Get-NestedValue -Value $message -Path @("result", "thread", "id")
            if ([string]::IsNullOrWhiteSpace([string]$actualThreadId) -or [string]$actualThreadId -ne $ResumeThreadId) {
                throw "Observer thread/resume response did not return the requested thread ID."
            }
            $resumeResult = [ordered]@{
                succeeded = $true
                requestedThreadId = $ResumeThreadId
                actualThreadId = [string]$actualThreadId
                responseId = 2
                completedAt = [DateTimeOffset]::UtcNow.ToString("o")
            } | ConvertTo-Json -Compress
            [IO.File]::WriteAllText($ResumeMarkerPath, $resumeResult, [Text.Encoding]::UTF8)
            Write-ReadyMarker
        }
    }
} finally {
    if ($safetyStopRequested) {
        $socket.Abort()
    } elseif ($socket.State -eq [System.Net.WebSockets.WebSocketState]::Open) {
        [void]$socket.CloseAsync(
            [System.Net.WebSockets.WebSocketCloseStatus]::NormalClosure,
            "Gate A observer stopped",
            [Threading.CancellationToken]::None
        ).GetAwaiter().GetResult()
    }
    $socket.Dispose()
}

if ($safetyStopRequested) {
    $exitIntent = [ordered]@{
        exitCode = 42
        writtenAt = [DateTimeOffset]::UtcNow.ToString("o")
    } | ConvertTo-Json -Compress
    [IO.File]::WriteAllText($SafetyExitIntentPath, $exitIntent, [Text.Encoding]::UTF8)
    exit 42
}
