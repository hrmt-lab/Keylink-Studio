# Keylink Studio - Release build script
# Usage: .\build-release.ps1

$root = $PSScriptRoot
$uiDir = Join-Path $root "ui"
$tauriDir = Join-Path $root "crates\rawhid-host-tauri"
$tauriConfig = Join-Path $tauriDir "tauri.conf.json"
$version = (Get-Content $tauriConfig -Raw | ConvertFrom-Json).version
$bundleDir = Join-Path $root "target\release\bundle"
$releaseDir = Join-Path $root ("release\Keylink-Studio-v{0}" -f $version)

Write-Host "Building Keylink Studio v$version (release)..."

Write-Host "Building Claude hook Helper..."
Set-Location $root
& cargo build -p rawhid-host-core --bin keylink-claude-hook --release
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
$rustHost = (& rustc -vV | Select-String '^host: ' | ForEach-Object { $_.Line.Substring(6).Trim() })
if (-not $rustHost) {
    Write-Error "Rust host triple could not be determined."
    exit 1
}
$sidecarDir = Join-Path $tauriDir "binaries"
New-Item -ItemType Directory -Force -Path $sidecarDir | Out-Null
$sidecarPath = Join-Path $sidecarDir ("keylink-claude-hook-{0}.exe" -f $rustHost)
Copy-Item -LiteralPath (Join-Path $root "target\release\keylink-claude-hook.exe") -Destination $sidecarPath -Force

# Install npm deps if needed
if (-not (Test-Path (Join-Path $uiDir "node_modules"))) {
    Write-Host "Installing frontend dependencies..."
    Set-Location $uiDir
    npm install
}

Write-Host "Running: cargo tauri build with Claude hook Helper sidecar"
Set-Location $tauriDir
$sidecarConfig = '{"bundle":{"externalBin":["binaries/keylink-claude-hook"]}}'
& cargo tauri build --config $sidecarConfig
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
