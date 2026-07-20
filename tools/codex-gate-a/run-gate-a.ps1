[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet(
        "ThreadTurn",
        "LateObserver",
        "Resume",
        "Approval",
        "UserInput",
        "McpElicitation",
        "PendingApprovalDisconnect",
        "PendingInputDisconnect"
    )]
    [string]$Scenario,

    [string]$CodexPath = "codex",

    [ValidateRange(1024, 65535)]
    [int]$Port = 4500,

    [string]$ThreadId,

    [ValidatePattern("^[A-Za-z0-9._-]+$")]
    [string]$ValidationPairId,

    [ValidatePattern("^[A-Za-z0-9._-]+$")]
    [string]$RunId,

    [switch]$PrepareOnly,

    [switch]$SelfTestFailureAfterMetadata,

    [ValidateRange(10, 600)]
    [int]$TurnStartTimeoutSeconds = 60,

    [ValidateRange(30, 1800)]
    [int]$ScenarioTimeoutSeconds = 180
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$tokenEnvironmentVariable = "KEYLINK_GATE_A_TOKEN"
$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $scriptDirectory "..\.."))
$artifactRoot = Join-Path $repositoryRoot "target\gate-a"
$schemaRoot = Join-Path $artifactRoot "schema"
$runsRoot = Join-Path $artifactRoot "runs"
$observerScript = Join-Path $scriptDirectory "observer.ps1"
$appServerScript = Join-Path $scriptDirectory "start-app-server.ps1"
$cliScript = Join-Path $scriptDirectory "launch-cli.ps1"
$mcpServerScript = Join-Path $scriptDirectory "mcp-elicitation-server.mjs"
$commonScript = Join-Path $scriptDirectory "gate-a-common.ps1"
$listenUri = "ws://127.0.0.1:$Port"
$runJsonPath = $null
. $commonScript

function Write-RunMetadata {
    param([Collections.IDictionary]$Metadata)

    if (-not [string]::IsNullOrWhiteSpace($script:runJsonPath)) {
        Write-GateAJsonAtomic -Path $script:runJsonPath -Value $Metadata
    }
}

function Get-StringSha256 {
    param([string]$Text)

    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [Text.Encoding]::UTF8.GetBytes($Text)
        return ([BitConverter]::ToString($sha256.ComputeHash($bytes))).Replace("-", "")
    } finally {
        $sha256.Dispose()
    }
}

function Get-HarnessManifest {
    $paths = @(Get-ChildItem -LiteralPath $scriptDirectory -File | Where-Object {
        $_.Extension -in @(".ps1", ".mjs")
    } | Select-Object -ExpandProperty FullName | Sort-Object)

    $files = @($paths | ForEach-Object {
        [ordered]@{
            path = $_.Substring($repositoryRoot.Length).TrimStart([char[]]@(92, 47)).Replace("\\", "/")
            sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $_).Hash
        }
    })
    $canonical = ($files | ForEach-Object { "$($_.path)=$($_.sha256)" }) -join "`n"
    return [ordered]@{
        fingerprintSha256 = Get-StringSha256 -Text $canonical
        files = $files
    }
}

function Assert-ArtifactRootIgnored {
    $tracked = @(& git -C $repositoryRoot ls-files -- "target/gate-a" 2>$null)
    if ($LASTEXITCODE -ne 0) {
        throw "git ls-files failed while checking target/gate-a."
    }
    if ($tracked.Count -gt 0) {
        throw "target/gate-a contains Git-tracked files. Gate A cannot continue."
    }

    & git -C $repositoryRoot check-ignore -q -- "target/gate-a/"
    if ($LASTEXITCODE -ne 0) {
        throw "target/gate-a is not excluded by Git. Gate A cannot continue."
    }
}

function Get-ValidationBaseline {
    param([string]$PairId)

    $candidates = Get-ChildItem -LiteralPath $runsRoot -Filter "run.json" -File -Recurse -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTimeUtc -Descending
    foreach ($candidate in $candidates) {
        try {
            $data = Get-Content -LiteralPath $candidate.FullName -Raw -Encoding UTF8 | ConvertFrom-Json
            if ($data.scenario -eq "Resume" -and
                $data.validationPair.id -eq $PairId -and
                $data.status -eq "passed") {
                return [pscustomobject]@{
                    Path = $candidate.FullName
                    Data = $data
                }
            }
        } catch {
            continue
        }
    }
    return $null
}

function Get-JsonMarker {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        return $null
    }
    return Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json
}

