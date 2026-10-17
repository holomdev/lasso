<#
.SYNOPSIS
    Opens the colour palette by hovering, picks a colour, and reports what changed.

.DESCRIPTION
    Three captures: the strip before hovering, the strip with the pointer on the colour
    button, and the strip after a swatch has been clicked. What has to be true is that the
    palette is absent in the first, present in the second, and that the button's own swatch
    carries the chosen colour in the third.
#>
[CmdletBinding()]
param(
    [string]$Rect = "600,250,1200,700",
    [string]$OutDir = "target\overlay-palette"
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$exe = Join-Path $root 'target\debug\lasso.exe'
if (-not [System.IO.Path]::IsPathRooted($OutDir)) { $OutDir = Join-Path $root $OutDir }
if (Test-Path $OutDir) { Remove-Item $OutDir -Recurse -Force }
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Pal {
    [StructLayout(LayoutKind.Sequential)]
    struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)]
    struct INPUT { public uint type; public MOUSEINPUT mi; }
    [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] i, int s);
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int i);
    static void Flags(uint f) {
        var input = new INPUT { type = 0 }; input.mi.dwFlags = f;
        SendInput(1, new[] { input }, Marshal.SizeOf(typeof(INPUT)));
    }
    public static void MoveTo(int x, int y) {
        int vx = GetSystemMetrics(76), vy = GetSystemMetrics(77);
        int vw = GetSystemMetrics(78), vh = GetSystemMetrics(79);
        var input = new INPUT { type = 0 };
        input.mi.dx = (int)Math.Round((x - vx) * 65535.0 / (vw - 1));
        input.mi.dy = (int)Math.Round((y - vy) * 65535.0 / (vh - 1));
        input.mi.dwFlags = 0x0001 | 0x8000 | 0x4000;
        SendInput(1, new[] { input }, Marshal.SizeOf(typeof(INPUT)));
    }
    public static void Press()   { Flags(0x0002); }
    public static void Release() { Flags(0x0004); }
    public static void Click() { Press(); System.Threading.Thread.Sleep(30); Release(); }
}
'@

$r = $Rect -split ',' | ForEach-Object { [int]$_ }
function Shot([string]$Name) {
    $bmp = New-Object System.Drawing.Bitmap(1400, 900)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen(0, 0, 0, 0, (New-Object System.Drawing.Size(1400, 900)))
    $g.Dispose()
    $bmp.Save((Join-Path $OutDir "$Name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}

# The strip's geometry, the same walk the program does.
$button = 26; $gap = 2; $padding = 4; $groupGap = 5; $standoff = 8
$groupStarts = @(5, 7, 9)
function ButtonTop([int]$index) {
    $y = $padding
    for ($i = 0; $i -lt $index; $i++) {
        $y += $gap
        if ($groupStarts -contains $i) { $y += $groupGap }
        $y += $button
    }
    if ($groupStarts -contains $index) {
        $y += $groupGap - $gap
    }
    return $y
}

$stripX = $r[0] - $standoff - ($button + $padding * 2)
$stripY = $r[1]
$colourTop = $stripY + (ButtonTop 5)
$colourX = $stripX + $padding + [int]($button / 2)
$colourY = $colourTop + [int]($button / 2)

# The palette sits beside the strip, centred on the colour button, swatches in a row.
$paletteW = 5 * $button + 4 * $gap + 2 * $padding
$paletteH = $button + 2 * $padding
$paletteX = $stripX + ($button + 2 * $padding) + $standoff
$paletteY = $colourY - [int]($paletteH / 2)
$swatchX = { param($index) $paletteX + $padding + $index * ($button + $gap) + [int]($button / 2) }
$swatchY = $paletteY + $padding + [int]($button / 2)

Write-Host "strip at ($stripX, $stripY); colour button centre ($colourX, $colourY)"
Write-Host "palette at ($paletteX, $paletteY), ${paletteW}x${paletteH}"

$proc = Start-Process -FilePath $exe -ArgumentList 'select', '--out', (Join-Path $OutDir 'result.png') -PassThru
Start-Sleep -Milliseconds 2500
if ($proc.HasExited) { throw "the overlay exited early" }

# A region, well away from where the palette will appear.
[Pal]::MoveTo($r[0], $r[1]); Start-Sleep -Milliseconds 120
[Pal]::Press()
Start-Sleep -Milliseconds 60
for ($i = 1; $i -le 6; $i++) {
    [Pal]::MoveTo($r[0] + [int](($r[2]-$r[0])*$i/6), $r[1] + [int](($r[3]-$r[1])*$i/6))
    Start-Sleep -Milliseconds 30
}
[Pal]::Release()
Start-Sleep -Milliseconds 500

# Away from the strip: no palette.
[Pal]::MoveTo($r[0] + 400, $r[1] + 300); Start-Sleep -Milliseconds 300
Shot '0-no-hover'

# On the colour button: the palette opens.
[Pal]::MoveTo($colourX, $colourY); Start-Sleep -Milliseconds 400
Shot '1-hover'

# A swatch well inside the palette: blue is the fourth (red, yellow, green, blue, white).
[Pal]::MoveTo((& $swatchX 3), $swatchY); Start-Sleep -Milliseconds 350
Shot '2-on-swatch'
[Pal]::Click(); Start-Sleep -Milliseconds 350
Shot '3-picked-blue'

# Away again, so the palette closes and the button keeps the chosen colour.
[Pal]::MoveTo($r[0] + 400, $r[1] + 300); Start-Sleep -Milliseconds 400
Shot '4-closed'

if (-not $proc.HasExited) { $proc.Kill() }
Write-Host "output in $OutDir"
