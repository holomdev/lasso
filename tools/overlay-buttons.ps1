<#
.SYNOPSIS
    Presses every button of the strip in turn and checks what survives each press.

.DESCRIPTION
    The report is that the icons go and the drawing disappears after choosing something
    from the strip. Which button matters — a tool, a colour, undo? — so this presses each
    one in turn, drawing a stroke after each, and measures what is on screen afterwards:

      * how many of the strip's buttons still show an icon
      * whether the stroke drawn a moment ago is still there

    Everything is measured from a full-screen capture per step, because the fault is a
    repaint fault and only what ends up on screen can show it.
#>
[CmdletBinding()]
param(
    [string]$OutDir = "target\overlay-buttons",
    [string]$Rect = "500,250,1100,700"
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$exe = Join-Path $root 'target\debug\lasso.exe'
if (-not (Test-Path $exe)) { throw "no binary at $exe; build first" }
if (-not [System.IO.Path]::IsPathRooted($OutDir)) { $OutDir = Join-Path $root $OutDir }
if (Test-Path $OutDir) { Remove-Item $OutDir -Recurse -Force }
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Btn {
    [StructLayout(LayoutKind.Sequential)]
    struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)]
    struct INPUT { public uint type; public MOUSEINPUT mi; }
    [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] i, int s);
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int i);
    static void Flags(uint f) {
        var input = new INPUT { type = 0 }; input.u.mi.dwFlags = f;
        SendInput(1, new[] { input }, Marshal.SizeOf(typeof(INPUT)));
    }
    public static void MoveTo(int x, int y) {
        int vx = GetSystemMetrics(76), vy = GetSystemMetrics(77);
        int vw = GetSystemMetrics(78), vh = GetSystemMetrics(79);
        var input = new INPUT { type = 0 };
        input.u.mi.dx = (int)Math.Round((x - vx) * 65535.0 / (vw - 1));
        input.u.mi.dy = (int)Math.Round((y - vy) * 65535.0 / (vh - 1));
        input.u.mi.dwFlags = 0x0001 | 0x8000 | 0x4000;
        SendInput(1, new[] { input }, Marshal.SizeOf(typeof(INPUT)));
    }
    public static void Press()   { Flags(0x0002); }
    public static void Release() { Flags(0x0004); }
}
'@

$button = 26; $gap = 2; $padding = 4; $groupGap = 5; $standoff = 8
$stripWidth = $button + $padding * 2
$stripHeight = 12 * $button + 11 * $gap + 3 * $groupGap + $padding * 2
$groupStarts = @(5, 7, 9)

function ButtonTop([int]$index) {
    $y = $padding
    for ($i = 0; $i -le $index; $i++) {
        if ($i -gt 0) { $y += $gap }
        if ($groupStarts -contains $i) { $y += $groupGap }
        if ($i -eq $index) { break }
        $y += $button
    }
    return $y
}

$r = $Rect -split ',' | ForEach-Object { [int]$_ }
$stripX = $r[0] - $standoff - $stripWidth
$stripY = $r[1]

function Shot([string]$Name) {
    $bmp = New-Object System.Drawing.Bitmap(1400, 900)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen(0, 0, 0, 0, (New-Object System.Drawing.Size(1400, 900)))
    $g.Dispose()
    $bmp.Save((Join-Path $OutDir "$Name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}

function Measure([string]$Name) {
    $path = Join-Path $OutDir "$Name.png"
    $out = & 'C:\Users\spacejockey\.dsh\dsh-runtimes\dsh-primary-runtime\dependencies\python\python.exe' `
        (Join-Path $root 'tools\check-strip.py') $path 2>&1
    $icons = 0
    foreach ($line in $out) {
        if ($line -match '^\s+button\s+\d+.*:\s+(\d+) bright') {
            if ([int]$Matches[1] -gt 20) { $icons++ }
        }
    }
    $red = 0
    foreach ($line in $out) {
        if ($line -match 'drawn rectangle:\s+(\d+) red') { $red = [int]$Matches[1] }
    }
    return [pscustomobject]@{ Case = $Name; ButtonsWithIcons = $icons; RedPixels = $red }
}

$proc = Start-Process -FilePath $exe -ArgumentList 'select', '--out', (Join-Path $OutDir 'result.png') -PassThru
Start-Sleep -Milliseconds 2200
if ($proc.HasExited) { throw "the overlay exited early" }

[Btn]::MoveTo($r[0], $r[1]); Start-Sleep -Milliseconds 90
[Btn]::Press()
for ($i = 1; $i -le 8; $i++) {
    [Btn]::MoveTo($r[0] + [int](($r[2]-$r[0])*$i/8), $r[1] + [int](($r[3]-$r[1])*$i/8))
    Start-Sleep -Milliseconds 20
}
[Btn]::Release(); Start-Sleep -Milliseconds 300
Shot 'base'

$results = @()
$results += Measure 'base'

# Draw a stroke first, so there is something that can disappear.
[Btn]::MoveTo($r[0] + 80, $r[1] + 80); Start-Sleep -Milliseconds 80
[Btn]::Press()
for ($i = 1; $i -le 8; $i++) {
    [Btn]::MoveTo($r[0] + 80 + [int](200*$i/8), $r[1] + 80 + [int](120*$i/8))
    Start-Sleep -Milliseconds 20
}
[Btn]::Release(); Start-Sleep -Milliseconds 300

# Each tool button, then a fresh stroke, then a measurement.
foreach ($index in 0..4) {
    $bx = [int]($stripX + $padding + $button / 2)
    $by = [int]($stripY + (ButtonTop $index) + $button / 2)
    [Btn]::MoveTo($bx, $by); Start-Sleep -Milliseconds 120
    [Btn]::Press(); Start-Sleep -Milliseconds 60; [Btn]::Release()
    Start-Sleep -Milliseconds 250

    # Draw with whatever is armed, if it draws at all. `Move` will move the region instead.
    [Btn]::MoveTo($r[0] + 300, $r[1] + 300); Start-Sleep -Milliseconds 80
    [Btn]::Press()
    for ($i = 1; $i -le 8; $i++) {
        [Btn]::MoveTo($r[0] + 300 + [int](120*$i/8), $r[1] + 300 + [int](90*$i/8))
        Start-Sleep -Milliseconds 20
    }
    [Btn]::Release(); Start-Sleep -Milliseconds 250

    $name = "tool-$index"
    Shot $name
    $results += Measure $name
}

# The colour button twice, and undo.
foreach ($index in 5, 5, 7) {
    $bx = [int]($stripX + $padding + $button / 2)
    $by = [int]($stripY + (ButtonTop $index) + $button / 2)
    [Btn]::MoveTo($bx, $by); Start-Sleep -Milliseconds 120
    [Btn]::Press(); Start-Sleep -Milliseconds 60; [Btn]::Release()
    Start-Sleep -Milliseconds 250
    $name = "other-$index"
    Shot $name
    $results += Measure $name
}

$proc.Kill()
$results | Format-Table -AutoSize