function Invoke-CodexHelp {
    param([string[]]$Arguments)

    $output = & $CodexPath @Arguments 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) {
        throw "codex $($Arguments -join ' ') failed.`n$output"
    }
    return $output
}

function Require-Literal {
    param(
        [string]$Text,
        [string]$Literal,
        [string]$Source
    )

    if (-not $Text.Contains($Literal)) {
        throw "Required option or field '$Literal' was not found in $Source."
    }
}

function Get-LatestObservedThreadId {
    $logs = Get-ChildItem -LiteralPath $runsRoot -Filter "observer.jsonl" -File -Recurse -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTimeUtc -Descending
    foreach ($log in $logs) {
        $records = @(Get-Content -LiteralPath $log.FullName -Encoding UTF8 -ErrorAction SilentlyContinue | Select-Object -Last 200)
        [array]::Reverse($records)
        foreach ($line in $records) {
            try {
                $record = $line | ConvertFrom-Json
                if (-not [string]::IsNullOrWhiteSpace([string]$record.threadId)) {
                    return [string]$record.threadId
                }
            } catch {
                continue
            }
        }
    }
    return $null
}

function New-GateATokenFile {
    $temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    $directory = Join-Path $temporaryRoot ("keylink-studio-gate-a-" + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $directory -Force | Out-Null

    $bytes = New-Object byte[] 32
    [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
    $token = [Convert]::ToBase64String($bytes).TrimEnd("=").Replace("+", "-").Replace("/", "_")
    $path = Join-Path $directory "capability-token.txt"
    [IO.File]::WriteAllText($path, $token, [Text.Encoding]::ASCII)

    $identity = [Security.Principal.WindowsIdentity]::GetCurrent().User
    $acl = [Security.AccessControl.FileSecurity]::new()
    $acl.SetOwner($identity)
    $acl.SetAccessRuleProtection($true, $false)
    $rule = [Security.AccessControl.FileSystemAccessRule]::new(
        $identity,
        [Security.AccessControl.FileSystemRights]::FullControl,
        [Security.AccessControl.AccessControlType]::Allow
    )
    $acl.AddAccessRule($rule)
    Set-Acl -LiteralPath $path -AclObject $acl

    return [pscustomobject]@{
        Directory = $directory
        Path = $path
        Token = $token
    }
}

function Start-ObserverProcess {
    param(
        [string]$LogPath,
        [string]$ResumeId,
        [string]$ReadyPath,
        [string]$ResumeMarkerPath,
        [string]$ServerRequestMarkerPath,
        [string]$SafetyExitIntentPath,
        [string]$StdoutPath,
        [string]$StderrPath
    )

    $arguments = @(
        "-NoProfile",
        "-ExecutionPolicy", "Bypass",
        "-File", ('"' + $observerScript + '"'),
        "-Uri", $listenUri,
        "-TokenEnvVar", $tokenEnvironmentVariable,
        "-LogPath", ('"' + $LogPath + '"'),
        "-ReadyPath", ('"' + $ReadyPath + '"'),
        "-ResumeMarkerPath", ('"' + $ResumeMarkerPath + '"'),
        "-ServerRequestMarkerPath", ('"' + $ServerRequestMarkerPath + '"'),
        "-SafetyExitIntentPath", ('"' + $SafetyExitIntentPath + '"')
    )
    if (-not [string]::IsNullOrWhiteSpace($ResumeId)) {
        $arguments += @("-ResumeThreadId", $ResumeId)
    }
    return Start-Process -FilePath "powershell.exe" `
        -ArgumentList $arguments `
        -RedirectStandardOutput $StdoutPath `
        -RedirectStandardError $StderrPath `
        -WindowStyle Hidden `
        -PassThru
}

function Wait-ObserverReady {
    param(
        [Diagnostics.Process]$Process,
        [string]$ReadyPath,
        [string]$StderrPath
    )

    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        if (Test-Path -LiteralPath $ReadyPath) {
            return
        }
        $Process.Refresh()
        if ($Process.HasExited) {
            $detail = if (Test-Path -LiteralPath $StderrPath) {
                (Get-Content -LiteralPath $StderrPath -Encoding UTF8 | Select-Object -Last 20) -join [Environment]::NewLine
            } else {
                "No Observer stderr was captured."
            }
            throw "Observer exited before initialization completed. Exit code: $($Process.ExitCode)`n$detail"
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)

    throw "Observer initialization did not complete within 10 seconds."
}

function Start-CliProcess {
    $arguments = @(
        "-NoProfile",
        "-ExecutionPolicy", "Bypass",
        "-File", ('"' + $cliScript + '"'),
        "-CodexPath", ('"' + $CodexPath + '"'),
        "-Uri", $listenUri,
        "-TokenEnvVar", $tokenEnvironmentVariable,
        "-Scenario", $Scenario,
        "-McpServerPath", ('"' + $mcpServerScript + '"'),
        "-ResumeStartedMarkerPath", ('"' + $cliResumeStarted + '"')
    )
    if ($requiresExistingThread -and -not [string]::IsNullOrWhiteSpace($ThreadId)) {
        $arguments += @("-ResumeThreadId", $ThreadId)
    }

    return Start-Process -FilePath "powershell.exe" -ArgumentList $arguments -PassThru
}

function Test-PortAvailable {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, $Port)
    try {
        $listener.Start()
        return $true
    } catch [Net.Sockets.SocketException] {
        return $false
    } finally {
        $listener.Stop()
    }
}

