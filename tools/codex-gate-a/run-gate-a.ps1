[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("Resume", "Approval")]
    [string]$Scenario,

    [string]$CodexPath = "codex",

    [ValidateRange(1024, 65535)]
    [int]$Port = 4500,

    [Parameter(Mandatory = $true)]
    [string]$ThreadId,

    [Parameter(Mandatory = $true)]
    [ValidatePattern("^[A-Za-z0-9._-]+$")]
    [string]$ValidationPairId,

    [ValidatePattern("^[A-Za-z0-9._-]+$")]
    [string]$RunId,

    [switch]$PrepareOnly,

    [ValidateRange(10, 600)]
    [int]$TurnStartTimeoutSeconds = 60,

    [ValidateRange(30, 1800)]
    [int]$ScenarioTimeoutSeconds = 180
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Some managed Windows shells contain both Path and PATH. Start-Process uses a
# case-insensitive dictionary, so normalize only this process when necessary.
$processEnvironment = [Environment]::GetEnvironmentVariables("Process")
$pathEntries = @($processEnvironment.GetEnumerator() | Where-Object { [string]$_.Key -ieq "Path" })
if ($pathEntries.Count -gt 1) {
    $combinedPath = ($pathEntries | ForEach-Object { [string]$_.Value } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }) -join ";"
    foreach ($entry in $pathEntries) {
        [Environment]::SetEnvironmentVariable([string]$entry.Key, $null, "Process")
    }
    [Environment]::SetEnvironmentVariable("Path", $combinedPath, "Process")
}

$tokenEnvironmentVariable = "KEYLINK_GATE_A_TOKEN"
$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $scriptDirectory "..\.."))
$artifactRoot = Join-Path $repositoryRoot "target\gate-a"
$schemaRoot = Join-Path $artifactRoot "schema"
$runsRoot = Join-Path $artifactRoot "runs"
$observerScript = Join-Path $scriptDirectory "observer.ps1"
$appServerScript = Join-Path $scriptDirectory "start-app-server.ps1"
$cliScript = Join-Path $scriptDirectory "launch-cli.ps1"
$listenUri = "ws://127.0.0.1:$Port"
$script:runJsonPath = $null

function Get-StringSha256([string]$Text) {
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha256.ComputeHash([Text.Encoding]::UTF8.GetBytes($Text)))).Replace("-", "")
    } finally {
        $sha256.Dispose()
    }
}

function Write-JsonAtomic([string]$Path, [object]$Value) {
    $directory = Split-Path -Parent $Path
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
    $temporaryPath = Join-Path $directory ((Split-Path -Leaf $Path) + "." + [Guid]::NewGuid().ToString("N") + ".tmp")
    $backupPath = $temporaryPath + ".bak"
    try {
        [IO.File]::WriteAllText($temporaryPath, ($Value | ConvertTo-Json -Depth 16), [Text.UTF8Encoding]::new($false))
        if (Test-Path -LiteralPath $Path) {
            [IO.File]::Replace($temporaryPath, $Path, $backupPath)
            Remove-Item -LiteralPath $backupPath -Force -ErrorAction SilentlyContinue
        } else {
            [IO.File]::Move($temporaryPath, $Path)
        }
    } finally {
        Remove-Item -LiteralPath $temporaryPath -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $backupPath -Force -ErrorAction SilentlyContinue
    }
}

function Write-RunMetadata([Collections.IDictionary]$Metadata) {
    if (-not [string]::IsNullOrWhiteSpace($script:runJsonPath)) {
        Write-JsonAtomic -Path $script:runJsonPath -Value $Metadata
    }
}

function Get-HarnessManifest {
    $paths = @(Get-ChildItem -LiteralPath $scriptDirectory -File | Where-Object { $_.Extension -in @(".ps1", ".mjs") } | Select-Object -ExpandProperty FullName | Sort-Object)
    $files = @($paths | ForEach-Object {
        [ordered]@{
            path = $_.Substring($repositoryRoot.Length).TrimStart([char[]]@(92, 47)).Replace("\", "/")
            sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $_).Hash
        }
    })
    $canonical = ($files | ForEach-Object { "$($_.path)=$($_.sha256)" }) -join "`n"
    return [ordered]@{ fingerprintSha256 = Get-StringSha256 $canonical; files = $files }
}

