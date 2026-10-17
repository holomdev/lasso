<#
.SYNOPSIS
    Reproduces "the icons vanish and the drawing disappears after using the strip".

.DESCRIPTION
    Captures the overlay at each step of the sequence that was reported:

      1. select a region
      2. arm a tool
      3. draw a stroke
      4. move the pointer over the strip   <- the report says it starts here
      5. press a button on the strip
      6. capture again

    Each step is a separate PNG, so the moment the drawing or the icons go can be pinned
    down rather than guessed at. The magnifier is in the captures too, because it is drawn
    from the same buffer and its state says something about what is behind it.
#>
[CmdletBinding()]
param(
    [string]$OutDir = "target\overlay-repro",
    [string]$Rect = "500,300,1000,700"
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
public static class Rep {
    [StructLayout(LayoutKind.Sequential)]
    struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)]
    struct KEYBDINPUT { public ushort wVk, wScan; public uint dwFlags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Explicit)]
    struct INPUTUNION { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)]
    struct INPUT { public uint type; public INPUTUNION u; }
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
    public static void Key(ushort vk) {
        var down = new INPUT { type = 1 }; down.u.ki.wVk = vk;
        var up = down; up.u.ki.dwFlags = 0x0002;
        SendInput(1, new[] { down }, Marshal.SizeOf(typeof(INPUT)));
        System.Threading.Thread.Sleep(30);
        SendInput(1, new[] { up }, Marshal.SizeOf(typeof(INPUT)));
    }
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
    Write-Host "  shot $Name"
}

function ButtonAt([int]$index) {
    return @{
        X = [int]($stripX + $padding + $button / 2)
        Y = [int]($stripY + (ButtonTop $index) + $button / 2)
    }
}

$proc = Start-Process -FilePath $exe -ArgumentList 'select', '--out', (Join-Path $OutDir 'result.png') -PassThru
Start-Sleep -Milliseconds 2200
if ($proc.HasExited) { throw "the overlay exited early" }

Write-Host "1. selecting $Rect"
[Rep]::MoveTo($r[0], $r[1]); Start-Sleep -Milliseconds 80
[Rep]::Press()
for ($i = 1; $i -le 8; $i++) {
    [Rep]::MoveTo($r[0] + [int](($r[2]-$r[0])*$i/8), $r[1] + [int](($r[3]-$r[1])*$i/8))
    Start-Sleep -Milliseconds 20
}
[Rep]::Release(); Start-Sleep -Milliseconds 250
Shot '1-selected'

Write-Host "2. arm the rectangle tool"
[Rep]::Key(0x52)   # 'R'
Start-Sleep -Milliseconds 250
# Move away from the strip first, so the next capture shows the tool highlighted rather
# than a hover.
[Rep]::MoveTo($r[0] + 400, $r[1] + 300); Start-Sleep -Milliseconds 200
Shot '2-tool-armed'

Write-Host "3. drawing a rectangle"
[Rep]::MoveTo($r[0] + 60, $r[1] + 60); Start-Sleep -Milliseconds 80
[Rep]::Press()
for ($i = 1; $i -le 8; $i++) {
    [Rep]::MoveTo($r[0] + 60 + [int](300*$i/8), $r[1] + 60 + [int](200*$i/8))
    Start-Sleep -Milliseconds 20
}
[Rep]::Release(); Start-Sleep -Milliseconds 250
Shot '3-drawn'

Write-Host "4. pointer moving onto the strip, in small steps"
# Stepped rather than jumped: the report is about moving the pointer there, and a single
# jump produces one mouse-move where a person produces dozens. If the fault is in what a
# move invalidates, only a stepped move will show it.
$over = ButtonAt 3
$from = @{ X = $r[0] + 200; Y = $r[1] + 200 }
[Rep]::MoveTo($from.X, $from.Y); Start-Sleep -Milliseconds 200
for ($i = 1; $i -le 24; $i++) {
    [Rep]::MoveTo(
        $from.X + [int](($over.X - $from.X) * $i / 24),
        $from.Y + [int](($over.Y - $from.Y) * $i / 24))
    Start-Sleep -Milliseconds 25
}
Start-Sleep -Milliseconds 300
Shot '4-hovered-strip'

Write-Host "5. pressing the ellipse button and holding"
[Rep]::MoveTo($over.X, $over.Y); Start-Sleep -Milliseconds 150
[Rep]::Press(); Start-Sleep -Milliseconds 120
Shot '5-pressed-down'
[Rep]::Release(); Start-Sleep -Milliseconds 300
Shot '6-after-press'

Write-Host "7. drawing with the new tool"
[Rep]::MoveTo($r[0] + 100, $r[1] + 250); Start-Sleep -Milliseconds 100
[Rep]::Press()
for ($i = 1; $i -le 10; $i++) {
    [Rep]::MoveTo($r[0] + 100 + [int](220*$i/10), $r[1] + 250 + [int](120*$i/10))
    Start-Sleep -Milliseconds 20
}
[Rep]::Release(); Start-Sleep -Milliseconds 250
Shot '7-drawn-second'

Write-Host "8. hovering back over the strip"
[Rep]::MoveTo($over.X, $over.Y); Start-Sleep -Milliseconds 300
Shot '8-hover-again'

if (-not $proc.WaitForExit(6000)) { $proc.Kill(); Write-Host "  the overlay did not close" }
Write-Host "exit $($proc.ExitCode); output in $OutDir"