function Wait-PortAvailable {
    param([int]$TimeoutSeconds = 10)

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        if (Test-PortAvailable) {
            return
        }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)

    throw "Port $Port is still in use after $TimeoutSeconds seconds. Stop the existing listener before retrying."
}

function Stop-ProcessTree {
    param([Diagnostics.Process]$RootProcess)
    return Stop-GateAProcessTree -RootProcess $RootProcess
}

function Wait-AppServerReady {
    param([Diagnostics.Process]$Process)

    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $Process.Refresh()
        if ($Process.HasExited) {
            $detail = if (Test-Path -LiteralPath $appServerStderr) {
                (Get-Content -LiteralPath $appServerStderr -Encoding UTF8 | Select-Object -Last 20) -join [Environment]::NewLine
            } else {
                "No App Server stderr was captured."
            }
            throw "The newly started App Server exited before becoming ready.`n$detail"
        }
        try {
            $response = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/readyz" -UseBasicParsing -TimeoutSec 1
            if ($response.StatusCode -eq 200) {
                $Process.Refresh()
                if ($Process.HasExited) {
                    throw "The newly started App Server exited during the readiness check."
                }
                return
            }
        } catch {
            Start-Sleep -Milliseconds 200
        }
    } while ([DateTime]::UtcNow -lt $deadline)

    throw "App Server did not become ready within 10 seconds."
}

Assert-ArtifactRootIgnored
New-Item -ItemType Directory -Path $schemaRoot -Force | Out-Null
New-Item -ItemType Directory -Path $runsRoot -Force | Out-Null

$versionOutput = Invoke-CodexHelp -Arguments @("--version")
$cliHelp = Invoke-CodexHelp -Arguments @("--help")
$resumeHelp = Invoke-CodexHelp -Arguments @("resume", "--help")
$appServerHelp = Invoke-CodexHelp -Arguments @("app-server", "--help")
$schemaHelp = Invoke-CodexHelp -Arguments @("app-server", "generate-json-schema", "--help")

Require-Literal -Text $cliHelp -Literal "--remote <ADDR>" -Source "codex --help"
Require-Literal -Text $cliHelp -Literal "--remote-auth-token-env <ENV_VAR>" -Source "codex --help"
Require-Literal -Text $resumeHelp -Literal "[SESSION_ID]" -Source "codex resume --help"
Require-Literal -Text $resumeHelp -Literal "[PROMPT]" -Source "codex resume --help"
Require-Literal -Text $resumeHelp -Literal "--remote <ADDR>" -Source "codex resume --help"
Require-Literal -Text $resumeHelp -Literal "--remote-auth-token-env <ENV_VAR>" -Source "codex resume --help"
Require-Literal -Text $appServerHelp -Literal "--listen <URL>" -Source "codex app-server --help"
Require-Literal -Text $appServerHelp -Literal "--ws-auth <MODE>" -Source "codex app-server --help"
Require-Literal -Text $appServerHelp -Literal "--ws-token-file <PATH>" -Source "codex app-server --help"
Require-Literal -Text $schemaHelp -Literal "--out <DIR>" -Source "generate-json-schema --help"
Require-Literal -Text $schemaHelp -Literal "--experimental" -Source "generate-json-schema --help"

$version = $versionOutput.Trim()
$versionDirectoryName = $version -replace "[^A-Za-z0-9._-]", "-"
$schemaDirectory = Join-Path $schemaRoot $versionDirectoryName
New-Item -ItemType Directory -Path $schemaDirectory -Force | Out-Null
& $CodexPath app-server generate-json-schema --experimental --out $schemaDirectory
if ($LASTEXITCODE -ne 0) {
    throw "Schema generation failed."
}

