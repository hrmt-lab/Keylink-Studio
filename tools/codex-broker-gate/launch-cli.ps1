[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$CodexPath,
    [Parameter(Mandatory = $true)][string]$BrokerUri,
    [Parameter(Mandatory = $true)][string]$TokenFile
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$tokenEnvironmentVariable = "KEYLINK_CODEX_BROKER_TOKEN"
$token = [IO.File]::ReadAllText($TokenFile, [Text.Encoding]::ASCII).Trim()
if ([string]::IsNullOrWhiteSpace($token)) {
    throw "Broker token file is empty."
}

try {
    [Environment]::SetEnvironmentVariable($tokenEnvironmentVariable, $token, "Process")
    if ($env:KEYLINK_BROKER_FAKE_TEST -ne "1") {
        Write-Host "A single model-consuming Broker E2E turn will start." -ForegroundColor Cyan
        Write-Host "At the approval prompt, choose either approve or decline. After the turn completes, use /exit."
    }
    & $CodexPath `
        --ask-for-approval on-request `
        --sandbox read-only `
        --remote $BrokerUri `
        --remote-auth-token-env $tokenEnvironmentVariable `
        "Create target/codex-broker-gate/manual-approval-test.tmp containing GATE_BROKER. Request approval before writing it. Do nothing else."
    exit $LASTEXITCODE
} finally {
    [Environment]::SetEnvironmentVariable($tokenEnvironmentVariable, $null, "Process")
    $token = $null
}