function Assert-ArtifactRootIgnored {
    $savedPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $tracked = @(& git -C $repositoryRoot ls-files -- "target/gate-a" 2>$null)
        $trackedExit = $LASTEXITCODE
        & git -C $repositoryRoot check-ignore -q -- "target/gate-a/" 2>$null
        $ignoredExit = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $savedPreference
    }
    if ($trackedExit -ne 0 -or $tracked.Count -gt 0) {
        throw "target/gate-a contains tracked files or could not be checked."
    }
    if ($ignoredExit -ne 0) {
        throw "target/gate-a is not excluded by Git."
    }
}

function Invoke-CodexHelp([string[]]$Arguments) {
    $output = & $CodexPath @Arguments 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) {
        throw "codex $($Arguments -join ' ') failed.`n$output"
    }
    return $output
}

function Require-Option([string]$Text, [string]$Option, [string]$Source) {
    $pattern = "(?m)^\s*(?:-[A-Za-z0-9],\s*)?" + [regex]::Escape($Option) + "(?:\s|=|$)"
    if (-not [regex]::IsMatch($Text, $pattern)) {
        throw "$Source does not contain required option: $Option"
    }
}

function Require-Literal([string]$Text, [string]$Literal, [string]$Source) {
    if (-not $Text.Contains($Literal)) {
        throw "$Source does not contain required value: $Literal"
    }
}

function Get-JsonMarker([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    return Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json
}

function ConvertTo-Timestamp([object]$Value) {
    if ($null -eq $Value) { return $null }
    try { return [DateTimeOffset]::Parse([string]$Value, [Globalization.CultureInfo]::InvariantCulture) } catch { return $null }
}

function Get-ObserverRecords([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return @() }
    $text = $null
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        $stream = $null
        $reader = $null
        try {
            $stream = [IO.FileStream]::new($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
            $reader = [IO.StreamReader]::new($stream, [Text.Encoding]::UTF8, $true)
            $text = $reader.ReadToEnd()
            break
        } catch [IO.IOException] {
            Start-Sleep -Milliseconds 25
        } finally {
            if ($null -ne $reader) { $reader.Dispose() } elseif ($null -ne $stream) { $stream.Dispose() }
        }
    }
    if ($null -eq $text) { return @() }
    return @($text -split '\r?\n' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | ForEach-Object {
        try { $_ | ConvertFrom-Json } catch { $null }
    } | Where-Object { $null -ne $_ })
}

function Get-CorrelatedEvidence([object[]]$Records, [string]$ExpectedThreadId, [DateTimeOffset]$CliStartedAt) {
    $turnStarts = @($Records | Where-Object {
        $_.direction -eq "inbound" -and $_.method -eq "turn/started" -and
        $_.threadId -eq $ExpectedThreadId -and
        (ConvertTo-Timestamp $_.timestamp) -ge $CliStartedAt -and
        -not [string]::IsNullOrWhiteSpace([string]$_.turnId)
    } | Sort-Object timestamp)

    $completedTurnId = $null
    $approval = $null
    foreach ($start in $turnStarts) {
        $startAt = ConvertTo-Timestamp $start.timestamp
        $completion = $Records | Where-Object {
            $_.direction -eq "inbound" -and $_.method -eq "turn/completed" -and
            $_.threadId -eq $ExpectedThreadId -and $_.turnId -eq $start.turnId -and
            (ConvertTo-Timestamp $_.timestamp) -ge $startAt
        } | Select-Object -First 1
        if ($null -ne $completion -and $null -eq $completedTurnId) { $completedTurnId = [string]$start.turnId }

        $candidate = $Records | Where-Object {
            $_.direction -eq "inbound" -and $_.kind -eq "server_request" -and
            $_.method -eq "item/commandExecution/requestApproval" -and $_.requiresResponse -eq $true -and
            $_.threadId -eq $ExpectedThreadId -and $_.turnId -eq $start.turnId -and
            (ConvertTo-Timestamp $_.timestamp) -ge $startAt
        } | Select-Object -First 1
        if ($null -ne $candidate -and $null -eq $approval) { $approval = $candidate }
    }

    $observerResponded = $false
    if ($null -ne $approval) {
        $observerResponded = @($Records | Where-Object {
            $_.direction -eq "outbound" -and $_.kind -eq "response" -and $_.id -eq $approval.id
        }).Count -gt 0
    }
    return [ordered]@{
        turnStarted = ($turnStarts.Count -gt 0)
        turnId = if ($turnStarts.Count -gt 0) { [string]$turnStarts[0].turnId } else { $null }
        sameTurnCompleted = (-not [string]::IsNullOrWhiteSpace($completedTurnId))
        approvalCorrelated = ($null -ne $approval)
        approvalMethod = if ($null -ne $approval) { [string]$approval.method } else { $null }
        approvalId = if ($null -ne $approval) { $approval.id } else { $null }
        approvalTurnId = if ($null -ne $approval) { [string]$approval.turnId } else { $null }
        observerResponded = $observerResponded
    }
}

function Get-ValidationBaseline([string]$PairId) {
    $candidates = Get-ChildItem -LiteralPath $runsRoot -Filter "run.json" -File -Recurse -ErrorAction SilentlyContinue | Sort-Object LastWriteTimeUtc -Descending
    foreach ($candidate in $candidates) {
        try {
            $data = Get-Content -LiteralPath $candidate.FullName -Raw -Encoding UTF8 | ConvertFrom-Json
            if ($data.scenario -eq "Resume" -and $data.validationPair.id -eq $PairId -and $data.status -eq "passed") {
                return [pscustomobject]@{ Path = $candidate.FullName; Data = $data }
            }
        } catch { continue }
    }
    return $null
}

function New-PrivateTokenFile {
    $directory = Join-Path ([IO.Path]::GetTempPath()) ("keylink-studio-gate-a-" + [Guid]::NewGuid().ToString("N"))
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
    $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($identity, [Security.AccessControl.FileSystemRights]::FullControl, [Security.AccessControl.AccessControlType]::Allow))
    Set-Acl -LiteralPath $path -AclObject $acl
    return [pscustomobject]@{ Directory = $directory; Path = $path; Token = $token }
}

