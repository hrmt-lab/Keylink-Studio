Add-Type -AssemblyName System.Drawing

$iconsDir = Join-Path $PSScriptRoot "crates\rawhid-host-tauri\icons"
if (-not (Test-Path $iconsDir)) { New-Item -ItemType Directory -Force -Path $iconsDir | Out-Null }

# ── Rounded-rect path helper ───────────────────────────────────────────────────
function New-RRPath([float]$x,[float]$y,[float]$w,[float]$h,[float]$r) {
    $p = New-Object System.Drawing.Drawing2D.GraphicsPath
    $p.AddArc($x,         $y,         $r*2,$r*2, 180, 90)
    $p.AddArc($x+$w-$r*2, $y,         $r*2,$r*2, 270, 90)
    $p.AddArc($x+$w-$r*2, $y+$h-$r*2, $r*2,$r*2,   0, 90)
    $p.AddArc($x,         $y+$h-$r*2, $r*2,$r*2,  90, 90)
    $p.CloseFigure()
    return $p
}

# ── Main icon drawing (renders at $size x $size) ───────────────────────────────
# Matches the in-app sidebar logo: white "power button" circle with the
# accent-orange keyboard glyph (lucide "keyboard", 24x24 grid, stroke 2).
function Draw-Icon {
    param([int]$size)

    $bmp = New-Object System.Drawing.Bitmap($size, $size,
           [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g   = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode      = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
    $g.PixelOffsetMode    = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.Clear([System.Drawing.Color]::Transparent)

    [float]$sc = $size / 512.0
    $orange = [System.Drawing.Color]::FromArgb(255, 239, 91, 37)   # #EF5B25

    # ── White circle body ─────────────────────────────────────────────────────
    [float]$margin = 8 * $sc
    [float]$d = $size - $margin * 2

    # Soft drop shadow so the white disc reads on light backgrounds
    for ($i = 4; $i -ge 1; $i--) {
        $shadow = New-Object System.Drawing.SolidBrush(
            [System.Drawing.Color]::FromArgb([int](7*$i), 96, 110, 130)
        )
        [float]$grow = $i * 2 * $sc
        $g.FillEllipse($shadow, $margin - $grow, $margin - $grow + 3*$sc,
                       $d + $grow*2, $d + $grow*2)
        $shadow.Dispose()
    }

    $discBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::White)
    $g.FillEllipse($discBrush, $margin, $margin, $d, $d)
    $discBrush.Dispose()

    $ringPen = New-Object System.Drawing.Pen(
        [System.Drawing.Color]::FromArgb(255, 227, 231, 236), [float](3 * $sc))
    $g.DrawEllipse($ringPen, $margin, $margin, $d, $d)
    $ringPen.Dispose()

    # ── Keyboard glyph (lucide 24x24 units, centered at 12,12) ────────────────
    [float]$u  = 13.4 * $sc          # px per lucide unit (glyph ~52% of canvas)
    [float]$cx = $size / 2.0
    [float]$cy = $size / 2.0
    [float]$sw = 2.0 * $u            # stroke width (lucide stroke = 2)

    function MapX([float]$v) { return [float]($cx + ($v - 12.0) * $u) }
    function MapY([float]$v) { return [float]($cy + ($v - 12.0) * $u) }

    # Outer body: rect x2 y4 w20 h16 rx2
    $body = New-RRPath (MapX 2) (MapY 4) ([float](20*$u)) ([float](16*$u)) ([float](2*$u))
    $pen = New-Object System.Drawing.Pen($orange, $sw)
    $pen.LineJoin = [System.Drawing.Drawing2D.LineJoin]::Round
    $g.DrawPath($pen, $body)
    $body.Dispose()

    # Key dots (round-cap "h.01" strokes -> filled circles, radius = sw/2)
    $dotBrush = New-Object System.Drawing.SolidBrush($orange)
    $dots = @(@(6,8), @(10,8), @(14,8), @(18,8), @(8,12), @(12,12), @(16,12))
    foreach ($pt in $dots) {
        [float]$px = MapX $pt[0]
        [float]$py = MapY $pt[1]
        $g.FillEllipse($dotBrush, $px - $sw/2, $py - $sw/2, $sw, $sw)
    }
    $dotBrush.Dispose()

    # Space bar: line 7,16 -> 17,16 with round caps
    $pen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
    $pen.EndCap   = [System.Drawing.Drawing2D.LineCap]::Round
    $g.DrawLine($pen, (MapX 7), (MapY 16), (MapX 17), (MapY 16))
    $pen.Dispose()

    $g.Dispose()
    return $bmp
}

# ── Resize helper ──────────────────────────────────────────────────────────────
function Resize-Bitmap {
    param([System.Drawing.Bitmap]$src, [int]$size)
    $dst = New-Object System.Drawing.Bitmap($size, $size,
           [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g   = [System.Drawing.Graphics]::FromImage($dst)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
    $g.DrawImage($src, 0, 0, $size, $size)
    $g.Dispose()
    return $dst
}

# ── Save helpers ───────────────────────────────────────────────────────────────
function Save-Png {
    param([System.Drawing.Bitmap]$bmp, [string]$path)
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    Write-Host "  created: $path"
}

function Create-Ico {
    param([string]$outPath, [System.Drawing.Bitmap[]]$bitmaps)
    $ms = New-Object System.IO.MemoryStream
    # ICO header
    $ms.WriteByte(0); $ms.WriteByte(0)   # reserved
    $ms.WriteByte(1); $ms.WriteByte(0)   # type: ICO
    $count = [int]$bitmaps.Count
    $ms.WriteByte([byte]($count -band 0xFF))
    $ms.WriteByte([byte](($count -shr 8) -band 0xFF))
    # Encode each image to PNG bytes
    $pngList = @()
    foreach ($bmp in $bitmaps) {
        $pm = New-Object System.IO.MemoryStream
        $bmp.Save($pm, [System.Drawing.Imaging.ImageFormat]::Png)
        $pngList += ,$pm.ToArray()
    }
    # Directory entries
    $offset = 6 + 16 * $count
    for ($i = 0; $i -lt $count; $i++) {
        $s  = $bitmaps[$i].Width; $sz = [int]$s; if ($sz -ge 256) { $sz = 0 }
        $len = $pngList[$i].Length
        $ms.WriteByte([byte]$sz); $ms.WriteByte([byte]$sz)
        $ms.WriteByte(0); $ms.WriteByte(0)   # color count, reserved
        $ms.WriteByte(1); $ms.WriteByte(0)   # planes
        $ms.WriteByte(32); $ms.WriteByte(0)  # bit depth
        $ms.WriteByte([byte]($len -band 0xFF))
        $ms.WriteByte([byte](($len -shr  8) -band 0xFF))
        $ms.WriteByte([byte](($len -shr 16) -band 0xFF))
        $ms.WriteByte([byte](($len -shr 24) -band 0xFF))
        $ms.WriteByte([byte]($offset -band 0xFF))
        $ms.WriteByte([byte](($offset -shr  8) -band 0xFF))
        $ms.WriteByte([byte](($offset -shr 16) -band 0xFF))
        $ms.WriteByte([byte](($offset -shr 24) -band 0xFF))
        $offset += $len
    }
    foreach ($data in $pngList) { $ms.Write($data, 0, $data.Length) }
    [System.IO.File]::WriteAllBytes($outPath, $ms.ToArray())
    Write-Host "  created: $outPath"
}

# ── Generate ───────────────────────────────────────────────────────────────────
Write-Host "Generating icons..."
$base   = Draw-Icon 512
$bmp16  = Resize-Bitmap $base 16
$bmp32  = Resize-Bitmap $base 32
$bmp48  = Resize-Bitmap $base 48
$bmp128 = Resize-Bitmap $base 128
$bmp256 = Resize-Bitmap $base 256

Save-Png $base   (Join-Path $iconsDir "icon.png")
Save-Png $bmp32  (Join-Path $iconsDir "32x32.png")
Save-Png $bmp128 (Join-Path $iconsDir "128x128.png")
Save-Png $bmp256 (Join-Path $iconsDir "128x128@2x.png")
Create-Ico (Join-Path $iconsDir "icon.ico") @($bmp16, $bmp32, $bmp48, $bmp256)

# In-app icon asset (kept in sync with the app icon design)
Save-Png $bmp256 (Join-Path $PSScriptRoot "ui\src\assets\app-icon.png")

# ICNS placeholder (macOS)
$icnsPath = Join-Path $iconsDir "icon.icns"
if (-not (Test-Path $icnsPath)) {
    [System.IO.File]::WriteAllBytes($icnsPath, @())
}

Write-Host "Done. Icons saved to $iconsDir"
