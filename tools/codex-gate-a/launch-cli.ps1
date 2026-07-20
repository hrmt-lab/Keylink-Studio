[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$CodexPath,

    [Parameter(Mandatory = $true)]
    [string]$Uri,

    [Parameter(Mandatory = $true)]
    [string]$TokenEnvVar,

    [Parameter(Mandatory = $true)]
    [string]$Scenario,

    [string]$McpServerPath,

    [string]$ResumeThreadId,

    [Parameter(Mandatory = $true)]
    [string]$ResumeStartedMarkerPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDirectory "gate-a-common.ps1")

if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($TokenEnvVar, "Process"))) {
    throw "Environment variable '$TokenEnvVar' is missing."
}

$Host.UI.RawUI.WindowTitle = "Keylink Studio Gate A - $Scenario"

Write-Host "Gate A scenario: $Scenario" -ForegroundColor Cyan
switch ($Scenario) {
    "ThreadTurn" {
        Write-Host "Complete one short turn in a new thread without using tools."
    }
    "LateObserver" {
        Write-Host "Complete one short turn, return to the launcher and press Enter, then come back here for a second turn."
    }
    "Resume" {
        Write-Host "The observer resumed an existing thread. The fixed Resume prompt is submitted automatically."
    }
    "Approval" {
        Write-Host "The fixed Approval prompt is submitted automatically. Do not answer the approval."
    }
    "UserInput" {
        Write-Host "Enter Plan mode, trigger one request_user_input choice, wait five seconds, then answer it."
    }
    "McpElicitation" {
        Write-Host "Call gate_a_request_elicitation once, wait five seconds at the elicitation, then answer it."
    }
    "PendingApprovalDisconnect" {
        Write-Host "Leave the approval unanswered and close this CLI window."
    }
    "PendingInputDisconnect" {
        Write-Host "Leave request_user_input unanswered and close this CLI window."
    }
}

$codexArgs = @(
    "--ask-for-approval", "on-request",
    "--sandbox", "read-only"
)

if ($Scenario -eq "McpElicitation") {
    if ([string]::IsNullOrWhiteSpace($McpServerPath)) {
        throw "McpServerPath is required for the MCP elicitation scenario."
    }
    $nodePath = (Get-Command node -ErrorAction Stop).Source
    if ($nodePath.Contains("'") -or $McpServerPath.Contains("'")) {
        throw "MCP executable and server paths must not contain a single quote."
    }
    # TOML literal strings keep Windows backslashes intact when passed through
    # PowerShell's native-command argument handling.
    $nodeToml = "'$nodePath'"
    $argsToml = "['$McpServerPath']"
    $codexArgs += @(
        "-c", "mcp_servers.gate_a.command=$nodeToml",
        "-c", "mcp_servers.gate_a.args=$argsToml"
    )
}

$codexArgs += @(
    "--remote", $Uri,
    "--remote-auth-token-env", $TokenEnvVar
)

if (-not [string]::IsNullOrWhiteSpace($ResumeThreadId)) {
    $codexArgs = @("resume") + $codexArgs + @($ResumeThreadId)
    $initialPrompt = Get-GateAInitialPrompt -Scenario $Scenario
    if (-not [string]::IsNullOrWhiteSpace($initialPrompt)) {
        $codexArgs += $initialPrompt
    }
    $resumeStart = [ordered]@{
        threadId = $ResumeThreadId
        startedAt = [DateTimeOffset]::UtcNow.ToString("o")
    } | ConvertTo-Json -Compress
    [IO.File]::WriteAllText($ResumeStartedMarkerPath, $resumeStart, [Text.Encoding]::UTF8)
}

& $CodexPath @codexArgs
$exitCodeVariable = Get-Variable -Name LASTEXITCODE -ErrorAction SilentlyContinue
if ($null -eq $exitCodeVariable) {
    exit 0
}
exit $LASTEXITCODE