$schemaPath = Join-Path $schemaDirectory "codex_app_server_protocol.schemas.json"
$schema = Get-Content -LiteralPath $schemaPath -Raw -Encoding UTF8 | ConvertFrom-Json
$initializeParams = $schema.definitions.InitializeParams
$initializeCapabilities = $schema.definitions.InitializeCapabilities
$clientInfo = $schema.definitions.ClientInfo
$resumeParams = $schema.definitions.v2.ThreadResumeParams

if ($initializeParams.required -notcontains "clientInfo" -or
    $null -eq $initializeParams.properties.capabilities -or
    $clientInfo.required -notcontains "name" -or
    $clientInfo.required -notcontains "version" -or
    $null -eq $clientInfo.properties.title -or
    $null -eq $initializeCapabilities.properties.experimentalApi -or
    $resumeParams.required -notcontains "threadId") {
    throw "Generated Schema does not contain the expected formal initialize/resume fields."
}

$schemaText = Get-Content -LiteralPath $schemaPath -Raw -Encoding UTF8
foreach ($method in @(
    "thread/started",
    "turn/started",
    "turn/completed",
    "item/commandExecution/requestApproval",
    "item/fileChange/requestApproval",
    "item/tool/requestUserInput",
    "mcpServer/elicitation/request",
    "serverRequest/resolved"
)) {
    Require-Literal -Text $schemaText -Literal ('"' + $method + '"') -Source $schemaPath
}

$schemaHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $schemaPath).Hash
$harnessManifest = Get-HarnessManifest
if ($PrepareOnly) {
    Write-Host "Gate A preparation checks passed." -ForegroundColor Green
    Write-Host "Codex version: $version"
    Write-Host "Schema path: $schemaPath"
    Write-Host "Schema SHA-256: $schemaHash"
    Write-Host "Harness SHA-256: $($harnessManifest.fingerprintSha256)"
    Write-Host "target/gate-a Git exclusion: verified"
    return
}

$timestamp = [DateTime]::UtcNow.ToString("yyyyMMdd-HHmmss")
if ([string]::IsNullOrWhiteSpace($RunId)) {
    $RunId = $timestamp + "-" + [Guid]::NewGuid().ToString("N").Substring(0, 8)
}
$runDirectory = Join-Path $runsRoot "$RunId-$Scenario"
if (Test-Path -LiteralPath $runDirectory) {
    throw "Run directory already exists for RunId '$RunId'."
}
New-Item -ItemType Directory -Path $runDirectory -Force | Out-Null
$observerLog = Join-Path $runDirectory "observer.jsonl"
$observerReady = Join-Path $runDirectory "observer.ready"
$observerResumeSucceeded = Join-Path $runDirectory "observer-resume-succeeded.json"
$observerServerRequest = Join-Path $runDirectory "observer-server-request.marker"
$observerSafetyExitIntent = Join-Path $runDirectory "observer-safety-exit-intent.json"
$cliResumeStarted = Join-Path $runDirectory "cli-resume-started.json"
$observerStdout = Join-Path $runDirectory "observer.stdout.log"
$observerStderr = Join-Path $runDirectory "observer.stderr.log"
$appServerStdout = Join-Path $runDirectory "app-server.stdout.log"
$appServerStderr = Join-Path $runDirectory "app-server.stderr.log"

$resumeScenarios = @(
    "Resume",
    "Approval",
    "UserInput",
    "McpElicitation",
    "PendingApprovalDisconnect",
    "PendingInputDisconnect"
)
$requiresExistingThread = $Scenario -in $resumeScenarios

if ($Scenario -in @("Resume", "Approval")) {
    if ([string]::IsNullOrWhiteSpace($ThreadId)) {
        throw "Scenario $Scenario requires an explicit -ThreadId. Automatic thread selection is not allowed for revalidation."
    }
    if ([string]::IsNullOrWhiteSpace($ValidationPairId)) {
        throw "Scenario $Scenario requires -ValidationPairId so Resume and Approval can be related."
    }
} elseif ($requiresExistingThread -and [string]::IsNullOrWhiteSpace($ThreadId)) {
    $ThreadId = Get-LatestObservedThreadId
    if ([string]::IsNullOrWhiteSpace($ThreadId)) {
        throw "No previously observed threadId was found. Run ThreadTurn first or pass -ThreadId."
    }
}