function Test-PortAvailable {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, $Port)
    try { $listener.Start(); return $true } catch [Net.Sockets.SocketException] { return $false } finally { $listener.Stop() }
}

function Wait-PortAvailable([int]$TimeoutSeconds = 10) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        if (Test-PortAvailable) { return }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Port $Port is still in use after $TimeoutSeconds seconds."
}

function Get-ProcessExitCode([Diagnostics.Process]$Process) {
    if ($null -eq $Process) { return $null }
    $Process.Refresh()
    if (-not $Process.HasExited) { return $null }
    try { $Process.WaitForExit(); $Process.Refresh(); return [int]$Process.ExitCode } catch { return $null }
}

function Stop-ProcessTree([Diagnostics.Process]$Process) {
    if ($null -eq $Process) { return $null }
    $Process.Refresh()
    $rootId = $Process.Id
    if (-not $Process.HasExited) {
        $savedPreference = $ErrorActionPreference
        try {
            $ErrorActionPreference = "Continue"
            & taskkill.exe /PID $rootId /T /F 1>$null 2>$null
        } catch { } finally { $ErrorActionPreference = $savedPreference }
        Stop-Process -Id $rootId -Force -ErrorAction SilentlyContinue
    }
    if (-not $Process.HasExited) { [void]$Process.WaitForExit(5000) }
    return [ordered]@{ rootPid = $rootId; exited = $Process.HasExited }
}

function Wait-AppServerReady([Diagnostics.Process]$Process, [string]$StderrPath) {
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $Process.Refresh()
        if ($Process.HasExited) {
            $detail = if (Test-Path -LiteralPath $StderrPath) { (Get-Content -LiteralPath $StderrPath -Encoding UTF8 | Select-Object -Last 20) -join "`n" } else { "No stderr captured." }
            throw "App Server exited before becoming ready.`n$detail"
        }
        try {
            $response = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/readyz" -UseBasicParsing -TimeoutSec 1
            if ($response.StatusCode -eq 200) { return }
        } catch { Start-Sleep -Milliseconds 100 }
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "App Server did not become ready within 10 seconds."
}

