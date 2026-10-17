<#
.SYNOPSIS
    Draws a stroke, then leaves the overlay alone and captures it every second.

.DESCRIPTION
    The report is that the strip loses its icons and the drawing goes black a few seconds
    after drawing, with no action in between. Nothing that presses keys can reproduce that,
    so this does the opposite: it draws once, then sits still and takes a screenshot every
    second.

    With `LASSO_DUMP_CANVAS` set, the overlay also writes its own composed buffer, so what
    reached the buffer can be told apart from what the desktop did to the window.
#>
[CmdletBinding()]
param(
    [string]$OutDir = "target\overlay-idle",
    [string]$Rect = "700,250,1300,700",
    [int]$Seconds = 8
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
public static class Idle {
    // Flattened rather than a managed union: C# has no union a struct may hold, and the
    // mouse form is the one that begins at the same offset anyway.
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

$r = $Rect -split ',' | ForEach-Object { [int]$_ }

function Shot([string]$Name) {
    $bmp = New-Object System.Drawing.Bitmap(1400, 900)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen(0, 0, 0, 0, (New-Object System.Drawing.Size(1400, 900)))
    $g.Dispose()
    $bmp.Save((Join-Path $OutDir "$Name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}

$env:LASSO_DUMP_CANVAS = Join-Path $OutDir 'canvas'
# A log written by the script itself, so a run that dies part way still says how far it got.
# Capturing the console instead loses everything if the process is ended by something else.
$log = Join-Path $OutDir 'run.txt'
function Say([string]$Text) {
    Write-Host $Text
    Add-Content -Path $log -Value $Text
}
Say "starting; dumps to $env:LASSO_DUMP_CANVAS"

$proc = Start-Process -FilePath $exe -ArgumentList 'select', '--out', (Join-Path $OutDir 'result.png') -PassThru
Start-Sleep -Milliseconds 2200
if ($proc.HasExited) { Say "the overlay exited early"; throw "the overlay exited early" }
Say "overlay up, pid $($proc.Id)"

Say "selecting $Rect and drawing"
[Idle]::MoveTo($r[0], $r[1]); Start-Sleep -Milliseconds 90
[Idle]::Press()
for ($i = 1; $i -le 8; $i++) {
    [Idle]::MoveTo($r[0] + [int](($r[2]-$r[0])*$i/8), $r[1] + [int](($r[3]-$r[1])*$i/8))
    Start-Sleep -Milliseconds 20
}
[Idle]::Release(); Start-Sleep -Milliseconds 200

# Arm the rectangle tool by key, so the pointer never has to visit the strip: the report
# says the fault happens without any action, so visiting it is not required.
[Idle]::MoveTo($r[0] + 250, $r[1] + 200); Start-Sleep -Milliseconds 120
[Idle]::Press()
for ($i = 1; $i -le 8; $i++) {
    [Idle]::MoveTo($r[0] + 250 + [int](200*$i/8), $r[1] + 200 + [int](140*$i/8))
    Start-Sleep -Milliseconds 20
}
[Idle]::Release(); Start-Sleep -Milliseconds 300
Shot 'idle-00'

# Now do nothing at all, and watch.
for ($i = 1; $i -le $Seconds; $i++) {
    Start-Sleep -Seconds 1
    Shot ("idle-{0:d2}" -f $i)
    if ($proc.HasExited) { Say "  the overlay exited at $i s"; break }
}

if (-not $proc.HasExited) { $proc.Kill() }
Say "done; output in $OutDir"
