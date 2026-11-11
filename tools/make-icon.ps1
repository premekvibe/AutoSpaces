# Regenerates assets/autospaces.ico, icon-256.png, icon-preview.png from assets/icon-source.jpg.
# Usage: pwsh tools/make-icon.ps1
Add-Type -AssemblyName System.Drawing
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$assets = Join-Path $root 'assets'
$src = [System.Drawing.Bitmap]::new((Join-Path $assets 'icon-source.jpg'))
$W = $src.Width; $H = $src.Height

# --- read source pixels (BGRA) ---
$rect = [System.Drawing.Rectangle]::new(0, 0, $W, $H)
$bd = $src.LockBits($rect, 'ReadOnly', 'Format32bppArgb')
$buf = New-Object byte[] ($bd.Stride * $H)
[System.Runtime.InteropServices.Marshal]::Copy($bd.Scan0, $buf, 0, $buf.Length)
$stride = $bd.Stride
$src.UnlockBits($bd)

# --- bounding box of DARK pixels (shadow is light grey, so threshold on darkness) ---
$thr = 110
function IsDark($x, $y) {
  $i = $y * $stride + $x * 4
  return ($buf[$i] -lt $thr -and $buf[$i+1] -lt $thr -and $buf[$i+2] -lt $thr)
}
# use a row/column needing >= 50 dark pixels to ignore specks
$cols = New-Object int[] $W; $rows = New-Object int[] $H
for ($y = 0; $y -lt $H; $y++) { for ($x = 0; $x -lt $W; $x++) {
  if (IsDark $x $y) { $cols[$x]++; $rows[$y]++ } } }
$x0 = [array]::FindIndex($cols, [Predicate[int]]{ param($v) $v -ge 50 })
$x1 = [array]::FindLastIndex($cols, [Predicate[int]]{ param($v) $v -ge 50 })
$y0 = [array]::FindIndex($rows, [Predicate[int]]{ param($v) $v -ge 50 })
$y1 = [array]::FindLastIndex($rows, [Predicate[int]]{ param($v) $v -ge 50 })
"bbox: x $x0..$x1  y $y0..$y1"

# --- corner radius from diagonal scans: first dark pixel at t => r = t / (1 - 1/sqrt2) ---
function DiagT($sx, $sy, $dx, $dy) {
  for ($t = 0; $t -lt 600; $t++) { if (IsDark ($sx + $dx*$t) ($sy + $dy*$t)) { return $t } }
  return 0
}
$ts = @((DiagT $x0 $y0 1 1), (DiagT $x1 $y0 (-1) 1), (DiagT $x0 $y1 1 (-1)), (DiagT $x1 $y1 (-1) (-1)))
$k = 1 - 1/[math]::Sqrt(2)
$r = (($ts | Measure-Object -Average).Average) / $k
"corner diag t: $($ts -join ',')  radius ~ $([math]::Round($r,1))"

# --- square crop, inset ---
$inset = 3
$size = [math]::Min($x1 - $x0 + 1, $y1 - $y0 + 1) - 2 * $inset
$cx0 = $x0 + $inset + (($x1 - $x0 + 1 - 2*$inset - $size) / 2)
$cy0 = $y0 + $inset + (($y1 - $y0 + 1 - 2*$inset - $size) / 2)
$cx0 = [int][math]::Floor($cx0); $cy0 = [int][math]::Floor($cy0)
$rad = $r - $inset
"crop $size px at ($cx0,$cy0), mask radius $([math]::Round($rad,1))"

# --- build premultiplied BGRA with analytic anti-aliased rounded-rect coverage ---
$out = New-Object byte[] ($size * $size * 4)
$half = $size / 2.0
$bx = $half - $rad
for ($y = 0; $y -lt $size; $y++) {
  $py = [math]::Abs($y + 0.5 - $half) - $bx
  for ($x = 0; $x -lt $size; $x++) {
    $px = [math]::Abs($x + 0.5 - $half) - $bx
    # signed distance to rounded rect (negative inside)
    $ox = [math]::Max($px, 0.0); $oy = [math]::Max($py, 0.0)
    $d = [math]::Sqrt($ox*$ox + $oy*$oy) + [math]::Min([math]::Max($px, $py), 0.0) - $rad
    $a = [math]::Min([math]::Max(0.5 - $d, 0.0), 1.0)
    $si = ($cy0 + $y) * $stride + ($cx0 + $x) * 4
    $oi = ($y * $size + $x) * 4
    $out[$oi]   = [byte][math]::Round($buf[$si]   * $a)
    $out[$oi+1] = [byte][math]::Round($buf[$si+1] * $a)
    $out[$oi+2] = [byte][math]::Round($buf[$si+2] * $a)
    $out[$oi+3] = [byte][math]::Round(255 * $a)
  }
}
$big = [System.Drawing.Bitmap]::new($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppPArgb)
$bd = $big.LockBits([System.Drawing.Rectangle]::new(0,0,$size,$size), 'WriteOnly', 'Format32bppPArgb')
[System.Runtime.InteropServices.Marshal]::Copy($out, 0, $bd.Scan0, $out.Length)
$big.UnlockBits($bd)

