[CmdletBinding(PositionalBinding = $false)]
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Arguments)

$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
$node = (Get-Command node -ErrorAction Stop).Source
if ($Arguments.Count -eq 1 -and $Arguments[0] -eq "--version") { Write-Output "codex-cli 0.144.6-fake"; exit 0 }
if ($Arguments.Count -eq 1 -and $Arguments[0] -eq "--help") {
    # Codex CLI 0.144.6 uses ADDR here. The harness must validate the option
    # name without coupling to clap's display-only metavar.
    Write-Output "--remote <ADDR>"
    Write-Output "--remote-auth-token-env <ENV_VAR>"
    exit 0
}
if ($Arguments[0] -eq "app-server" -and $Arguments.Count -eq 2 -and $Arguments[1] -eq "--help") {
    Write-Output "--listen <URL>"
    Write-Output "--ws-auth <MODE>"
    Write-Output "--ws-token-file <PATH>"
    exit 0
}
if ($Arguments[0] -eq "app-server") {
    & $node (Join-Path $scriptDirectory "fake-app-server.mjs") @($Arguments[1..($Arguments.Count - 1)])
    exit $LASTEXITCODE
}

$remoteIndex = [Array]::IndexOf($Arguments, "--remote")
$tokenIndex = [Array]::IndexOf($Arguments, "--remote-auth-token-env")
if ($remoteIndex -lt 0 -or $tokenIndex -lt 0) { throw "Fake Codex did not receive remote arguments." }
& $node (Join-Path $scriptDirectory "fake-cli.mjs") --remote $Arguments[$remoteIndex + 1] --token-env $Arguments[$tokenIndex + 1]
exit $LASTEXITCODE
