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

    # ── Background: deep navy gradient ────────────────────────────────────────
    $bgp = New-RRPath 0 0 ([float]$size) ([float]$size) ([float](80*$sc))
    $bgb = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
        (New-Object System.Drawing.PointF([float]0,     [float]0)),
        (New-Object System.Drawing.PointF([float]$size, [float]$size)),
        [System.Drawing.Color]::FromArgb(255, 11, 17, 32),
        [System.Drawing.Color]::FromArgb(255,  5,  9, 19)
    )
    $g.FillPath($bgb, $bgp)
    $bgb.Dispose(); $bgp.Dispose()

    # Subtle corner accent (top-left lightness)
    $accentBrush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
        (New-Object System.Drawing.PointF([float]0, [float]0)),
        (New-Object System.Drawing.PointF([float]($size*0.6), [float]($size*0.6))),
        [System.Drawing.Color]::FromArgb(18, 100, 140, 220),
        [System.Drawing.Color]::FromArgb(0,  100, 140, 220)
    )
    $accentPath = New-RRPath 0 0 ([float]$size) ([float]$size) ([float](80*$sc))
    $g.FillPath($accentBrush, $accentPath)
    $accentBrush.Dispose(); $accentPath.Dispose()

    # ── Keyboard body ─────────────────────────────────────────────────────────
    [float]$kx = 50  * $sc
    [float]$ky = 144 * $sc
    [float]$kw = 412 * $sc
    [float]$kh = 224 * $sc
    [float]$kr = 22  * $sc

    # Glow layers behind active row (soft radial bloom)
    [float]$glowCx = $kx + $kw * 0.5
    [float]$glowCy = $ky + $kh * 0.48
    for ($i = 8; $i -ge 1; $i--) {
        [float]$gw = $kw * 0.75 + $i * 16 * $sc
        [float]$gh = 44  * $sc  + $i * 11 * $sc
        [float]$gx = $glowCx - $gw * 0.5
        [float]$gy = $glowCy - $gh * 0.5
        $gb = New-Object System.Drawing.SolidBrush(
            [System.Drawing.Color]::FromArgb([int](4.5*$i), 72, 114, 200)
        )
        $g.FillEllipse($gb, $gx, $gy, $gw, $gh)
        $gb.Dispose()
    }

    # Keyboard body fill
    $kbp = New-RRPath $kx $ky $kw $kh $kr
    $kbb = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
        (New-Object System.Drawing.PointF([float]$kx, [float]$ky)),
        (New-Object System.Drawing.PointF([float]$kx, [float]($ky+$kh))),
        [System.Drawing.Color]::FromArgb(255, 31, 46, 76),
        [System.Drawing.Color]::FromArgb(255, 17, 27, 50)
    )
    $g.FillPath($kbb, $kbp)
    $kbb.Dispose()

    # Keyboard body border
    $kbpen = New-Object System.Drawing.Pen(
        [System.Drawing.Color]::FromArgb(68, 110, 150, 215),
        [float](1.5 * $sc)
    )
    $g.DrawPath($kbpen, $kbp)
    $kbpen.Dispose(); $kbp.Dispose()

    # Inner top-edge highlight (subtle bevel on keyboard body)
    $bevelPath = New-RRPath ([float]($kx+1*$sc)) ([float]($ky+1*$sc)) `
                            ([float]($kw-2*$sc))  ([float]($kr*1.5))  ([float]($kr*0.9))
    $bevelBrush = New-Object System.Drawing.SolidBrush(
        [System.Drawing.Color]::FromArgb(22, 180, 210, 255)
    )
    $g.FillPath($bevelBrush, $bevelPath)
    $bevelBrush.Dispose(); $bevelPath.Dispose()

    # ── Key rows ──────────────────────────────────────────────────────────────
    # Rows: [baseY@512, keyCount, keyW@512, keyH@512, isActiveLayer]
    $rowDefs = @(
        @{ Y=163; N=13; W=24; H=19; Active=$false },   # top / fn row
        @{ Y=193; N=12; W=27; H=21; Active=$false },   # number row
        @{ Y=225; N=11; W=29; H=22; Active=$true  },   # HOME ROW (active layer)
        @{ Y=258; N= 9; W=33; H=23; Active=$false },   # lower row
        @{ Y=292; N= 6; W=30; H=21; Active=$false }    # bottom row (abbreviated)
    )
    [float]$gap  = 5.2 * $sc
    [float]$keyR = 3.5 * $sc

    foreach ($row in $rowDefs) {
        [float]$ry   = $row.Y * $sc
        [int]   $cnt  = $row.N
        [float]$rw   = $row.W * $sc
        [float]$rh   = $row.H * $sc
        [bool]  $act  = $row.Active

        [float]$tw = $cnt * $rw + ($cnt - 1) * $gap
        [float]$sx = $kx + ($kw - $tw) * 0.5

        for ($k = 0; $k -lt $cnt; $k++) {
            [float]$rx = $sx + $k * ($rw + $gap)
            $kp = New-RRPath $rx $ry $rw $rh $keyR

            if ($act) {
                # Active-layer key: primary blue, glowing
                $fb = New-Object System.Drawing.SolidBrush(
                    [System.Drawing.Color]::FromArgb(218, 82, 124, 184)
                )
                $g.FillPath($fb, $kp); $fb.Dispose()

                # Bevel highlight on top half of key
                $hlp = New-RRPath $rx $ry $rw ([float]($rh * 0.48)) $keyR
                $hlb = New-Object System.Drawing.SolidBrush(
                    [System.Drawing.Color]::FromArgb(52, 200, 230, 255)
                )
                $g.FillPath($hlb, $hlp); $hlb.Dispose(); $hlp.Dispose()

                # Key border
                $kpen = New-Object System.Drawing.Pen(
                    [System.Drawing.Color]::FromArgb(180, 148, 192, 245),
                    [float](1.0 * $sc)
                )
                $g.DrawPath($kpen, $kp); $kpen.Dispose()
            } else {
                # Inactive key: subtle translucent white
                $fb = New-Object System.Drawing.SolidBrush(
                    [System.Drawing.Color]::FromArgb(34, 215, 230, 255)
                )
                $g.FillPath($fb, $kp); $fb.Dispose()

                $kpen = New-Object System.Drawing.Pen(
                    [System.Drawing.Color]::FromArgb(46, 195, 215, 255),
                    [float](0.75 * $sc)
                )
                $g.DrawPath($kpen, $kp); $kpen.Dispose()
            }
            $kp.Dispose()
        }
    }

    # ── Layer indicator dot (small circle, bottom-right of keyboard) ───────────
    [float]$dotR  = 16 * $sc
    [float]$dotX  = $kx + $kw - $dotR * 2 - 18 * $sc
    [float]$dotY  = $ky - $dotR * 1.8
    # Outer glow
    for ($i = 3; $i -ge 1; $i--) {
        [float]$gr2 = $dotR + $i * 4 * $sc
        $dotGlow = New-Object System.Drawing.SolidBrush(
            [System.Drawing.Color]::FromArgb([int](20*$i), 100, 155, 240)
        )
        $g.FillEllipse($dotGlow, $dotX - $i*4*$sc, $dotY - $i*4*$sc,
                       $dotR*2 + $i*8*$sc, $dotR*2 + $i*8*$sc)
        $dotGlow.Dispose()
    }
    # Dot fill
    $dotBrush = New-Object System.Drawing.SolidBrush(
        [System.Drawing.Color]::FromArgb(255, 91, 140, 210)
    )
    $g.FillEllipse($dotBrush, $dotX, $dotY, $dotR*2, $dotR*2)
    $dotBrush.Dispose()
    # Dot highlight
    $dotHL = New-Object System.Drawing.SolidBrush(
        [System.Drawing.Color]::FromArgb(90, 200, 230, 255)
    )
    $g.FillEllipse($dotHL, $dotX + $dotR*0.2, $dotY + $dotR*0.15,
                   $dotR * 0.85, $dotR * 0.6)
    $dotHL.Dispose()

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

# ICNS placeholder (macOS)
$icnsPath = Join-Path $iconsDir "icon.icns"
if (-not (Test-Path $icnsPath)) {
    [System.IO.File]::WriteAllBytes($icnsPath, @())
}

Write-Host "Done. Icons saved to $iconsDir"
