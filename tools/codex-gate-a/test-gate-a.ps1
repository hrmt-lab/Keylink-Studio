[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $scriptDirectory "..\.."))
$commonScript = Join-Path $scriptDirectory "gate-a-common.ps1"
$observerScript = Join-Path $scriptDirectory "observer.ps1"
$serverScript = Join-Path $scriptDirectory "observer-self-test-server.mjs"
. $commonScript

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) {
        throw "Self-test failed: $Message"
    }
}

function Start-HiddenTestProcess {
    param([string]$FilePath, [string]$Arguments)

    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $FilePath
    $startInfo.Arguments = $Arguments
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    return [Diagnostics.Process]::Start($startInfo)
}

function New-TestRecord {
    param(
        [string]$Timestamp,
        [string]$Direction,
        [string]$Kind,
        [string]$Method,
        [object]$Id,
        [string]$ThreadId,
        [string]$TurnId,
        [bool]$RequiresResponse = $false
    )
    return [pscustomobject]@{
        timestamp = $Timestamp
        direction = $Direction
        kind = $Kind
        method = $Method
        id = $Id
        threadId = $ThreadId
        turnId = $TurnId
        requiresResponse = $RequiresResponse
    }
}

$artifactRoot = Join-Path $repositoryRoot "target\gate-a\self-test"
$testDirectory = Join-Path $artifactRoot ([Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $testDirectory -Force | Out-Null

$serverProcess = $null
$observerProcess = $null
$treeProcess = $null
$treeChildPid = $null
$failureRunDirectory = $null
$failureProcess = $null
$logLockProcess = $null
$previousToken = [Environment]::GetEnvironmentVariable("KEYLINK_GATE_A_TOKEN", "Process")
try {
    $atomicPath = Join-Path $testDirectory "atomic.json"
    Write-GateAJsonAtomic -Path $atomicPath -Value ([ordered]@{ status = "first" })
    Write-GateAJsonAtomic -Path $atomicPath -Value ([ordered]@{ status = "second" })
    $atomic = Get-Content -LiteralPath $atomicPath -Raw -Encoding UTF8 | ConvertFrom-Json
    Assert-True ($atomic.status -eq "second") "atomic run metadata replacement"
    Assert-True ((Get-GateAInitialPrompt -Scenario "Resume") -eq "Respond with exactly GATE_A_RESUME_OK. Do not use tools.") "fixed Resume initial prompt"
    Assert-True ((Get-GateAInitialPrompt -Scenario "Approval") -eq "Create target/gate-a/manual-approval-test.tmp containing GATE_A. Request approval before writing it. Do nothing else.") "fixed Approval initial prompt"
    $timeoutStart = [DateTimeOffset]::Parse("2026-07-20T00:00:00Z")
    Assert-True ((Get-GateATimeoutReason $timeoutStart $timeoutStart.AddSeconds(59) $false 60 180) -eq $null) "no premature Turn timeout"
    Assert-True ((Get-GateATimeoutReason $timeoutStart $timeoutStart.AddSeconds(60) $false 60 180) -eq "turn_start_timeout") "Turn start timeout"
    Assert-True ((Get-GateATimeoutReason $timeoutStart $timeoutStart.AddSeconds(180) $true 60 180) -eq "scenario_timeout") "scenario timeout after Turn start"

    $lockedLog = Join-Path $testDirectory "locked-observer.jsonl"
    $lockReady = Join-Path $testDirectory "log-lock.ready"
    $lockScript = Join-Path $scriptDirectory "log-lock-self-test.ps1"
    $lockArguments = '-NoProfile -ExecutionPolicy Bypass -File "{0}" -LogPath "{1}" -ReadyPath "{2}"' -f $lockScript, $lockedLog, $lockReady
    $logLockProcess = Start-HiddenTestProcess -FilePath "powershell.exe" -Arguments $lockArguments
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    while (-not (Test-Path -LiteralPath $lockReady) -and [DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 25
    }
    Assert-True (Test-Path -LiteralPath $lockReady) "concurrent log writer readiness"
    $lockedRecords = @(Get-GateAObserverRecords -Path $lockedLog)
    Assert-True ($lockedRecords.Count -eq 1 -and $lockedRecords[0].turnId -eq "lock-turn") "concurrent observer log read retry"

    $startAt = [DateTimeOffset]::Parse("2026-07-20T00:00:01Z")
    $validRecords = @(
        (New-TestRecord "2026-07-20T00:00:02Z" "inbound" "notification" "turn/started" $null "thread-a" "turn-a"),
        (New-TestRecord "2026-07-20T00:00:03Z" "inbound" "server_request" "item/commandExecution/requestApproval" 9 "thread-a" "turn-a" $true),
        (New-TestRecord "2026-07-20T00:00:04Z" "inbound" "notification" "turn/completed" $null "thread-a" "turn-a")
    )
    $validEvidence = Get-GateACorrelatedEvidence -Records $validRecords -ThreadId "thread-a" -CliResumeStartedAt $startAt
    Assert-True $validEvidence.cliResumeObserved "CLI resume evidence from turn/started"
    Assert-True $validEvidence.resumeTurnCompleted "same-turn completion correlation"
    Assert-True $validEvidence.approvalRequestCorrelated "approval Thread/Turn correlation"
    Assert-True (-not $validEvidence.observerSentApprovalResponse) "absence of Observer response"

    $responseRecords = @($validRecords) + @(
        (New-TestRecord "2026-07-20T00:00:03.500Z" "outbound" "response" $null 9 $null $null)
    )
    $responseEvidence = Get-GateACorrelatedEvidence -Records $responseRecords -ThreadId "thread-a" -CliResumeStartedAt $startAt
    Assert-True $responseEvidence.observerSentApprovalResponse "Observer response must be detected"

    $wrongTurnRecords = @(
        (New-TestRecord "2026-07-20T00:00:02Z" "inbound" "notification" "turn/started" $null "thread-a" "turn-a"),
        (New-TestRecord "2026-07-20T00:00:03Z" "inbound" "notification" "turn/completed" $null "thread-a" "turn-b"),
        (New-TestRecord "2026-07-20T00:00:04Z" "inbound" "server_request" "item/commandExecution/requestApproval" 10 "thread-b" "turn-a" $true)
    )
    $wrongEvidence = Get-GateACorrelatedEvidence -Records $wrongTurnRecords -ThreadId "thread-a" -CliResumeStartedAt $startAt
    Assert-True (-not $wrongEvidence.resumeTurnCompleted) "mismatched completion must fail"
    Assert-True (-not $wrongEvidence.approvalRequestCorrelated) "wrong-thread approval must fail"

    $staleRecords = @(
        (New-TestRecord "2026-07-20T00:00:00Z" "inbound" "notification" "turn/started" $null "thread-a" "turn-old"),
        (New-TestRecord "2026-07-20T00:00:00.500Z" "inbound" "server_request" "item/commandExecution/requestApproval" 11 "thread-a" "turn-old" $true)
    )
    $staleEvidence = Get-GateACorrelatedEvidence -Records $staleRecords -ThreadId "thread-a" -CliResumeStartedAt $startAt
    Assert-True (-not $staleEvidence.cliResumeObserved) "stale turn must not prove CLI resume"
    Assert-True (-not $staleEvidence.approvalRequestCorrelated) "stale approval must fail"

    Assert-True (Test-GateAObserverSafetyOutcome $true $true 42).accepted "marker plus exit 42 must pass"
    Assert-True (-not (Test-GateAObserverSafetyOutcome $false $true 42).accepted) "exit 42 without marker must fail"
    Assert-True (-not (Test-GateAObserverSafetyOutcome $true $true 1).accepted) "marker with exit 1 must fail"
    Assert-True (-not (Test-GateAObserverSafetyOutcome $true $false $null).accepted) "running Observer must fail"

    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $port = ([Net.IPEndPoint]$listener.LocalEndpoint).Port
    $listener.Stop()

    $serverReady = Join-Path $testDirectory "server.ready"
    $nodePath = (Get-Command node -ErrorAction Stop).Source
    $serverArguments = '"{0}" {1} "{2}"' -f $serverScript, $port, $serverReady
    $serverProcess = Start-HiddenTestProcess -FilePath $nodePath -Arguments $serverArguments

    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    while (-not (Test-Path -LiteralPath $serverReady) -and [DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 50
    }
    Assert-True (Test-Path -LiteralPath $serverReady) "local WebSocket self-test server readiness"

    [Environment]::SetEnvironmentVariable("KEYLINK_GATE_A_TOKEN", "self-test-token", "Process")
    $observerLog = Join-Path $testDirectory "observer.jsonl"
    $observerReady = Join-Path $testDirectory "observer.ready"
    $resumeMarker = Join-Path $testDirectory "resume.json"
    $requestMarker = Join-Path $testDirectory "request.marker"
    $exitIntent = Join-Path $testDirectory "exit-intent.json"
    $observerArguments = '-NoProfile -ExecutionPolicy Bypass -File "{0}" -Uri "ws://127.0.0.1:{1}" -TokenEnvVar KEYLINK_GATE_A_TOKEN -LogPath "{2}" -ReadyPath "{3}" -ResumeMarkerPath "{4}" -ServerRequestMarkerPath "{5}" -SafetyExitIntentPath "{6}"' -f `
        $observerScript, $port, $observerLog, $observerReady, $resumeMarker, $requestMarker, $exitIntent
    $observerProcess = Start-HiddenTestProcess -FilePath "powershell.exe" -Arguments $observerArguments

    [void]$observerProcess.WaitForExit(10000)
    $observerProcess.Refresh()
    $outcome = Test-GateAObserverSafetyOutcome `
        -MarkerExists (Test-Path -LiteralPath $requestMarker) `
        -HasExited $observerProcess.HasExited `
        -ExitCode (Get-GateAProcessExitCode -Process $observerProcess)
    if (-not $outcome.accepted) {
        $exitDescription = if ($observerProcess.HasExited) { [string]$observerProcess.ExitCode } else { "running" }
        $stderr = if ($observerProcess.HasExited) { $observerProcess.StandardError.ReadToEnd().Trim() } else { "" }
        throw "Self-test failed: real Observer safety stop; reason=$($outcome.reason); exit=$exitDescription; stderr=$stderr"
    }
    Assert-True ((Get-Content -LiteralPath $requestMarker -Raw -Encoding ASCII).Trim() -eq "item/commandExecution/requestApproval") "Observer marker method"
    $exitIntentData = Get-Content -LiteralPath $exitIntent -Raw -Encoding UTF8 | ConvertFrom-Json
    Assert-True ($exitIntentData.exitCode -eq 42) "Observer safety exit intent"

    $treeReady = Join-Path $testDirectory "process-tree.ready.json"
    $treeScript = Join-Path $scriptDirectory "process-tree-self-test.ps1"
    $treeArguments = '-NoProfile -ExecutionPolicy Bypass -File "{0}" -ReadyPath "{1}"' -f $treeScript, $treeReady
    $treeProcess = Start-HiddenTestProcess -FilePath "powershell.exe" -Arguments $treeArguments
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    while (-not (Test-Path -LiteralPath $treeReady) -and [DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 50
    }
    Assert-True (Test-Path -LiteralPath $treeReady) "process-tree self-test readiness"
    $treeIds = Get-Content -LiteralPath $treeReady -Raw -Encoding UTF8 | ConvertFrom-Json
    $treeChildPid = [int]$treeIds.childPid
    $cleanupReport = Stop-GateAProcessTree -RootProcess $treeProcess
    Assert-True ($cleanupReport.remainingPids.Count -eq 0) "process-tree cleanup has no remaining known PIDs"
    Assert-True ($null -eq (Get-Process -Id $treeIds.parentPid -ErrorAction SilentlyContinue)) "process-tree parent stopped"
    Assert-True ($null -eq (Get-Process -Id $treeIds.childPid -ErrorAction SilentlyContinue)) "process-tree child stopped"

    $failureRunId = "self-test-failure-" + [Guid]::NewGuid().ToString("N")
    $failureRunDirectory = Join-Path $repositoryRoot "target\gate-a\runs\$failureRunId-ThreadTurn"
    $runnerScript = Join-Path $scriptDirectory "run-gate-a.ps1"
    $failureArguments = '-NoProfile -ExecutionPolicy Bypass -File "{0}" -Scenario ThreadTurn -RunId {1} -SelfTestFailureAfterMetadata' -f `
        $runnerScript, $failureRunId
    $failureProcess = Start-HiddenTestProcess -FilePath "powershell.exe" -Arguments $failureArguments
    [void]$failureProcess.WaitForExit(30000)
    $failureProcess.Refresh()
    Assert-True $failureProcess.HasExited "intentional runner failure completed"
    Assert-True ($failureProcess.ExitCode -ne 0) "intentional runner failure exits nonzero"
    $failureRunJson = Join-Path $failureRunDirectory "run.json"
    Assert-True (Test-Path -LiteralPath $failureRunJson) "failed run writes run.json"
    $failureMetadata = Get-Content -LiteralPath $failureRunJson -Raw -Encoding UTF8 | ConvertFrom-Json
    Assert-True ($failureMetadata.status -eq "failed") "failed run status is finalized"
    Assert-True ($failureMetadata.failure.message -eq "intentional_self_test_failure_after_metadata") "failed run records error"
    Assert-True ($failureMetadata.cleanup.processCleanupAttempted -eq $true) "failed run records cleanup"
    Assert-True ($failureMetadata.cleanup.portReleased -eq $true) "failed run records released port"

    Write-Host "Gate A self-test passed." -ForegroundColor Green
    Write-Host "Observer safety exit: 42"
    Write-Host "Synthetic event correlation: passed"
    Write-Host "Atomic metadata write: passed"
    Write-Host "Process-tree cleanup: passed"
    Write-Host "Failed-run metadata finalization: passed"
    Write-Host "Fixed prompt and timeout policy: passed"
    Write-Host "Concurrent observer log read: passed"
} finally {
    if ($null -ne $treeChildPid) {
        Stop-Process -Id $treeChildPid -Force -ErrorAction SilentlyContinue
    }
    foreach ($process in @($logLockProcess, $failureProcess, $treeProcess, $observerProcess, $serverProcess)) {
        if ($null -ne $process -and -not $process.HasExited) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
            [void]$process.WaitForExit(3000)
        }
    }
    [Environment]::SetEnvironmentVariable("KEYLINK_GATE_A_TOKEN", $previousToken, "Process")
    Remove-Item -LiteralPath $testDirectory -Recurse -Force -ErrorAction SilentlyContinue
    if ($null -ne $failureRunDirectory) {
        Remove-Item -LiteralPath $failureRunDirectory -Recurse -Force -ErrorAction SilentlyContinue
    }
}