$baseline = $null
if ($Scenario -eq "Approval") {
    $baseline = Get-ValidationBaseline -PairId $ValidationPairId
    if ($null -eq $baseline) {
        throw "No passed Resume run was found for validation pair '$ValidationPairId'. Run Resume first."
    }
    if ([string]$baseline.Data.thread.id -ne $ThreadId) {
        throw "Approval Thread ID does not match the passed Resume run for validation pair '$ValidationPairId'."
    }
    if ([string]$baseline.Data.harness.fingerprintSha256 -ne $harnessManifest.fingerprintSha256) {
        throw "Gate A scripts changed after Resume. Restore the same harness before running Approval."
    }
}

$metadata = [ordered]@{
    status = "prepared"
    runId = $RunId
    scenario = $Scenario
    codexVersion = $version
    schemaSha256 = $schemaHash
    targetGateAIgnoredByGit = $true
    harness = $harnessManifest
    thread = [ordered]@{
        id = $ThreadId
        source = if ([string]::IsNullOrWhiteSpace($ThreadId)) { "not_applicable" } elseif ($Scenario -in @("Resume", "Approval")) { "explicit_parameter" } else { "automatic_previous_observation" }
    }
    validationPair = if ($Scenario -in @("Resume", "Approval")) {
        [ordered]@{
            id = $ValidationPairId
            relationship = "Resume and Approval use the same explicit threadId and identical harness fingerprint"
            role = $Scenario.ToLowerInvariant()
            baselineRunJson = if ($null -ne $baseline) { $baseline.Path } else { $null }
        }
    } else { $null }
    observerResume = [ordered]@{
        requested = $requiresExistingThread
        requestedThreadId = $ThreadId
        actualThreadId = $null
        responseId = $null
        succeeded = $false
        completedAt = $null
    }
    cliResume = [ordered]@{
        requested = $requiresExistingThread
        threadId = $ThreadId
        started = $false
        startedAt = $null
        observedViaTurnStarted = $false
        observedTurnId = $null
    }
    cliExitCode = $null
    cliTermination = $null
    observerExitCode = $null
    observerSafetyExitIntent = $null
    responseRequiredRequestMethod = $null
    responseRequiredRequest = [ordered]@{
        method = $null
        threadId = $null
        turnId = $null
        requestId = $null
        correlatedToCurrentTurn = $false
    }
    assertions = [ordered]@{}
    failedAssertions = @()
    failure = $null
    cleanup = [ordered]@{
        processCleanupAttempted = $false
        processTrees = @()
        portReleased = $false
        errors = @()
    }
    listenAddress = "127.0.0.1"
    port = $Port
    timeouts = [ordered]@{
        turnStartSeconds = $TurnStartTimeoutSeconds
        scenarioSeconds = $ScenarioTimeoutSeconds
    }
    startedAt = [DateTimeOffset]::UtcNow.ToString("o")
}
$runJsonPath = Join-Path $runDirectory "run.json"
Write-RunMetadata -Metadata $metadata

$tokenFile = $null
$appServerProcess = $null
$observerProcess = $null
$cliProcess = $null
$previousToken = [Environment]::GetEnvironmentVariable($tokenEnvironmentVariable, "Process")
$capturedError = $null

