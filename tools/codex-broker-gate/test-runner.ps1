[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
function Get-FreePort {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    try { return ([Net.IPEndPoint]$listener.LocalEndpoint).Port } finally { $listener.Stop() }
}
$appPort = Get-FreePort
do { $brokerPort = Get-FreePort } while ($brokerPort -eq $appPort)
try {
    $env:KEYLINK_BROKER_FAKE_TEST = "1"
    & (Join-Path $scriptDirectory "run-broker-gate.ps1") -AppServerPort $appPort -BrokerPort $brokerPort -CodexPath (Join-Path $scriptDirectory "fake-codex.ps1")
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
    Remove-Item Env:KEYLINK_BROKER_FAKE_TEST -ErrorAction SilentlyContinue
}
Write-Host "Broker PowerShell runner self-test passed."
