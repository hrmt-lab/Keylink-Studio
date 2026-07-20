[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$LogPath,

    [Parameter(Mandatory = $true)]
    [string]$ReadyPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$stream = [IO.FileStream]::new($LogPath, [IO.FileMode]::Create, [IO.FileAccess]::Write, [IO.FileShare]::None)
try {
    $line = '{"timestamp":"2026-07-20T00:00:00Z","direction":"inbound","kind":"notification","method":"turn/started","threadId":"lock-test","turnId":"lock-turn","requiresResponse":false}' + [Environment]::NewLine
    $bytes = [Text.Encoding]::UTF8.GetBytes($line)
    $stream.Write($bytes, 0, $bytes.Length)
    $stream.Flush()
    [IO.File]::WriteAllText($ReadyPath, "ready", [Text.Encoding]::ASCII)
    Start-Sleep -Milliseconds 300
} finally {
    $stream.Dispose()
}
