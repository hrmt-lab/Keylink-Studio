# Keylink Studio - Dev launcher
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

Write-Host '[2/3] Building Claude hook Helper...' -ForegroundColor Cyan
Push-Location $root
try {
    cargo build -p rawhid-host-core --bin keylink-claude-hook
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
    Pop-Location
}

Write-Host '[3/3] Starting cargo tauri dev...' -ForegroundColor Cyan
Write-Host ''

Push-Location $tauriDir
try {
    cargo tauri dev
} finally {
    Pop-Location
}
