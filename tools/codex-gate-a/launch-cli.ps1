[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$CodexPath,
    [Parameter(Mandatory = $true)][string]$Uri,
    [Parameter(Mandatory = $true)][string]$TokenEnvVar,
    [Parameter(Mandatory = $true)][ValidateSet("Resume", "Approval")][string]$Scenario,
    [Parameter(Mandatory = $true)][string]$ThreadId,
    [Parameter(Mandatory = $true)][string]$ResumeStartedMarkerPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($TokenEnvVar, "Process"))) {
    throw "Environment variable '$TokenEnvVar' is missing."
}

$prompt = if ($Scenario -eq "Resume") {
    "Respond with exactly GATE_A_RESUME_OK. Do not use tools."
} else {
    "Create target/gate-a/manual-approval-test.tmp containing GATE_A. Request approval before writing it. Do nothing else."
}

$marker = [ordered]@{
    threadId = $ThreadId
    startedAt = [DateTimeOffset]::UtcNow.ToString("o")
} | ConvertTo-Json -Compress
[IO.File]::WriteAllText($ResumeStartedMarkerPath, $marker, [Text.Encoding]::UTF8)

$Host.UI.RawUI.WindowTitle = "Keylink Studio Gate A - $Scenario"
Write-Host "Gate A scenario: $Scenario" -ForegroundColor Cyan
if ($Scenario -eq "Resume") {
    Write-Host "Wait for the fixed turn to complete, then use /exit."
} else {
    Write-Host "Do not answer the approval. The harness stops after Observer delivery is recorded."
}

& $CodexPath resume `
    --ask-for-approval on-request `
    --sandbox read-only `
    --remote $Uri `
    --remote-auth-token-env $TokenEnvVar `
    $ThreadId `
    $prompt

exit $LASTEXITCODE
