<#
.SYNOPSIS
    Drives the selection overlay with real input and captures what it shows.

.DESCRIPTION
    The overlay is interactive, so the only way to know what it looks like is to use it.
    This runs `lasso select`, sends genuine pointer and key input through SendInput, and
    grabs the screen at chosen moments:

      1. drag out a region
      2. screenshot                 (the strip appeared, the hint went away)
      3. press a tool key
      4. drag inside the region to draw
      5. screenshot                 (the stroke is on the capture)
      6. Enter, so the run finishes

    The screenshots are taken by the same process that drives the input, which is what
    makes the timing observable instead of guessed.

.PARAMETER OutDir
    Directory for the screenshots and the saved capture.

.PARAMETER Rect
    The region to select, as "left,top,right,bottom" in virtual-desktop coordinates.

.PARAMETER Tool
    The key that arms the drawing tool: A arrow, R rectangle, E ellipse, P pencil.

.PARAMETER Stroke
    The stroke to draw inside the region, as "x1,y1,x2,y2" in the region's own
    coordinates.

.PARAMETER KeepOpen
    Do not send Enter at the end, leaving the overlay up for a person to look at.
#>
[CmdletBinding()]
param(
    [string]$OutDir = "overlay-check",
    [string]$Rect = "400,300,900,700",
    [string]$Tool = "R",
    [string]$Stroke = "40,40,300,200",
    [switch]$KeepOpen
)

$ErrorActionPreference = 'Stop'
# Resolved from the script's own place, not the current directory: a relative path here
# would be taken against wherever the caller happened to be.
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$exe = Join-Path $root 'target\debug\lasso.exe'
if (-not (Test-Path $exe)) { throw "no binary at $exe; build first" }
if (-not [System.IO.Path]::IsPathRooted($OutDir)) { $OutDir = Join-Path $root "target\$OutDir" }
if (-not (Test-Path $OutDir)) { New-Item -ItemType Directory -Path $OutDir -Force | Out-Null }
$OutDir = (Resolve-Path $OutDir).Path

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class Input {
    [StructLayout(LayoutKind.Sequential)]
    struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)]
    struct KEYBDINPUT { public ushort wVk, wScan; public uint dwFlags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Explicit)]
    struct INPUTUNION { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)]
    struct INPUT { public uint type; public INPUTUNION u; }

    const uint INPUT_MOUSE = 0, INPUT_KEYBOARD = 1;
    const uint LEFTDOWN = 0x0002, LEFTUP = 0x0004, MOVE = 0x0001;
    const uint KEYUP = 0x0002;

    [DllImport("user32.dll", SetLastError = true)]
    static extern uint SendInput(uint n, INPUT[] inputs, int size);

    public static void MoveTo(int x, int y) {
        // Absolute coordinates normalised over the virtual screen, which is what makes
        // this work on a multi-monitor desktop with a negative origin.
        int vx = GetSystemMetrics(76), vy = GetSystemMetrics(77);
        int vw = GetSystemMetrics(78), vh = GetSystemMetrics(79);
        var input = new INPUT { type = INPUT_MOUSE };
        input.u.mi.dx = (int)Math.Round((x - vx) * 65535.0 / (vw - 1));
        input.u.mi.dy = (int)Math.Round((y - vy) * 65535.0 / (vh - 1));
        input.u.mi.dwFlags = MOVE | 0x8000 | 0x4000; // MOVE | ABSOLUTE | VIRTUALDESK
        SendInput(1, new[] { input }, Marshal.SizeOf(typeof(INPUT)));
    }

    public static void LeftDown() { Send(MOUSEINPUT_FLAG(LEFTDOWN), 0, 0); }
    public static void LeftUp()   { Send(MOUSEINPUT_FLAG(LEFTUP), 0, 0); }

    static uint MOUSEINPUT_FLAG(uint flag) { return flag; }

    static void Send(uint flags, int dx, int dy) {
        var input = new INPUT { type = INPUT_MOUSE };
        input.u.mi.dwFlags = flags; input.u.mi.dx = dx; input.u.mi.dy = dy;
        SendInput(1, new[] { input }, Marshal.SizeOf(typeof(INPUT)));
    }

    public static void Key(ushort vk) {
        var down = new INPUT { type = INPUT_KEYBOARD };
        down.u.ki.wVk = vk;
        var up = down; up.u.ki.dwFlags = KEYUP;
        SendInput(1, new[] { down }, Marshal.SizeOf(typeof(INPUT)));
        System.Threading.Thread.Sleep(30);
        SendInput(1, new[] { up }, Marshal.SizeOf(typeof(INPUT)));
    }

    [DllImport("user32.dll")] static extern int GetSystemMetrics(int index);
}
'@

function Capture([string]$Name) {
    $x = [int]($Rect -split ',')[0]; $y = [int]($Rect -split ',')[1]
    $w = 1600; $h = 1000
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen(0, 0, 0, 0, (New-Object System.Drawing.Size($w, $h)))
    $g.Dispose()
    $path = Join-Path $OutDir "$Name.png"
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Host "  captured $Name.png"
}

$r = $Rect -split ',' | ForEach-Object { [int]$_ }
$s = $Stroke -split ',' | ForEach-Object { [int]$_ }

Write-Host "starting the overlay"
$proc = Start-Process -FilePath $exe -ArgumentList 'select', '--out', (Join-Path $OutDir 'result.png') -PassThru
Start-Sleep -Milliseconds 2500
if ($proc.HasExited) { throw "the overlay exited early with code $($proc.ExitCode)" }

Write-Host "dragging the region $Rect"
[Input]::MoveTo($r[0], $r[1]); Start-Sleep -Milliseconds 90
[Input]::LeftDown(); Start-Sleep -Milliseconds 90
# Several steps rather than one jump, so the overlay sees a drag and not a teleport.
for ($i = 1; $i -le 8; $i++) {
    $x = $r[0] + [int](($r[2] - $r[0]) * $i / 8)
    $y = $r[1] + [int](($r[3] - $r[1]) * $i / 8)
    [Input]::MoveTo($x, $y); Start-Sleep -Milliseconds 25
}
[Input]::LeftUp(); Start-Sleep -Milliseconds 250
Capture 'after-selection'

Write-Host "arming tool '$Tool'"
[Input]::Key([byte][char]$Tool); Start-Sleep -Milliseconds 250
Capture 'after-tool'

# The stroke is given in the region's own pixels, so the pointer has to be moved to the
# region's place on the desktop.
$x1 = $r[0] + $s[0]; $y1 = $r[1] + $s[1]
$x2 = $r[0] + $s[2]; $y2 = $r[1] + $s[3]
Write-Host "drawing from ($x1, $y1) to ($x2, $y2)"
[Input]::MoveTo($x1, $y1); Start-Sleep -Milliseconds 90
[Input]::LeftDown(); Start-Sleep -Milliseconds 90
for ($i = 1; $i -le 10; $i++) {
    $x = $x1 + [int](($x2 - $x1) * $i / 10)
    $y = $y1 + [int](($y2 - $y1) * $i / 10)
    [Input]::MoveTo($x, $y); Start-Sleep -Milliseconds 20
}
[Input]::LeftUp(); Start-Sleep -Milliseconds 250
Capture 'after-draw'

if (-not $KeepOpen) {
    Write-Host "pressing Enter to finish"
    [Input]::Key(0x0D)
    if (-not $proc.WaitForExit(6000)) { $proc.Kill(); throw "the overlay did not close" }
    Write-Host "exit code $($proc.ExitCode)"
}

Write-Host "output in $OutDir"
