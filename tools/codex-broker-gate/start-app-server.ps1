[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$CodexPath,
    [Parameter(Mandatory = $true)][string]$ListenUri,
    [Parameter(Mandatory = $true)][string]$TokenFile
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

& $CodexPath app-server `
    --listen $ListenUri `
    --ws-auth capability-token `
    --ws-token-file $TokenFile

exit $LASTEXITCODE
