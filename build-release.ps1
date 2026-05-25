# RawHID Host - リリースビルドスクリプト
# Usage: .\build-release.ps1

$root = $PSScriptRoot
$uiDir = Join-Path $root "ui"
$tauriDir = Join-Path $root "crates\rawhid-host-tauri"

Write-Host "Building RawHID Host (release)..."

# Install npm deps if needed
if (-not (Test-Path (Join-Path $uiDir "node_modules"))) {
    Write-Host "Installing frontend dependencies..."
    Set-Location $uiDir
    npm install
}

Write-Host "Running: cargo tauri build"
Set-Location $tauriDir
& cargo tauri build

Write-Host ""
Write-Host "Build complete!"
Write-Host "Installer: $root\target\release\bundle\"
