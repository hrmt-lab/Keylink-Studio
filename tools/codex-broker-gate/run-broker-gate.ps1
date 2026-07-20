[CmdletBinding()]
param(
    [ValidateRange(1024, 65534)][int]$AppServerPort = 4500,
    [ValidateRange(1025, 65535)][int]$BrokerPort = 4501,
    [string]$CodexPath = "codex"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if ($AppServerPort -eq $BrokerPort) { throw "App Server and Broker ports must be different." }

# Some orchestrated Windows shells inject both Path and PATH. Start-Process uses
# a case-insensitive dictionary and fails before process creation in that state.
# Normalize only this harness process; no persistent environment is changed.
$processEnvironment = [Environment]::GetEnvironmentVariables("Process")
$pathEntries = @($processEnvironment.GetEnumerator() | Where-Object { [string]$_.Key -ieq "Path" })
if ($pathEntries.Count -gt 1) {
    $combinedPath = ($pathEntries | ForEach-Object { [string]$_.Value } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }) -join ";"
    foreach ($entry in $pathEntries) { [Environment]::SetEnvironmentVariable([string]$entry.Key, $null, "Process") }
    [Environment]::SetEnvironmentVariable("Path", $combinedPath, "Process")
}

$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = [IO.Path]::GetFullPath((Join-Path $scriptDirectory "..\.."))
$targetRoot = Join-Path $repoRoot "target\codex-broker-gate"
$runId = [DateTimeOffset]::Now.ToString("yyyyMMdd-HHmmss") + "-" + [Guid]::NewGuid().ToString("N").Substring(0, 8)
$runDirectory = Join-Path $targetRoot "runs\$runId"
$metadataLog = Join-Path $runDirectory "broker-metadata.jsonl"
$runJson = Join-Path $runDirectory "run.json"
$appServerStdout = Join-Path $runDirectory "app-server.stdout.log"
$appServerStderr = Join-Path $runDirectory "app-server.stderr.log"
$brokerStdout = Join-Path $runDirectory "broker.stdout.log"
$brokerStderr = Join-Path $runDirectory "broker.stderr.log"
$appServerUri = "ws://127.0.0.1:$AppServerPort"
$brokerUri = "ws://127.0.0.1:$BrokerPort"
$nodePath = (Get-Command node -ErrorAction Stop).Source
$appServerProcess = $null
$brokerProcess = $null
$appToken = $null
$brokerToken = $null
$cliExitCode = $null
$status = "failed"
$failure = $null
$startedAt = [DateTimeOffset]::UtcNow

function Require-Option([string]$Text, [string]$Option, [string]$Source) {
    $pattern = "(?m)^\s*(?:-[A-Za-z0-9],\s*)?" + [regex]::Escape($Option) + "(?:\s|=|$)"
    if (-not [regex]::IsMatch($Text, $pattern)) { throw "$Source does not contain required option: $Option" }
}

function Test-PortAvailable([int]$Port) {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, $Port)
    try { $listener.Start(); return $true } catch [Net.Sockets.SocketException] { return $false } finally { $listener.Stop() }
}

function Wait-HttpReady([Diagnostics.Process]$Process, [int]$Port, [string]$StderrPath) {
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $Process.Refresh()
        if ($Process.HasExited) {
            $detail = if (Test-Path -LiteralPath $StderrPath) { (Get-Content -LiteralPath $StderrPath -Encoding UTF8 | Select-Object -Last 20) -join "`n" } else { "No stderr captured." }
            throw "Process exited before port $Port became ready. Exit code: $($Process.ExitCode)`n$detail"
        }
        try {
            $response = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/readyz" -UseBasicParsing -TimeoutSec 1
            if ($response.StatusCode -eq 200) { return }
        } catch { Start-Sleep -Milliseconds 100 }
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Port $Port did not become ready within 10 seconds."
}

