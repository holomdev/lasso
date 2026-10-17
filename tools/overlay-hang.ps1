<#
.SYNOPSIS
    Finds out how many repaints it takes before the overlay stops answering.

.DESCRIPTION
    The report is "the icons go and nothing works". A window that draws and then stops
    answering has a dead message loop, and `SendMessageTimeout` with `SMTO_ABORTIFHUNG`
    answers exactly that without guessing: it returns zero when the window's thread does
    not pick the message up in time.

    So this drives the overlay, asks after every batch whether it is still alive, and says
    at which batch it stopped. A number instead of an opinion.
#>
[CmdletBinding()]
param(
    [string]$Rect = "600,250,1200,700",
    [int]$Batches = 40,
    [int]$PerBatch = 25
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$exe = Join-Path $root 'target\debug\lasso.exe'

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class Hp {
    [StructLayout(LayoutKind.Sequential)]
    struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)]
    struct INPUT { public uint type; public MOUSEINPUT mi; }
    [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] i, int s);
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int i);
    delegate bool EnumProc(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr p);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);

    /// The process's overlay window, found by walking the top-level windows.
    ///
    /// `FindWindow` by class did not find it, and a tool window is not guaranteed a main
    /// window handle, so neither of the obvious routes works. Walking every top-level
    /// window and matching the owning process does not depend on either.
    public static IntPtr OverlayWindow(uint pid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, p) => {
            uint owner;
            GetWindowThreadProcessId(h, out owner);
            if (owner != pid) return true;
            var name = new StringBuilder(256);
            if (GetClassNameW(h, name, name.Capacity) > 0 && name.ToString() == "LassoOverlay") {
                found = h;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    [DllImport("user32.dll", SetLastError = true)]
    static extern IntPtr SendMessageTimeout(IntPtr h, uint msg, IntPtr w, IntPtr l, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);

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

    /// Is the window's thread answering? False means the message loop is gone.
    public static bool Answers(IntPtr window) {
        IntPtr result;
        // SMTO_ABORTIFHUNG | SMTO_BLOCK, 150 ms.
        IntPtr ok = SendMessageTimeout(window, 0x0000, IntPtr.Zero, IntPtr.Zero, 0x0002 | 0x0001, 150, out result);
        return ok != IntPtr.Zero;
    }
}
'@

$points = $Rect -split ',' | ForEach-Object { [int]$_ }
$err = Join-Path $root 'target\hang-stderr.txt'
Remove-Item $err -ErrorAction SilentlyContinue

$proc = Start-Process -FilePath $exe `
    -ArgumentList 'select', '--out', (Join-Path $root 'target\hang.png') `
    -RedirectStandardError $err -PassThru -WindowStyle Hidden
Start-Sleep -Milliseconds 2500
if ($proc.HasExited) { throw "the overlay exited early; see $err" }

$window = [Hp]::OverlayWindow($proc.Id)
Write-Host "overlay pid $($proc.Id), window handle $window"
if ($window -eq [IntPtr]::Zero) { throw "could not find the overlay window" }

# A region to work on.
[Hp]::MoveTo($points[0], $points[1]); Start-Sleep -Milliseconds 90
[Hp]::Press()
for ($i = 1; $i -le 6; $i++) {
    [Hp]::MoveTo($points[0] + [int](($points[2]-$points[0])*$i/6), $points[1] + [int](($points[3]-$points[1])*$i/6))
    Start-Sleep -Milliseconds 15
}
[Hp]::Release(); Start-Sleep -Milliseconds 200

$total = 0
for ($batch = 1; $batch -le $Batches; $batch++) {
    for ($k = 0; $k -lt $PerBatch; $k++) {
        # Movement inside the selection: each one is a repaint.
        $x = $points[0] + 40 + (($k * 13) % 300)
        $y = $points[1] + 40 + (($k * 7) % 200)
        [Hp]::MoveTo($x, $y)
        Start-Sleep -Milliseconds 8
        $total++
    }

    $answers = [Hp]::Answers($window)
    Write-Host ("  batch {0,3}: {1,5} moves, answers = {2}" -f $batch, $total, $answers)
    if (-not $answers) {
        Write-Host "  STOPPED ANSWERING after $total pointer moves"
        break
    }
}

if (-not $proc.HasExited) { $proc.Kill() }
Write-Host ""
Write-Host "=== stderr ==="
Get-Content $err -ErrorAction SilentlyContinue | Select-Object -Last 20
