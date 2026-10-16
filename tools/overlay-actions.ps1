<#
.SYNOPSIS
    Presses the strip's action buttons and reports what each one did.

.DESCRIPTION
    Three ways out, and each has to be checked by its effect rather than by the code
    looking right:

      * Save    - a file appears, holding the region
      * Copy    - the clipboard holds the region, and no file is written
      * Cancel  - neither happens

    The buttons are drawn rather than being real controls, so synthetic input is the only
    way to press one. Their positions come from the same arithmetic the program uses:
    a 26px button, a 2px gap, 4px of padding, and 5px more above any button that starts a
    group.
#>
[CmdletBinding()]
param(
    [string]$OutDir = "target\overlay-actions",
    [string]$Rect = "700,300,1100,650"
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$exe = Join-Path $root 'target\debug\lasso.exe'
if (-not (Test-Path $exe)) { throw "no binary at $exe; build first" }
if (-not [System.IO.Path]::IsPathRooted($OutDir)) { $OutDir = Join-Path $root $OutDir }
if (Test-Path $OutDir) { Remove-Item $OutDir -Recurse -Force }
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Act {
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

    static void Flags(uint flags) {
        var input = new INPUT { type = 0 };
        input.u.mi.dwFlags = flags;
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

# The strip's geometry, matching `toolbar::Metrics::BASE`.
$button = 26; $gap = 2; $padding = 4; $groupGap = 5; $standoff = 8
$stripWidth = $button + $padding * 2
$stripHeight = 12 * $button + 11 * $gap + 3 * $groupGap + $padding * 2
# Indices in Button::ALL that begin a group: Colour, Undo, Save.
$groupStarts = @(5, 7, 9)

# The same walk `Metrics::button_rects` does, which is what keeps the two in step. The
# group gap goes *before* each button that starts a group except the first, not after it:
# getting that the wrong way round put every button tens of pixels from where it is
# drawn, and a click on the plate does nothing at all.
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

function SelectRegion {
    [Act]::MoveTo($r[0], $r[1]); Start-Sleep -Milliseconds 90
    [Act]::Press()
    for ($i = 1; $i -le 8; $i++) {
        $x = $r[0] + [int](($r[2] - $r[0]) * $i / 8)
        $y = $r[1] + [int](($r[3] - $r[1]) * $i / 8)
        [Act]::MoveTo($x, $y); Start-Sleep -Milliseconds 20
    }
    [Act]::Release()
    Start-Sleep -Milliseconds 220
}

function TryCase([string]$Name, [int]$Index) {
    $out = Join-Path $OutDir "$Name.png"
    Remove-Item $out -ErrorAction SilentlyContinue
    [System.Windows.Forms.Clipboard]::Clear()
    Start-Sleep -Milliseconds 150

    $proc = Start-Process -FilePath $exe -ArgumentList 'select', '--out', $out -PassThru
    Start-Sleep -Milliseconds 2200
    if ($proc.HasExited) { throw "$Name`: the overlay exited early" }

    SelectRegion
    $bx = [int]($stripX + $padding + $button / 2)
    $by = [int]($stripY + (ButtonTop $Index) + $button / 2)
    [Act]::MoveTo($bx, $by); Start-Sleep -Milliseconds 180
    [Act]::Press(); Start-Sleep -Milliseconds 60; [Act]::Release()

    if (-not $proc.WaitForExit(6000)) { $proc.Kill(); throw "$Name`: the overlay did not close" }

    $file = Test-Path $out
    $clip = "empty"
    try {
        if ([System.Windows.Forms.Clipboard]::ContainsImage()) {
            $img = [System.Windows.Forms.Clipboard]::GetImage()
            $clip = "$($img.Width)x$($img.Height)"
            $img.Dispose()
        }
    } catch { $clip = "unreadable" }

    [pscustomobject]@{
        Case        = $Name
        Exit        = $proc.ExitCode
        FileWritten = $file
        Bytes       = if ($file) { (Get-Item $out).Length } else { 0 }
        Clipboard   = $clip
    }
}

Write-Host "region $Rect; strip at ($stripX, $stripY), ${stripWidth}x${stripHeight}"
$results = @()
$results += TryCase 'save' 9
$results += TryCase 'copy' 10
$results += TryCase 'cancel' 11
$results | Format-Table -AutoSize