function Wait-ObserverReady([Diagnostics.Process]$Process, [string]$ReadyPath, [string]$StderrPath) {
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        if (Test-Path -LiteralPath $ReadyPath) { return }
        $Process.Refresh()
        if ($Process.HasExited) {
            $detail = if (Test-Path -LiteralPath $StderrPath) { (Get-Content -LiteralPath $StderrPath -Encoding UTF8 | Select-Object -Last 20) -join "`n" } else { "No stderr captured." }
            throw "Observer exited before initialization completed.`n$detail"
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Observer initialization did not complete within 10 seconds."
}

Assert-ArtifactRootIgnored
New-Item -ItemType Directory -Path $schemaRoot -Force | Out-Null
New-Item -ItemType Directory -Path $runsRoot -Force | Out-Null

$version = (Invoke-CodexHelp @("--version")).Trim()
if ($version -ne "codex-cli 0.144.6") {
    throw "This reproduction harness requires codex-cli 0.144.6; detected '$version'."
}
$cliHelp = Invoke-CodexHelp @("--help")
$resumeHelp = Invoke-CodexHelp @("resume", "--help")
$appServerHelp = Invoke-CodexHelp @("app-server", "--help")
$schemaHelp = Invoke-CodexHelp @("app-server", "generate-json-schema", "--help")
Require-Option $cliHelp "--remote" "codex --help"
Require-Option $cliHelp "--remote-auth-token-env" "codex --help"
Require-Option $resumeHelp "--remote" "codex resume --help"
Require-Option $resumeHelp "--remote-auth-token-env" "codex resume --help"
Require-Literal $resumeHelp "[SESSION_ID]" "codex resume --help"
Require-Literal $resumeHelp "[PROMPT]" "codex resume --help"
Require-Option $appServerHelp "--listen" "codex app-server --help"
Require-Option $appServerHelp "--ws-auth" "codex app-server --help"
Require-Option $appServerHelp "--ws-token-file" "codex app-server --help"
Require-Option $schemaHelp "--out" "generate-json-schema --help"
Require-Option $schemaHelp "--experimental" "generate-json-schema --help"

$schemaDirectory = Join-Path $schemaRoot "codex-cli-0.144.6"
New-Item -ItemType Directory -Path $schemaDirectory -Force | Out-Null
& $CodexPath app-server generate-json-schema --experimental --out $schemaDirectory
if ($LASTEXITCODE -ne 0) { throw "Schema generation failed." }
$schemaPath = Join-Path $schemaDirectory "codex_app_server_protocol.schemas.json"
$schemaText = Get-Content -LiteralPath $schemaPath -Raw -Encoding UTF8
$schema = $schemaText | ConvertFrom-Json
if ($schema.definitions.InitializeParams.required -notcontains "clientInfo" -or
    $schema.definitions.ClientInfo.required -notcontains "name" -or
    $schema.definitions.ClientInfo.required -notcontains "version" -or
    $null -eq $schema.definitions.ClientInfo.properties.title -or
    $null -eq $schema.definitions.InitializeCapabilities.properties.experimentalApi -or
    $schema.definitions.v2.ThreadResumeParams.required -notcontains "threadId") {
    throw "Generated Schema does not contain the formal initialize/resume fields used by Observer."
}
foreach ($method in @("turn/started", "turn/completed", "item/commandExecution/requestApproval")) {
    Require-Literal $schemaText ('"' + $method + '"') $schemaPath
}
$schemaHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $schemaPath).Hash
$harness = Get-HarnessManifest
if ($PrepareOnly) {
    Write-Host "Gate A preparation checks passed." -ForegroundColor Green
    Write-Host "Codex version: $version"
    Write-Host "Schema SHA-256: $schemaHash"
    Write-Host "Harness SHA-256: $($harness.fingerprintSha256)"
    Write-Host "target/gate-a Git exclusion: verified"
    return
}

$baseline = $null
if ($Scenario -eq "Approval") {
    $baseline = Get-ValidationBaseline $ValidationPairId
    if ($null -eq $baseline) { throw "Run a passed Resume with validation pair '$ValidationPairId' first." }
    if ([string]$baseline.Data.thread.id -ne $ThreadId) { throw "Approval Thread ID does not match Resume." }
    if ([string]$baseline.Data.harness.fingerprintSha256 -ne $harness.fingerprintSha256) { throw "Harness changed after Resume." }
}

