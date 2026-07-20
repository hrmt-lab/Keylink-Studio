Set-StrictMode -Version Latest

function Get-GateAInitialPrompt {
    param([string]$Scenario)

    $prompt = switch ($Scenario) {
        "Resume" { "Respond with exactly GATE_A_RESUME_OK. Do not use tools." }
        "Approval" { "Create target/gate-a/manual-approval-test.tmp containing GATE_A. Request approval before writing it. Do nothing else." }
        default { $null }
    }
    return $prompt
}

function Get-GateATimeoutReason {
    param(
        [DateTimeOffset]$StartedAt,
        [DateTimeOffset]$Now,
        [bool]$TurnObserved,
        [int]$TurnStartTimeoutSeconds,
        [int]$ScenarioTimeoutSeconds
    )

    $elapsed = $Now - $StartedAt
    if (-not $TurnObserved -and $elapsed.TotalSeconds -ge $TurnStartTimeoutSeconds) {
        return "turn_start_timeout"
    }
    if ($elapsed.TotalSeconds -ge $ScenarioTimeoutSeconds) {
        return "scenario_timeout"
    }
    return $null
}

function Write-GateAJsonAtomic {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [object]$Value
    )

    $directory = Split-Path -Parent $Path
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
    $temporaryPath = Join-Path $directory ((Split-Path -Leaf $Path) + "." + [Guid]::NewGuid().ToString("N") + ".tmp")
    $backupPath = $temporaryPath + ".bak"
    try {
        $json = $Value | ConvertTo-Json -Depth 16
        [IO.File]::WriteAllText($temporaryPath, $json, [Text.UTF8Encoding]::new($false))
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

function Get-GateAObserverRecords {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        return @()
    }
    $text = $null
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        $stream = $null
        $reader = $null
        try {
            $stream = [IO.FileStream]::new(
                $Path,
                [IO.FileMode]::Open,
                [IO.FileAccess]::Read,
                ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
            )
            $reader = [IO.StreamReader]::new($stream, [Text.Encoding]::UTF8, $true)
            $text = $reader.ReadToEnd()
            break
        } catch [IO.IOException] {
            Start-Sleep -Milliseconds 25
        } finally {
            if ($null -ne $reader) {
                $reader.Dispose()
            } elseif ($null -ne $stream) {
                $stream.Dispose()
            }
        }
    }
    if ($null -eq $text) {
        return @()
    }
    return @($text -split '\r?\n' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | ForEach-Object {
        try { $_ | ConvertFrom-Json } catch { $null }
    } | Where-Object { $null -ne $_ })
}

function ConvertTo-GateATimestamp {
    param([object]$Value)

    if ($null -eq $Value) {
        return $null
    }
    try {
        return [DateTimeOffset]::Parse([string]$Value, [Globalization.CultureInfo]::InvariantCulture)
    } catch {
        return $null
    }
}

function Get-GateACorrelatedEvidence {
    param(
        [Parameter(Mandatory = $true)]
        [object[]]$Records,

        [Parameter(Mandatory = $true)]
        [string]$ThreadId,

        [Parameter(Mandatory = $true)]
        [DateTimeOffset]$CliResumeStartedAt
    )

    $turnStarts = @($Records | Where-Object {
        $_.direction -eq "inbound" -and
        $_.kind -eq "notification" -and
        $_.method -eq "turn/started" -and
        $_.threadId -eq $ThreadId -and
        (ConvertTo-GateATimestamp $_.timestamp) -ge $CliResumeStartedAt -and
        -not [string]::IsNullOrWhiteSpace([string]$_.turnId)
    } | Sort-Object timestamp)

    $matchedResumeTurnId = $null
    foreach ($start in $turnStarts) {
        $startTime = ConvertTo-GateATimestamp $start.timestamp
        $completion = $Records | Where-Object {
            $_.direction -eq "inbound" -and
            $_.kind -eq "notification" -and
            $_.method -eq "turn/completed" -and
            $_.threadId -eq $ThreadId -and
            $_.turnId -eq $start.turnId -and
            (ConvertTo-GateATimestamp $_.timestamp) -ge $startTime
        } | Select-Object -First 1
        if ($null -ne $completion) {
            $matchedResumeTurnId = [string]$start.turnId
            break
        }
    }

    $approvalRecord = $null
    $approvalTurnId = $null
    foreach ($start in $turnStarts) {
        $startTime = ConvertTo-GateATimestamp $start.timestamp
        $candidate = $Records | Where-Object {
            $_.direction -eq "inbound" -and
            $_.kind -eq "server_request" -and
            $_.method -eq "item/commandExecution/requestApproval" -and
            $_.requiresResponse -eq $true -and
            $_.threadId -eq $ThreadId -and
            $_.turnId -eq $start.turnId -and
            (ConvertTo-GateATimestamp $_.timestamp) -ge $startTime
        } | Select-Object -First 1
        if ($null -ne $candidate) {
            $approvalRecord = $candidate
            $approvalTurnId = [string]$start.turnId
            break
        }
    }

    $observerSentApprovalResponse = $false
    if ($null -ne $approvalRecord) {
        $observerSentApprovalResponse = @($Records | Where-Object {
            $_.direction -eq "outbound" -and
            $_.kind -eq "response" -and
            $_.id -eq $approvalRecord.id
        }).Count -gt 0
    }

    return [ordered]@{
        cliResumeObserved = ($turnStarts.Count -gt 0)
        firstObservedTurnId = if ($turnStarts.Count -gt 0) { [string]$turnStarts[0].turnId } else { $null }
        resumeTurnCompleted = (-not [string]::IsNullOrWhiteSpace($matchedResumeTurnId))
        matchedResumeTurnId = $matchedResumeTurnId
        approvalRequestCorrelated = ($null -ne $approvalRecord)
        approvalRequestId = if ($null -ne $approvalRecord) { $approvalRecord.id } else { $null }
        approvalTurnId = $approvalTurnId
        approvalMethod = if ($null -ne $approvalRecord) { [string]$approvalRecord.method } else { $null }
        observerSentApprovalResponse = $observerSentApprovalResponse
    }
}

