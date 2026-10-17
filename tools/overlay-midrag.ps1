<#
.SYNOPSIS
    Checks that the strip is still on screen in the middle of a drag.

.DESCRIPTION
    The report is that the strip disappears while a stroke is being drawn and comes back
    when the drag ends. That only exists *during* the drag, so a screenshot taken afterwards
    cannot see it: this presses, moves, and captures **without releasing**, then releases and
    captures again. Both are measured, so the two can be compared.
#>
[CmdletBinding()]
param(
    [string]$Rect = "600,250,1200,700",
    [string]$OutDir = "target\overlay-midrag"
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
public static class Mid {
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

    static void Mouse(uint flags) {
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
    public static void Press()   { Mouse(0x0002); }
    public static void Release() { Mouse(0x0004); }
    public static void Key(ushort vk) {
        var down = new INPUT { type = 1 }; down.u.ki.wVk = vk;
        var up = down; up.u.ki.dwFlags = 0x0002;
        SendInput(1, new[] { down }, Marshal.SizeOf(typeof(INPUT)));
        System.Threading.Thread.Sleep(30);
        SendInput(1, new[] { up }, Marshal.SizeOf(typeof(INPUT)));
    }
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

$proc = Start-Process -FilePath $exe -ArgumentList 'select', '--out', (Join-Path $OutDir 'result.png') -PassThru
Start-Sleep -Milliseconds 2500
if ($proc.HasExited) { throw "the overlay exited early" }

# A region to draw on.
[Mid]::MoveTo($r[0], $r[1]); Start-Sleep -Milliseconds 90
[Mid]::Press()
for ($i = 1; $i -le 6; $i++) {
    [Mid]::MoveTo($r[0] + [int](($r[2]-$r[0])*$i/6), $r[1] + [int](($r[3]-$r[1])*$i/6))
    Start-Sleep -Milliseconds 20
}
[Mid]::Release(); Start-Sleep -Milliseconds 300
Shot '0-selected'

# Each drawing tool: armed by its key, then held mid-stroke.
foreach ($tool in @(
    @{ Key = 0x41; Name = 'arrow' },
    @{ Key = 0x52; Name = 'rectangle' },
    @{ Key = 0x45; Name = 'ellipse' },
    @{ Key = 0x50; Name = 'pencil' }
)) {
    [Mid]::MoveTo($r[0] + 300, $r[1] + 300); Start-Sleep -Milliseconds 120
    [Mid]::Key([uint16]$tool.Key); Start-Sleep -Milliseconds 200
    [Mid]::MoveTo($r[0] + 80, $r[1] + 80); Start-Sleep -Milliseconds 80
    [Mid]::Press()
    for ($k = 1; $k -le 6; $k++) {
        [Mid]::MoveTo($r[0] + 80 + 28*$k, $r[1] + 80 + 18*$k)
        Start-Sleep -Milliseconds 25
    }
    # Still held down: this is the moment the strip used to vanish.
    Shot ("1-mid-$($tool.Name)")
    [Mid]::Release(); Start-Sleep -Milliseconds 300
    Shot ("2-after-$($tool.Name)")
    Write-Host "  drew with $($tool.Name)"
}

if (-not $proc.HasExited) { $proc.Kill() }
Write-Host "output in $OutDir"
