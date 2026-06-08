# RawHID Host - Release build script
# Usage: .\build-release.ps1

$root = $PSScriptRoot
$uiDir = Join-Path $root "ui"
$tauriDir = Join-Path $root "crates\rawhid-host-tauri"
$tauriConfig = Join-Path $tauriDir "tauri.conf.json"
$version = (Get-Content $tauriConfig -Raw | ConvertFrom-Json).version
$bundleDir = Join-Path $root "target\release\bundle"
$releaseDir = Join-Path $root ("release\RawHID-Host-v{0}" -f $version)

Write-Host "Building RawHID Host v$version (release)..."

# Install npm deps if needed
if (-not (Test-Path (Join-Path $uiDir "node_modules"))) {
    Write-Host "Installing frontend dependencies..."
    Set-Location $uiDir
    npm install
}

Write-Host "Running: cargo tauri build"
Set-Location $tauriDir
& cargo tauri build
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

Set-Location $root
if (Test-Path $releaseDir) {
    Remove-Item $releaseDir -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $releaseDir | Out-Null
Get-ChildItem -Path $bundleDir -Recurse -File |
    Where-Object { $_.Name -like "*$version*" } |
    ForEach-Object {
        $relativePath = $_.FullName.Substring($bundleDir.Length).TrimStart("\")
        $destination = Join-Path $releaseDir $relativePath
        $destinationDir = Split-Path $destination -Parent
        New-Item -ItemType Directory -Force -Path $destinationDir | Out-Null
        Copy-Item -Path $_.FullName -Destination $destination -Force
    }

Write-Host ""
Write-Host "Build complete!"
Write-Host "Bundle: $bundleDir"
Write-Host "Versioned release: $releaseDir"