function Wait-BrokerReady([Diagnostics.Process]$Process) {
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $Process.Refresh()
        if ($Process.HasExited) {
            $detail = if (Test-Path -LiteralPath $brokerStderr) { (Get-Content -LiteralPath $brokerStderr -Encoding UTF8 | Select-Object -Last 20) -join "`n" } else { "No stderr captured." }
            throw "Broker exited before becoming ready. Exit code: $($Process.ExitCode)`n$detail"
        }
        if (Test-Path -LiteralPath $metadataLog) {
            $ready = Get-Content -LiteralPath $metadataLog -Encoding UTF8 | ForEach-Object { try { $_ | ConvertFrom-Json } catch { $null } } | Where-Object { $_.event -eq "broker_ready" } | Select-Object -First 1
            if ($null -ne $ready) { return }
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Broker did not become ready within 10 seconds."
}

function New-PrivateTokenFile([string]$Name) {
    $directory = Join-Path ([IO.Path]::GetTempPath()) ("keylink-codex-broker-" + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
    $bytes = New-Object byte[] 32
    [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
    $token = [Convert]::ToBase64String($bytes).TrimEnd("=").Replace("+", "-").Replace("/", "_")
    $file = Join-Path $directory $Name
    [IO.File]::WriteAllText($file, $token, [Text.Encoding]::ASCII)
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent().User
    $acl = [Security.AccessControl.FileSecurity]::new()
    $acl.SetOwner($identity)
    $acl.SetAccessRuleProtection($true, $false)
    $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($identity, [Security.AccessControl.FileSystemRights]::FullControl, [Security.AccessControl.AccessControlType]::Allow))
    Set-Acl -LiteralPath $file -AclObject $acl
    return [pscustomobject]@{ Directory = $directory; Path = $file }
}

function Stop-Tree([Diagnostics.Process]$Process) {
    if ($null -eq $Process) { return }
    $Process.Refresh()
    if ($Process.HasExited) { return }
    $savedPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        & taskkill.exe /PID $Process.Id /T /F 1>$null 2>$null
    } catch {
        # Exact-root fallback below is still attempted.
    } finally {
        $ErrorActionPreference = $savedPreference
    }
    Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue
}

function Write-RunJson([Collections.IDictionary]$Evidence) {
    $scripts = Get-ChildItem -LiteralPath $scriptDirectory -File | Sort-Object Name | ForEach-Object {
        [ordered]@{ file = $_.Name; sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash }
    }
    $fingerprintText = ($scripts | ForEach-Object { "$($_.file)=$($_.sha256)" }) -join "`n"
    $fingerprintBytes = [Text.Encoding]::UTF8.GetBytes($fingerprintText)
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        $fingerprint = ([BitConverter]::ToString($sha256.ComputeHash($fingerprintBytes))).Replace("-", "")
    } finally {
        $sha256.Dispose()
    }
    $record = [ordered]@{
        runId = $runId
        status = $status
        startedAt = $startedAt.ToString("o")
        finishedAt = [DateTimeOffset]::UtcNow.ToString("o")
        codexVersion = $codexVersion
        appServerUri = $appServerUri
        brokerUri = $brokerUri
        cliExitCode = $cliExitCode
        harnessSha256 = $fingerprint
        scripts = @($scripts)
        evidence = $Evidence
        failure = $failure
    }
    $temporary = "$runJson.tmp"
    [IO.File]::WriteAllText($temporary, ($record | ConvertTo-Json -Depth 8), [Text.Encoding]::UTF8)
    Move-Item -LiteralPath $temporary -Destination $runJson -Force
    return $fingerprint
}

New-Item -ItemType Directory -Path $runDirectory -Force | Out-Null
$codexVersion = (& $CodexPath --version 2>&1 | Out-String).Trim()
$cliHelp = (& $CodexPath --help 2>&1 | Out-String)
$appHelp = (& $CodexPath app-server --help 2>&1 | Out-String)
Require-Option $cliHelp "--remote" "codex --help"
Require-Option $cliHelp "--remote-auth-token-env" "codex --help"
Require-Option $appHelp "--listen" "codex app-server --help"
Require-Option $appHelp "--ws-auth" "codex app-server --help"
Require-Option $appHelp "--ws-token-file" "codex app-server --help"
$savedErrorActionPreference = $ErrorActionPreference
try {
    $ErrorActionPreference = "Continue"
    $ignoreCheck = (& git -C $repoRoot check-ignore target/codex-broker-gate 2>$null | Out-String).Trim()
    $ignoreExitCode = $LASTEXITCODE
} finally {
    $ErrorActionPreference = $savedErrorActionPreference
}
if ($ignoreExitCode -ne 0 -or [string]::IsNullOrWhiteSpace($ignoreCheck)) { throw "target/codex-broker-gate must be ignored by Git before running." }
if (-not (Test-PortAvailable $AppServerPort)) { throw "App Server port $AppServerPort is already in use." }
if (-not (Test-PortAvailable $BrokerPort)) { throw "Broker port $BrokerPort is already in use." }

