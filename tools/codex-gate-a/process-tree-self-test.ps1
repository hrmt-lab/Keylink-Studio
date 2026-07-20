[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ReadyPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$startInfo = [Diagnostics.ProcessStartInfo]::new()
$startInfo.FileName = "powershell.exe"
$startInfo.Arguments = '-NoProfile -Command "Start-Sleep -Seconds 30"'
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
$child = [Diagnostics.Process]::Start($startInfo)

$ready = [ordered]@{
    parentPid = $PID
    childPid = $child.Id
} | ConvertTo-Json -Compress
[IO.File]::WriteAllText($ReadyPath, $ready, [Text.Encoding]::UTF8)

[void]$child.WaitForExit()
