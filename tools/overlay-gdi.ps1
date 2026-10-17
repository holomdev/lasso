<#
.SYNOPSIS
    Exercises the overlay and reports the process's GDI object count as it goes.

.DESCRIPTION
    A strip whose plate and separators draw but whose icons do not is what running out of
    GDI objects looks like: `FillRect` creates nothing, while every glyph needs a pen. So
    the question "are the icons failing or is something else failing" becomes "does the
    count climb".

    Short and to the point: one selection, then a lot of pointer movement and clicks.
#>
[CmdletBinding()]
param(
    [string]$Rect = "600,250,1200,700",
    [string]$OutDir = "target\gdi-count"
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$exe = Join-Path $root 'target\debug\lasso.exe'
if (-not [System.IO.Path]::IsPathRooted($OutDir)) { $OutDir = Join-Path $root $OutDir }
if (Test-Path $OutDir) { Remove-Item $OutDir -Recurse -Force }
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Gd {
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
}
'@

# The strip's geometry, the same walk the program does.
$button = 26; $gap = 2; $padding = 4; $groupGap = 5; $standoff = 8
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
$stripX = $r[0] - $standoff - ($button + $padding * 2)
$stripY = $r[1]
$log = Join-Path $OutDir 'run.txt'

$env:LASSO_DUMP_CANVAS = Join-Path $OutDir 'canvas'
$err = Join-Path $OutDir 'stderr.txt'
$proc = Start-Process -FilePath $exe `
    -ArgumentList 'select', '--out', (Join-Path $OutDir 'result.png') `
    -RedirectStandardError $err -PassThru -WindowStyle Hidden
Start-Sleep -Milliseconds 2500
if ($proc.HasExited) { throw "the overlay exited early" }

# A region.
[Gd]::MoveTo($r[0], $r[1]); Start-Sleep -Milliseconds 90
[Gd]::Press()
for ($i = 1; $i -le 6; $i++) {
    [Gd]::MoveTo($r[0] + [int](($r[2]-$r[0])*$i/6), $r[1] + [int](($r[3]-$r[1])*$i/6))
    Start-Sleep -Milliseconds 20
}
[Gd]::Release(); Start-Sleep -Milliseconds 250

# Plenty of movement and use, which is what the report says brings it on.
for ($round = 1; $round -le 30; $round++) {
    $bx = [int]($stripX + $padding + $button / 2)
    $by = [int]($stripY + (ButtonTop (1 + ($round % 4))) + $button / 2)
    [Gd]::MoveTo($bx, $by); Start-Sleep -Milliseconds 30
    [Gd]::Press(); Start-Sleep -Milliseconds 25; [Gd]::Release()
    # Draw something with whatever is armed.
    [Gd]::MoveTo($r[0] + 60, $r[1] + 60); Start-Sleep -Milliseconds 25
    [Gd]::Press()
    for ($k = 1; $k -le 4; $k++) {
        [Gd]::MoveTo($r[0] + 60 + 20*$k + $round, $r[1] + 60 + 15*$k)
        Start-Sleep -Milliseconds 15
    }
    [Gd]::Release(); Start-Sleep -Milliseconds 20
    # And away from the strip, so both kinds of repaint happen.
    [Gd]::MoveTo($r[0] + 300, $r[1] + 200); Start-Sleep -Milliseconds 20
}

Start-Sleep -Milliseconds 500
$proc.Kill()

Write-Host "=== GDI object count per dump ==="
Get-Content $err | Select-String -Pattern 'GDI objects|CreatePen failed' | ForEach-Object { "  " + $_.Line.Trim() }