if ([string]::IsNullOrWhiteSpace($RunId)) { $RunId = [DateTime]::UtcNow.ToString("yyyyMMdd-HHmmss") + "-" + [Guid]::NewGuid().ToString("N").Substring(0, 8) }
$runDirectory = Join-Path $runsRoot "$RunId-$Scenario"
if (Test-Path -LiteralPath $runDirectory) { throw "Run directory already exists: $runDirectory" }
New-Item -ItemType Directory -Path $runDirectory -Force | Out-Null
$observerLog = Join-Path $runDirectory "observer.jsonl"
$observerReady = Join-Path $runDirectory "observer.ready"
$observerResumeMarker = Join-Path $runDirectory "observer-resume.json"
$serverRequestMarker = Join-Path $runDirectory "server-request.marker"
$safetyIntentMarker = Join-Path $runDirectory "observer-exit-intent.json"
$cliStartedMarker = Join-Path $runDirectory "cli-resume-started.json"
$observerStdout = Join-Path $runDirectory "observer.stdout.log"
$observerStderr = Join-Path $runDirectory "observer.stderr.log"
$appServerStdout = Join-Path $runDirectory "app-server.stdout.log"
$appServerStderr = Join-Path $runDirectory "app-server.stderr.log"
$script:runJsonPath = Join-Path $runDirectory "run.json"

$metadata = [ordered]@{
    status = "prepared"
    decision = $null
    runId = $RunId
    scenario = $Scenario
    codexVersion = $version
    schemaSha256 = $schemaHash
    targetGateAIgnoredByGit = $true
    harness = $harness
    thread = [ordered]@{ id = $ThreadId; source = "explicit_parameter" }
    validationPair = [ordered]@{ id = $ValidationPairId; role = $Scenario.ToLowerInvariant(); baselineRunJson = if ($null -ne $baseline) { $baseline.Path } else { $null } }
    observerResume = [ordered]@{ succeeded = $false; actualThreadId = $null }
    cliResume = [ordered]@{ started = $false; startedAt = $null; turnId = $null }
    cliExitCode = $null
    observerExitCode = $null
    observerSafetyExitIntent = $null
    responseRequiredRequest = [ordered]@{ method = $null; requestId = $null; threadId = $null; turnId = $null; observerResponded = $null }
    assertions = [ordered]@{}
    diagnosticAssertions = [ordered]@{}
    failedAssertions = @()
    failure = $null
    cleanup = [ordered]@{ processTrees = @(); portReleased = $false }
    startedAt = [DateTimeOffset]::UtcNow.ToString("o")
}
Write-RunMetadata $metadata