function Test-GateAObserverSafetyOutcome {
    param(
        [bool]$MarkerExists,
        [bool]$HasExited,
        [object]$ExitCode
    )

    $accepted = $MarkerExists -and $HasExited -and $null -ne $ExitCode -and [int]$ExitCode -eq 42
    $reason = if ($accepted) {
        "marker_and_exit_42"
    } elseif (-not $MarkerExists) {
        "marker_missing"
    } elseif (-not $HasExited) {
        "observer_still_running"
    } elseif ($null -eq $ExitCode) {
        "exit_code_missing"
    } else {
        "unexpected_exit_code"
    }
    return [ordered]@{
        accepted = $accepted
        reason = $reason
    }
}

function Get-GateAProcessExitCode {
    param([Diagnostics.Process]$Process)

    if ($null -eq $Process) {
        return $null
    }
    $Process.Refresh()
    if (-not $Process.HasExited) {
        return $null
    }
    try {
        # Complete redirected stream handling after a timed WaitForExit call.
        $Process.WaitForExit()
        $Process.Refresh()
        return [int]$Process.ExitCode
    } catch {
        return $null
    }
}

function Stop-GateAProcessTree {
    param([Diagnostics.Process]$RootProcess)

    if ($null -eq $RootProcess) {
        return $null
    }

    $rootId = $RootProcess.Id
    $enumerationSucceeded = $true
    try {
        $snapshot = @(Get-CimInstance Win32_Process -ErrorAction Stop)
    } catch {
        $snapshot = @()
        $enumerationSucceeded = $false
    }
    $queue = [Collections.Generic.Queue[int]]::new()
    $orderedIds = [Collections.Generic.List[int]]::new()
    $queue.Enqueue($rootId)
    while ($queue.Count -gt 0) {
        $parentId = $queue.Dequeue()
        $orderedIds.Add($parentId)
        foreach ($child in $snapshot | Where-Object { $_.ParentProcessId -eq $parentId }) {
            $queue.Enqueue([int]$child.ProcessId)
        }
    }

    $ids = $orderedIds.ToArray()
    [array]::Reverse($ids)
    $treeTerminationCommandSucceeded = $false
    if (-not $RootProcess.HasExited) {
        $previousErrorPreference = $ErrorActionPreference
        try {
            $ErrorActionPreference = "Continue"
            & taskkill.exe /PID $rootId /T /F 1>$null 2>$null
            $treeTerminationCommandSucceeded = ($LASTEXITCODE -eq 0)
        } finally {
            $ErrorActionPreference = $previousErrorPreference
        }
    }
    foreach ($processId in $ids) {
        Stop-Process -Id $processId -Force -ErrorAction SilentlyContinue
    }
    if (-not $RootProcess.HasExited) {
        [void]$RootProcess.WaitForExit(5000)
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        $remainingIds = @($ids | Where-Object { $null -ne (Get-Process -Id $_ -ErrorAction SilentlyContinue) })
        if ($remainingIds.Count -eq 0) {
            break
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    return [ordered]@{
        rootPid = $rootId
        processEnumerationSucceeded = $enumerationSucceeded
        treeTerminationCommandSucceeded = $treeTerminationCommandSucceeded
        targetedPids = @($ids)
        remainingPids = $remainingIds
    }
}
