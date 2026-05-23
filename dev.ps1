# RawHID Host - Dev launcher
# Usage: .\dev.ps1

$root = $PSScriptRoot
$uiDir = Join-Path $root 'ui'
$tauriDir = Join-Path $root 'crates\rawhid-host-tauri'

foreach ($cmd in @('npm', 'cargo')) {
    if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) {
        Write-Error "$cmd not found. Please install Node.js / Rust."
        exit 1
    }
}

if (-not (Test-Path (Join-Path $uiDir 'node_modules'))) {
    Write-Host '[1/2] Installing frontend dependencies...' -ForegroundColor Cyan
    Push-Location $uiDir
    try {
        npm install
    } finally {
        Pop-Location
    }
} else {
    Write-Host '[1/2] Frontend dependencies are ready.' -ForegroundColor Gray
}

Write-Host '[2/2] Starting cargo tauri dev...' -ForegroundColor Cyan
Write-Host ''

Push-Location $tauriDir
try {
    cargo tauri dev
} finally {
    Pop-Location
}