try {
    $appToken = New-PrivateTokenFile "app-server-token.txt"
    $brokerToken = New-PrivateTokenFile "broker-cli-token.txt"
    $appTokenHash = (Get-FileHash -LiteralPath $appToken.Path -Algorithm SHA256).Hash
    $brokerTokenHash = (Get-FileHash -LiteralPath $brokerToken.Path -Algorithm SHA256).Hash
    if ($appTokenHash -eq $brokerTokenHash) { throw "Generated tokens are not distinct." }

    $appArgs = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ('"' + (Join-Path $scriptDirectory "start-app-server.ps1") + '"'), "-CodexPath", ('"' + $CodexPath + '"'), "-ListenUri", $appServerUri, "-TokenFile", ('"' + $appToken.Path + '"'))
    $appServerProcess = Start-Process -FilePath "powershell.exe" -ArgumentList $appArgs -RedirectStandardOutput $appServerStdout -RedirectStandardError $appServerStderr -WindowStyle Hidden -PassThru
    Wait-HttpReady $appServerProcess $AppServerPort $appServerStderr

    $brokerArgs = @((Join-Path $scriptDirectory "broker.mjs"), "--listen", $brokerUri, "--upstream", $appServerUri, "--client-token-file", $brokerToken.Path, "--app-server-token-file", $appToken.Path, "--metadata-log", $metadataLog)
    $brokerProcess = Start-Process -FilePath $nodePath -ArgumentList $brokerArgs -RedirectStandardOutput $brokerStdout -RedirectStandardError $brokerStderr -WindowStyle Hidden -PassThru
    Wait-BrokerReady $brokerProcess

    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $scriptDirectory "launch-cli.ps1") -CodexPath $CodexPath -BrokerUri $brokerUri -TokenFile $brokerToken.Path
    $cliExitCode = $LASTEXITCODE
    Start-Sleep -Milliseconds 500

    $records = @(Get-Content -LiteralPath $metadataLog -Encoding UTF8 | ForEach-Object { $_ | ConvertFrom-Json })
    $messages = @($records | Where-Object { $_.event -eq "message" })
    $approval = $messages | Where-Object { $_.direction -eq "app_server_to_cli" -and $_.kind -eq "request" -and $_.method -in @("item/commandExecution/requestApproval", "item/fileChange/requestApproval") } | Select-Object -First 1
    $approvalResponse = if ($null -ne $approval) { $messages | Where-Object { $_.direction -eq "cli_to_app_server" -and $_.kind -eq "response" -and [string]$_.id -eq [string]$approval.id } | Select-Object -First 1 } else { $null }
    $evidence = [ordered]@{
        connectionOpened = ($null -ne ($records | Where-Object { $_.event -eq "connection_opened" } | Select-Object -First 1))
        cliRequestForwarded = ($null -ne ($messages | Where-Object { $_.direction -eq "cli_to_app_server" -and $_.kind -eq "request" } | Select-Object -First 1))
        appServerNotificationForwarded = ($null -ne ($messages | Where-Object { $_.direction -eq "app_server_to_cli" -and $_.kind -eq "notification" } | Select-Object -First 1))
        approvalMethod = if ($null -ne $approval) { $approval.method } else { $null }
        approvalId = if ($null -ne $approval) { $approval.id } else { $null }
        approvalForwardedToCli = ($null -ne $approval)
        matchingCliResponseForwarded = ($null -ne $approvalResponse)
        tokensDistinct = ($appTokenHash -ne $brokerTokenHash)
    }
    if ($cliExitCode -ne 0) { throw "Codex CLI exited with code $cliExitCode." }
    foreach ($required in @("connectionOpened", "cliRequestForwarded", "appServerNotificationForwarded", "approvalForwardedToCli", "matchingCliResponseForwarded", "tokensDistinct")) {
        if (-not $evidence[$required]) { throw "E2E criterion failed: $required" }
    }
    $status = "passed"
} catch {
    $failure = $_.Exception.Message
} finally {
    Stop-Tree $brokerProcess
    Stop-Tree $appServerProcess
    foreach ($tokenFile in @($appToken, $brokerToken)) {
        if ($null -ne $tokenFile -and (Test-Path -LiteralPath $tokenFile.Directory)) { Remove-Item -LiteralPath $tokenFile.Directory -Recurse -Force }
    }
    if ($null -eq (Get-Variable -Name evidence -ErrorAction SilentlyContinue)) { $evidence = [ordered]@{} }
    $harnessHash = Write-RunJson $evidence
}

Write-Host "Broker E2E finished."
Write-Host "Run status: $status"
Write-Host "Run metadata: $runJson"
Write-Host "Harness SHA-256: $harnessHash"
if ($status -ne "passed") { throw $failure }
exit 0
