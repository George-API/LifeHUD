# Generates assets/lifehud.ico (PNG-compressed multi-size icon).
# Usage: powershell -ExecutionPolicy Bypass -File tools/make-icon.ps1
Add-Type -AssemblyName System.Drawing

function New-RoundedPath([single]$x, [single]$y, [single]$w, [single]$h, [single]$r) {
    $p = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = $r * 2
    $p.AddArc($x, $y, $d, $d, 180, 90)
    $p.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
    $p.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
    $p.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $p.CloseFigure()
    return $p
}

function New-IconPng([int]$s) {
    $bmp = New-Object System.Drawing.Bitmap $s, $s
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.Clear([System.Drawing.Color]::Transparent)

    $m = [single]($s * 0.04)
    $bg = New-RoundedPath $m $m ([single]($s - 2 * $m)) ([single]($s - 2 * $m)) ([single]($s * 0.22))
    $grad = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
        (New-Object System.Drawing.PointF(0, 0)), (New-Object System.Drawing.PointF($s, $s)),
        [System.Drawing.Color]::FromArgb(255, 64, 132, 255), [System.Drawing.Color]::FromArgb(255, 138, 92, 246))
    $g.FillPath($grad, $bg)

    # The HUD bar
    $white = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 255, 255, 255))
    $bar = New-RoundedPath ([single]($s * 0.2)) ([single]($s * 0.22)) ([single]($s * 0.6)) ([single]($s * 0.14)) ([single]($s * 0.07))
    $g.FillPath($white, $bar)

    # Two widget tiles
    $soft = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(150, 255, 255, 255))
    $t1 = New-RoundedPath ([single]($s * 0.2)) ([single]($s * 0.44)) ([single]($s * 0.27)) ([single]($s * 0.34)) ([single]($s * 0.06))
    $t2 = New-RoundedPath ([single]($s * 0.53)) ([single]($s * 0.44)) ([single]($s * 0.27)) ([single]($s * 0.34)) ([single]($s * 0.06))
    $g.FillPath($soft, $t1)
    $g.FillPath($soft, $t2)

    $g.Dispose()
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    return ,$ms.ToArray()
}

$sizes = 16, 20, 24, 32, 48, 64, 256
$images = @()
foreach ($sz in $sizes) { $images += ,(New-IconPng $sz) }

$out = Join-Path $PSScriptRoot "..\assets\lifehud.ico"
$fs = [System.IO.File]::Create($out)
$bw = New-Object System.IO.BinaryWriter($fs)
$bw.Write([UInt16]0); $bw.Write([UInt16]1); $bw.Write([UInt16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $sz = $sizes[$i]; $data = $images[$i]
    $dim = if ($sz -ge 256) { 0 } else { $sz }
    $bw.Write([byte]$dim); $bw.Write([byte]$dim); $bw.Write([byte]0); $bw.Write([byte]0)
    $bw.Write([UInt16]1); $bw.Write([UInt16]32)
    $bw.Write([UInt32]$data.Length); $bw.Write([UInt32]$offset)
    $offset += $data.Length
}
foreach ($data in $images) { $bw.Write($data) }
$bw.Close()
Write-Output "Wrote $out"