function Resize($bmp, $n) {
  $d = [System.Drawing.Bitmap]::new($n, $n, [System.Drawing.Imaging.PixelFormat]::Format32bppPArgb)
  $g = [System.Drawing.Graphics]::FromImage($d)
  $g.Clear([System.Drawing.Color]::Transparent)
  $g.CompositingMode = 'SourceCopy'
  $g.InterpolationMode = 'HighQualityBicubic'
  $g.SmoothingMode = 'HighQuality'
  $g.PixelOffsetMode = 'HighQuality'
  $ia = [System.Drawing.Imaging.ImageAttributes]::new()
  $ia.SetWrapMode('TileFlipXY')
  $g.DrawImage($bmp, [System.Drawing.Rectangle]::new(0,0,$n,$n), 0, 0, $bmp.Width, $bmp.Height, 'Pixel', $ia)
  $g.Dispose(); $ia.Dispose()
  return $d
}
function ToPng($bmp) {   # premultiplied -> straight ARGB PNG bytes
  $c = $bmp.Clone([System.Drawing.Rectangle]::new(0,0,$bmp.Width,$bmp.Height), [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $ms = [System.IO.MemoryStream]::new(); $c.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png); $c.Dispose()
  return ,$ms.ToArray()
}

# halve down to 512 first so small sizes get a well-filtered source
$mid = Resize $big 512
$sizes = 16,20,24,32,40,48,64,128,256
$pngs = @{}; $bmps = @{}
foreach ($s in $sizes) {
  $b = Resize $mid $s
  $bmps[$s] = $b; $pngs[$s] = ToPng $b
}
[System.IO.File]::WriteAllBytes((Join-Path $assets 'icon-256.png'), $pngs[256])

# --- ICO container: ICONDIR + ICONDIRENTRY[] + PNG blobs ---
$ms = [System.IO.MemoryStream]::new(); $bw = [System.IO.BinaryWriter]::new($ms)
$bw.Write([uint16]0); $bw.Write([uint16]1); $bw.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
foreach ($s in $sizes) {
  $n = if ($s -ge 256) { 0 } else { $s }
  $bw.Write([byte]$n); $bw.Write([byte]$n); $bw.Write([byte]0); $bw.Write([byte]0)
  $bw.Write([uint16]1); $bw.Write([uint16]32)
  $bw.Write([uint32]$pngs[$s].Length); $bw.Write([uint32]$offset)
  $offset += $pngs[$s].Length
}
foreach ($s in $sizes) { $bw.Write($pngs[$s]) }
$bw.Flush()
[System.IO.File]::WriteAllBytes((Join-Path $assets 'autospaces.ico'), $ms.ToArray())

# --- contact sheet: every size at 1:1 on light and dark strips ---
$pad = 16
$sheetW = $pad + ($sizes | ForEach-Object { $_ + $pad } | Measure-Object -Sum).Sum
$stripH = 256 + 2 * $pad
$sheet = [System.Drawing.Bitmap]::new($sheetW, 2 * $stripH, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$g = [System.Drawing.Graphics]::FromImage($sheet)
$g.CompositingMode = 'SourceOver'
$g.InterpolationMode = 'NearestNeighbor'; $g.PixelOffsetMode = 'Half'
$bgs = @([System.Drawing.ColorTranslator]::FromHtml('#f3f3f3'), [System.Drawing.ColorTranslator]::FromHtml('#202020'))
for ($i = 0; $i -lt 2; $i++) {
  $br = [System.Drawing.SolidBrush]::new($bgs[$i])
  $g.FillRectangle($br, 0, $i * $stripH, $sheetW, $stripH); $br.Dispose()
  $x = $pad
  foreach ($s in $sizes) {
    $g.DrawImageUnscaled($bmps[$s], $x, $i * $stripH + $pad + 256 - $s)
    $x += $s + $pad
  }
}
$g.Dispose()
$sheet.Save((Join-Path $assets 'icon-preview.png'), [System.Drawing.Imaging.ImageFormat]::Png)
"done: ico $([math]::Round((Get-Item (Join-Path $assets 'autospaces.ico')).Length/1KB,1)) KB"