$tokenFile = $null
$appServerProcess = $null
$observerProcess = $null
$cliProcess = $null
$capturedError = $null
$previousToken = [Environment]::GetEnvironmentVariable($tokenEnvironmentVariable, "Process")
try {
    Wait-PortAvailable
    $tokenFile = New-PrivateTokenFile
    [Environment]::SetEnvironmentVariable($tokenEnvironmentVariable, $tokenFile.Token, "Process")

    $appArgs = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ('"' + $appServerScript + '"'), "-CodexPath", ('"' + $CodexPath + '"'), "-ListenUri", $listenUri, "-TokenFile", ('"' + $tokenFile.Path + '"'))
    $appServerProcess = Start-Process -FilePath "powershell.exe" -ArgumentList $appArgs -RedirectStandardOutput $appServerStdout -RedirectStandardError $appServerStderr -WindowStyle Hidden -PassThru
    Wait-AppServerReady $appServerProcess $appServerStderr

    $observerArgs = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ('"' + $observerScript + '"'), "-Uri", $listenUri, "-TokenEnvVar", $tokenEnvironmentVariable, "-LogPath", ('"' + $observerLog + '"'), "-ResumeThreadId", $ThreadId, "-ReadyPath", ('"' + $observerReady + '"'), "-ResumeMarkerPath", ('"' + $observerResumeMarker + '"'), "-ServerRequestMarkerPath", ('"' + $serverRequestMarker + '"'), "-SafetyExitIntentPath", ('"' + $safetyIntentMarker + '"'))
    $observerProcess = Start-Process -FilePath "powershell.exe" -ArgumentList $observerArgs -RedirectStandardOutput $observerStdout -RedirectStandardError $observerStderr -WindowStyle Hidden -PassThru
    Wait-ObserverReady $observerProcess $observerReady $observerStderr

    $resumeMarker = Get-JsonMarker $observerResumeMarker
    if ($null -eq $resumeMarker -or -not [bool]$resumeMarker.succeeded) { throw "Observer thread/resume did not succeed." }
    $metadata.observerResume.succeeded = $true
    $metadata.observerResume.actualThreadId = [string]$resumeMarker.actualThreadId
    Write-RunMetadata $metadata

    $cliArgs = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ('"' + $cliScript + '"'), "-CodexPath", ('"' + $CodexPath + '"'), "-Uri", $listenUri, "-TokenEnvVar", $tokenEnvironmentVariable, "-Scenario", $Scenario, "-ThreadId", $ThreadId, "-ResumeStartedMarkerPath", ('"' + $cliStartedMarker + '"'))
    $cliProcess = Start-Process -FilePath "powershell.exe" -ArgumentList $cliArgs -PassThru

    while (-not $cliProcess.HasExited) {
        if (Test-Path -LiteralPath $serverRequestMarker) {
            Write-Warning "Observer received a response-required server request. No response was sent."
            if (-not $observerProcess.HasExited) { [void]$observerProcess.WaitForExit(15000) }
            Stop-ProcessTree $cliProcess | Out-Null
            break
        }
        if ($observerProcess.HasExited) { throw "Observer exited before a server request was recorded." }
        $cliMarker = Get-JsonMarker $cliStartedMarker
        if ($null -ne $cliMarker) {
            $cliStartedAt = ConvertTo-Timestamp $cliMarker.startedAt
            $records = Get-ObserverRecords $observerLog
            $turnObserved = @($records | Where-Object {
                $_.direction -eq "inbound" -and $_.method -eq "turn/started" -and $_.threadId -eq $ThreadId -and (ConvertTo-Timestamp $_.timestamp) -ge $cliStartedAt
            }).Count -gt 0
            $elapsed = [DateTimeOffset]::UtcNow - $cliStartedAt
            if (-not $turnObserved -and $elapsed.TotalSeconds -ge $TurnStartTimeoutSeconds) { throw "turn/started was not observed within $TurnStartTimeoutSeconds seconds." }
            if ($elapsed.TotalSeconds -ge $ScenarioTimeoutSeconds) { throw "Scenario did not complete within $ScenarioTimeoutSeconds seconds." }
        }
        Start-Sleep -Milliseconds 250
    }

    if (-not $cliProcess.HasExited) { [void]$cliProcess.WaitForExit(5000) }
    $metadata.cliExitCode = Get-ProcessExitCode $cliProcess
    $metadata.observerExitCode = Get-ProcessExitCode $observerProcess
    $metadata.observerSafetyExitIntent = Get-JsonMarker $safetyIntentMarker
    $cliMarker = Get-JsonMarker $cliStartedMarker
    if ($null -eq $cliMarker) { throw "CLI resume start marker is missing." }
    $metadata.cliResume.started = $true
    $metadata.cliResume.startedAt = [string]$cliMarker.startedAt
    $evidence = Get-CorrelatedEvidence (Get-ObserverRecords $observerLog) $ThreadId (ConvertTo-Timestamp $cliMarker.startedAt)
    $metadata.cliResume.turnId = $evidence.turnId

    if ($evidence.approvalCorrelated) {
        $metadata.responseRequiredRequest.method = $evidence.approvalMethod
        $metadata.responseRequiredRequest.requestId = $evidence.approvalId
        $metadata.responseRequiredRequest.threadId = $ThreadId
        $metadata.responseRequiredRequest.turnId = $evidence.approvalTurnId
        $metadata.responseRequiredRequest.observerResponded = $evidence.observerResponded
    }

    if ($Scenario -eq "Resume") {
        $metadata.assertions = [ordered]@{
            observerResumeSucceeded = $metadata.observerResume.succeeded
            observerResumeThreadMatches = ($metadata.observerResume.actualThreadId -eq $ThreadId)
            cliResumeStarted = $metadata.cliResume.started
            turnStarted = $evidence.turnStarted
            sameTurnCompleted = $evidence.sameTurnCompleted
            cliExitedZero = ($metadata.cliExitCode -eq 0)
        }
        $metadata.status = if (@($metadata.assertions.Values | Where-Object { $_ -ne $true }).Count -eq 0) { "passed" } else { "failed" }
        $metadata.decision = if ($metadata.status -eq "passed") { "resume_observation_reproduced" } else { "inconclusive" }
    } else {
        $markerMethod = if (Test-Path -LiteralPath $serverRequestMarker) { (Get-Content -LiteralPath $serverRequestMarker -Raw -Encoding ASCII).Trim() } else { $null }
        $metadata.assertions = [ordered]@{
            observerResumeSucceeded = $metadata.observerResume.succeeded
            observerResumeThreadMatches = ($metadata.observerResume.actualThreadId -eq $ThreadId)
            turnStarted = $evidence.turnStarted
            responseRequiredApprovalDelivered = $evidence.approvalCorrelated
            markerMethodMatches = ($markerMethod -eq $evidence.approvalMethod)
            observerSentNoResponse = ($evidence.approvalCorrelated -and -not $evidence.observerResponded)
            harnessMatchesResume = ($baseline.Data.harness.fingerprintSha256 -eq $harness.fingerprintSha256)
            threadMatchesResume = ($baseline.Data.thread.id -eq $ThreadId)
        }
        $metadata.diagnosticAssertions = [ordered]@{
            observerExitIntent42 = ($null -ne $metadata.observerSafetyExitIntent -and $metadata.observerSafetyExitIntent.exitCode -eq 42)
            observerOsExitCode42 = ($metadata.observerExitCode -eq 42)
        }
        $metadata.status = if (@($metadata.assertions.Values | Where-Object { $_ -ne $true }).Count -eq 0) { "broker_required" } else { "failed" }
        $metadata.decision = if ($metadata.status -eq "broker_required") { "observer_ineligible_response_required_request_delivered" } else { "inconclusive" }
    }
    $metadata.failedAssertions = @($metadata.assertions.GetEnumerator() | Where-Object { $_.Value -ne $true } | ForEach-Object { $_.Key })
    $metadata["completedAt"] = [DateTimeOffset]::UtcNow.ToString("o")
} catch {
    $capturedError = $_
    $metadata.status = "failed"
    $metadata.decision = "inconclusive"
    $metadata.failure = [ordered]@{ type = $_.Exception.GetType().FullName; message = $_.Exception.Message; occurredAt = [DateTimeOffset]::UtcNow.ToString("o") }
    $metadata["completedAt"] = [DateTimeOffset]::UtcNow.ToString("o")
} finally {
    foreach ($process in @($cliProcess, $observerProcess, $appServerProcess)) {
        try {
            $report = Stop-ProcessTree $process
            if ($null -ne $report) { $metadata.cleanup.processTrees += $report }
        } catch { }
    }
    try { Wait-PortAvailable; $metadata.cleanup.portReleased = $true } catch { $metadata.cleanup.portReleased = $false }
    $metadata.cliExitCode = if ($null -ne (Get-ProcessExitCode $cliProcess)) { Get-ProcessExitCode $cliProcess } else { $metadata.cliExitCode }
    $metadata.observerExitCode = if ($null -ne (Get-ProcessExitCode $observerProcess)) { Get-ProcessExitCode $observerProcess } else { $metadata.observerExitCode }
    $metadata.observerSafetyExitIntent = Get-JsonMarker $safetyIntentMarker
    [Environment]::SetEnvironmentVariable($tokenEnvironmentVariable, $previousToken, "Process")
    if ($null -ne $tokenFile) {
        $temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
        $tokenDirectory = [IO.Path]::GetFullPath($tokenFile.Directory)
        if ($tokenDirectory.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase) -and (Split-Path -Leaf $tokenDirectory).StartsWith("keylink-studio-gate-a-")) {
            Remove-Item -LiteralPath $tokenDirectory -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
    Write-RunMetadata $metadata
}

Write-Host "Scenario finished: $Scenario" -ForegroundColor Green
Write-Host "Run status: $($metadata.status)"
Write-Host "Decision: $($metadata.decision)"
Write-Host "Run metadata: $script:runJsonPath"
Write-Host "Sanitized observer log: $observerLog"
Write-Host "Schema SHA-256: $schemaHash"
Write-Host "Harness SHA-256: $($harness.fingerprintSha256)"

if ($null -ne $capturedError) { throw $capturedError }
if ($metadata.status -eq "failed") { throw "Gate A assertions failed: $($metadata.failedAssertions -join ', ')" }