try {
    if ($SelfTestFailureAfterMetadata) {
        throw "intentional_self_test_failure_after_metadata"
    }
    Wait-PortAvailable
    $tokenFile = New-GateATokenFile
    [Environment]::SetEnvironmentVariable($tokenEnvironmentVariable, $tokenFile.Token, "Process")

    $appArguments = @(
        "-NoProfile",
        "-ExecutionPolicy", "Bypass",
        "-File", ('"' + $appServerScript + '"'),
        "-CodexPath", ('"' + $CodexPath + '"'),
        "-ListenUri", $listenUri,
        "-TokenFile", ('"' + $tokenFile.Path + '"')
    )
    $appServerProcess = Start-Process -FilePath "powershell.exe" `
        -ArgumentList $appArguments `
        -RedirectStandardOutput $appServerStdout `
        -RedirectStandardError $appServerStderr `
        -WindowStyle Hidden `
        -PassThru

    Wait-AppServerReady -Process $appServerProcess

    if ($Scenario -eq "LateObserver") {
        $cliProcess = Start-CliProcess
        Read-Host "Press Enter after starting a thread and turn in the CLI"
        $observerProcess = Start-ObserverProcess `
            -LogPath $observerLog `
            -ResumeId $null `
            -ReadyPath $observerReady `
            -ResumeMarkerPath $observerResumeSucceeded `
            -ServerRequestMarkerPath $observerServerRequest `
            -SafetyExitIntentPath $observerSafetyExitIntent `
            -StdoutPath $observerStdout `
            -StderrPath $observerStderr
        Wait-ObserverReady -Process $observerProcess -ReadyPath $observerReady -StderrPath $observerStderr
        Write-Host "Observer initialized. Return to the CLI and complete a second short turn, then use /exit." -ForegroundColor Cyan
    } else {
        $resumeId = if ($requiresExistingThread) { $ThreadId } else { $null }
        $observerProcess = Start-ObserverProcess `
            -LogPath $observerLog `
            -ResumeId $resumeId `
            -ReadyPath $observerReady `
            -ResumeMarkerPath $observerResumeSucceeded `
            -ServerRequestMarkerPath $observerServerRequest `
            -SafetyExitIntentPath $observerSafetyExitIntent `
            -StdoutPath $observerStdout `
            -StderrPath $observerStderr
        Wait-ObserverReady -Process $observerProcess -ReadyPath $observerReady -StderrPath $observerStderr
        $cliProcess = Start-CliProcess
    }

    $resumeMarker = Get-JsonMarker -Path $observerResumeSucceeded
    if ($null -ne $resumeMarker) {
        $metadata.observerResume.requestedThreadId = [string]$resumeMarker.requestedThreadId
        $metadata.observerResume.actualThreadId = [string]$resumeMarker.actualThreadId
        $metadata.observerResume.responseId = $resumeMarker.responseId
        $metadata.observerResume.succeeded = [bool]$resumeMarker.succeeded
        $metadata.observerResume.completedAt = [string]$resumeMarker.completedAt
    }
    Write-RunMetadata -Metadata $metadata

    $observerSafetyStop = $false
    while (-not $cliProcess.HasExited) {
        if (Test-Path -LiteralPath $observerServerRequest) {
            Write-Warning "Observer received a response-required server request. The scenario is stopping without a response."
            if (-not $observerProcess.HasExited) {
                [void]$observerProcess.WaitForExit(15000)
            }
            $observerProcess.Refresh()
            $observedObserverExitCode = Get-GateAProcessExitCode -Process $observerProcess
            $safetyOutcome = Test-GateAObserverSafetyOutcome `
                -MarkerExists $true `
                -HasExited $observerProcess.HasExited `
                -ExitCode $observedObserverExitCode
            if (-not $safetyOutcome.accepted) {
                throw "Observer safety stop failed: $($safetyOutcome.reason)."
            }
            $observerSafetyStop = $true
            Stop-ProcessTree -RootProcess $cliProcess
            break
        }
        if ($observerProcess.HasExited) {
            $observerProcess.Refresh()
            $observedObserverExitCode = Get-GateAProcessExitCode -Process $observerProcess
            $safetyOutcome = Test-GateAObserverSafetyOutcome `
                -MarkerExists (Test-Path -LiteralPath $observerServerRequest) `
                -HasExited $true `
                -ExitCode $observedObserverExitCode
            if ($safetyOutcome.accepted) {
                $observerSafetyStop = $true
                Stop-ProcessTree -RootProcess $cliProcess
                break
            }
            throw "Observer exited unexpectedly. Exit code: $($observerProcess.ExitCode)"
        }
        if ($Scenario -in @("Resume", "Approval") -and (Test-Path -LiteralPath $cliResumeStarted)) {
            $liveCliMarker = Get-JsonMarker -Path $cliResumeStarted
            $liveCliStartedAt = ConvertTo-GateATimestamp -Value $liveCliMarker.startedAt
            if ($null -ne $liveCliStartedAt) {
                $liveRecords = Get-GateAObserverRecords -Path $observerLog
                $turnObserved = @($liveRecords | Where-Object {
                    $_.direction -eq "inbound" -and
                    $_.method -eq "turn/started" -and
                    $_.threadId -eq $ThreadId -and
                    (ConvertTo-GateATimestamp -Value $_.timestamp) -ge $liveCliStartedAt
                }).Count -gt 0
                $timeoutReason = Get-GateATimeoutReason `
                    -StartedAt $liveCliStartedAt `
                    -Now ([DateTimeOffset]::UtcNow) `
                    -TurnObserved $turnObserved `
                    -TurnStartTimeoutSeconds $TurnStartTimeoutSeconds `
                    -ScenarioTimeoutSeconds $ScenarioTimeoutSeconds
                if ($timeoutReason -eq "turn_start_timeout") {
                    throw "CLI turn/started was not observed within $TurnStartTimeoutSeconds seconds."
                }
                if ($timeoutReason -eq "scenario_timeout") {
                    throw "Scenario did not complete within $ScenarioTimeoutSeconds seconds."
                }
            }
        }
        Start-Sleep -Milliseconds 250
    }

    if (-not $cliProcess.HasExited) {
        [void]$cliProcess.WaitForExit(5000)
    }
    $cliProcess.Refresh()
    if ($cliProcess.HasExited) {
        $metadata.cliExitCode = $cliProcess.ExitCode
    }
    $metadata.cliTermination = if ($observerSafetyStop) {
        "stopped_by_harness_after_observer_safety_stop"
    } else {
        "exited_by_user_cli_session"
    }

    $cliResumeMarker = Get-JsonMarker -Path $cliResumeStarted
    if ($null -ne $cliResumeMarker) {
        $metadata.cliResume.threadId = [string]$cliResumeMarker.threadId
        $metadata.cliResume.started = $true
        $metadata.cliResume.startedAt = [string]$cliResumeMarker.startedAt
    }
    if ($observerProcess.HasExited) {
        $metadata.observerExitCode = Get-GateAProcessExitCode -Process $observerProcess
    }
    $metadata.observerSafetyExitIntent = Get-JsonMarker -Path $observerSafetyExitIntent
    if (Test-Path -LiteralPath $observerServerRequest) {
        $metadata.responseRequiredRequestMethod = (Get-Content -LiteralPath $observerServerRequest -Raw -Encoding ASCII).Trim()
    }

    $records = Get-GateAObserverRecords -Path $observerLog
    $evidence = $null
    $cliStartAt = if ($null -ne $cliResumeMarker) {
        ConvertTo-GateATimestamp -Value $cliResumeMarker.startedAt
    } else { $null }
    if ($null -ne $cliStartAt) {
        $evidence = Get-GateACorrelatedEvidence -Records $records -ThreadId $ThreadId -CliResumeStartedAt $cliStartAt
        $metadata.cliResume.observedViaTurnStarted = $evidence.cliResumeObserved
        $metadata.cliResume.observedTurnId = $evidence.firstObservedTurnId
        if ($evidence.approvalRequestCorrelated) {
            $metadata.responseRequiredRequest.method = $evidence.approvalMethod
            $metadata.responseRequiredRequest.threadId = $ThreadId
            $metadata.responseRequiredRequest.turnId = $evidence.approvalTurnId
            $metadata.responseRequiredRequest.requestId = $evidence.approvalRequestId
            $metadata.responseRequiredRequest.correlatedToCurrentTurn = $true
        }
    }

    if ($Scenario -eq "Resume") {
        $metadata.assertions = [ordered]@{
            observerResumeSucceeded = $metadata.observerResume.succeeded
            observerResumeThreadIdMatches = ($metadata.observerResume.actualThreadId -eq $metadata.thread.id)
            cliResumeInvocationStarted = $metadata.cliResume.started
            cliResumeThreadIdMatches = ($metadata.cliResume.threadId -eq $metadata.thread.id)
            cliResumeObservedViaTurnStarted = ($null -ne $evidence -and $evidence.cliResumeObserved)
            sameTurnStartedAndCompleted = ($null -ne $evidence -and $evidence.resumeTurnCompleted)
            cliExitedZero = ($metadata.cliExitCode -eq 0)
        }
        $metadata.status = if (@($metadata.assertions.Values | Where-Object { $_ -ne $true }).Count -eq 0) { "passed" } else { "failed" }
    } elseif ($Scenario -eq "Approval") {
        $metadata.assertions = [ordered]@{
            observerResumeSucceeded = $metadata.observerResume.succeeded
            observerResumeThreadIdMatches = ($metadata.observerResume.actualThreadId -eq $metadata.thread.id)
            cliResumeInvocationStarted = $metadata.cliResume.started
            cliResumeThreadIdMatches = ($metadata.cliResume.threadId -eq $metadata.thread.id)
            cliResumeObservedViaTurnStarted = ($null -ne $evidence -and $evidence.cliResumeObserved)
            approvalRequestCorrelatedToCurrentTurn = ($null -ne $evidence -and $evidence.approvalRequestCorrelated)
            markerMethodMatchesCorrelatedRequest = ($null -ne $evidence -and $evidence.approvalRequestCorrelated -and $metadata.responseRequiredRequestMethod -eq $metadata.responseRequiredRequest.method)
            observerSentNoResponse = ($null -ne $evidence -and $evidence.approvalRequestCorrelated -and -not $evidence.observerSentApprovalResponse)
            observerSafetyExit42 = ($metadata.observerExitCode -eq 42)
            harnessMatchesResume = ($baseline.Data.harness.fingerprintSha256 -eq $metadata.harness.fingerprintSha256)
            threadIdMatchesResume = ($baseline.Data.thread.id -eq $metadata.thread.id)
        }
        $metadata.status = if (@($metadata.assertions.Values | Where-Object { $_ -ne $true }).Count -eq 0) { "passed" } else { "failed" }
    } else {
        $metadata.status = if ($observerSafetyStop) { "observer_received_server_request" } else { "manual_review_required" }
    }
    $metadata.failedAssertions = @($metadata.assertions.GetEnumerator() | Where-Object { $_.Value -ne $true } | ForEach-Object { $_.Key })
    $metadata["completedAt"] = [DateTimeOffset]::UtcNow.ToString("o")
    Write-RunMetadata -Metadata $metadata

    Write-Host "Scenario finished: $Scenario" -ForegroundColor Green
    Write-Host "Run status: $($metadata.status)"
    Write-Host "Run metadata: $runJsonPath"
    Write-Host "Sanitized observer log: $observerLog"
    Write-Host "Schema SHA-256: $schemaHash"
    Write-Host "Harness SHA-256: $($harnessManifest.fingerprintSha256)"
} catch {
    $capturedError = $_
    $metadata.status = "failed"
    $metadata.failure = [ordered]@{
        type = $_.Exception.GetType().FullName
        message = $_.Exception.Message
        occurredAt = [DateTimeOffset]::UtcNow.ToString("o")
    }
    if (Test-Path -LiteralPath $observerServerRequest) {
        $metadata.responseRequiredRequestMethod = (Get-Content -LiteralPath $observerServerRequest -Raw -Encoding ASCII).Trim()
    }
    $cliResumeMarker = Get-JsonMarker -Path $cliResumeStarted
    if ($null -ne $cliResumeMarker) {
        $metadata.cliResume.threadId = [string]$cliResumeMarker.threadId
        $metadata.cliResume.started = $true
        $metadata.cliResume.startedAt = [string]$cliResumeMarker.startedAt
    }
    $metadata["completedAt"] = [DateTimeOffset]::UtcNow.ToString("o")
} finally {
    $metadata.cleanup.processCleanupAttempted = $true
    foreach ($process in @($cliProcess, $observerProcess, $appServerProcess)) {
        try {
            $cleanupReport = Stop-ProcessTree -RootProcess $process
            if ($null -ne $cleanupReport) {
                $metadata.cleanup.processTrees += $cleanupReport
                if (-not $cleanupReport.processEnumerationSucceeded) {
                    $metadata.cleanup.errors += "process_enumeration_failed"
                }
                if ($cleanupReport.remainingPids.Count -gt 0) {
                    $metadata.cleanup.errors += "processes_still_running"
                }
            }
        } catch {
            $metadata.cleanup.errors += "process_cleanup_failed"
        }
    }
    if ($null -ne $appServerProcess) {
        try {
            Wait-PortAvailable
            $metadata.cleanup.portReleased = $true
        } catch {
            $metadata.cleanup.errors += "port_not_released"
        }
    } else {
        $metadata.cleanup.portReleased = Test-PortAvailable
    }

    foreach ($entry in @(
        [pscustomobject]@{ Process = $cliProcess; Field = "cliExitCode" },
        [pscustomobject]@{ Process = $observerProcess; Field = "observerExitCode" }
    )) {
        if ($null -ne $entry.Process) {
            $exitCode = Get-GateAProcessExitCode -Process $entry.Process
            if ($null -ne $exitCode) {
                $metadata[$entry.Field] = $exitCode
            }
        }
    }

    [Environment]::SetEnvironmentVariable($tokenEnvironmentVariable, $previousToken, "Process")
    if ($null -ne $tokenFile) {
        $temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
        $resolvedTokenDirectory = [IO.Path]::GetFullPath($tokenFile.Directory)
        if ($resolvedTokenDirectory.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase) -and
            (Split-Path -Leaf $resolvedTokenDirectory).StartsWith("keylink-studio-gate-a-")) {
            Remove-Item -LiteralPath $resolvedTokenDirectory -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
    $metadata.observerSafetyExitIntent = Get-JsonMarker -Path $observerSafetyExitIntent
    Write-RunMetadata -Metadata $metadata
}

if ($null -ne $capturedError) {
    throw $capturedError
}
