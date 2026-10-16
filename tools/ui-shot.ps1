<#
.SYNOPSIS
    Starts the settings window, captures it to a PNG, and closes it again.

.DESCRIPTION
    `lasso capture` grabs the whole virtual desktop, so whatever else is on
    screen - an editor, a console - lands in the frame, often on top of the
    window being checked. This starts the window itself and captures it on its
    own, which is what a layout review needs to look at.

    The capture goes through `PrintWindow` rather than a screen copy. A screen copy
    reads the pixels that happen to be on the display, so a window that is partly
    off it - which a scaled-up layout can easily be - comes back with the desktop
    or nothing at all where the window is not. `PrintWindow` asks the window to
    draw itself into a bitmap of its own size, wherever it is.

    Everything happens in one process on purpose. A window started by one command
    and captured by another belongs to a process group that is gone by the time
    the second one runs, so the second one finds nothing to capture.

.PARAMETER Out
    Destination PNG path, relative to the project root.

.PARAMETER Arguments
    Extra arguments for `lasso settings`, such as `--dpi 144`.

.PARAMETER Keep
    Leave the window open instead of closing it when the capture is done.

.EXAMPLE
    .\tools\ui-shot.ps1 -Out target\test-output\ui.png

.EXAMPLE
    .\tools\ui-shot.ps1 -Arguments '--dpi 144' -Out target\test-output\ui-150.png
#>
[CmdletBinding()]
param(
    [string]$Out = 'target\test-output\ui.png',
    [string]$Arguments = '',
    [switch]$Keep
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root 'target\debug\lasso.exe'

if (-not (Test-Path $exe)) {
    throw "not built yet: $exe"
}

$settingsArguments = @('settings')
if ($Arguments) {
    $settingsArguments += $Arguments.Split(' ', [System.StringSplitOptions]::RemoveEmptyEntries)
}

$process = Start-Process $exe -ArgumentList $settingsArguments `
    -RedirectStandardOutput (Join-Path $root 'target\test-output\settings-out.txt') `
    -RedirectStandardError (Join-Path $root 'target\test-output\settings-err.txt') `
    -PassThru

for ($attempt = 0; $attempt -lt 40; $attempt++) {
    $process.Refresh()
    if ($process.MainWindowHandle -ne 0) { break }
    Start-Sleep -Milliseconds 250
}
if ($process.MainWindowHandle -eq 0) {
    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    throw 'the settings window never appeared'
}

Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.Runtime.InteropServices;
public static class Grab {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, IntPtr pid);
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
    [DllImport("user32.dll")] public static extern bool PeekMessageW(out MSG msg, IntPtr hWnd, uint min, uint max, uint remove);
    [DllImport("user32.dll")] public static extern bool TranslateMessage(ref MSG msg);
    [DllImport("user32.dll")] public static extern IntPtr DispatchMessageW(ref MSG msg);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct MSG {
        public IntPtr hwnd; public uint message; public IntPtr wParam; public IntPtr lParam;
        public uint time; public POINT pt;
    }

    // A process started by an agent does not own the foreground, so Windows
    // refuses to raise its window unless the request comes from the thread that
    // owns the current foreground window. Attaching to that window's input queue
    // is the documented way around it.
    public static void Raise(IntPtr hWnd) {
        IntPtr foreground = GetForegroundWindow();
        uint other = GetWindowThreadProcessId(foreground, IntPtr.Zero);
        uint self = GetCurrentThreadId();
        bool attached = other != 0 && other != self && AttachThreadInput(self, other, true);
        ShowWindow(hWnd, 9);
        SetForegroundWindow(hWnd);
        Pump(400);
        if (attached) { AttachThreadInput(self, other, false); }
    }

    // The window paints itself in response to the messages Windows posts, so a
    // capture taken without pumping can show a half-drawn state that is not what
    // the user sees.
    public static void Pump(int milliseconds) {
        MSG msg;
        DateTime deadline = DateTime.UtcNow.AddMilliseconds(milliseconds);
        while (DateTime.UtcNow < deadline) {
            if (PeekMessageW(out msg, IntPtr.Zero, 0, 0, 1)) {
                TranslateMessage(ref msg);
                DispatchMessageW(ref msg);
            } else {
                System.Threading.Thread.Sleep(10);
            }
        }
    }

    // PW_RENDERFULLCONTENT asks for the window as composed, which is what a
    // per-monitor DPI aware window needs: without it a scaled window can come back
    // blank.
    public static Bitmap Capture(IntPtr hWnd) {
        RECT rect;
        if (!GetWindowRect(hWnd, out rect)) { throw new InvalidOperationException("GetWindowRect failed"); }
        int width = rect.Right - rect.Left;
        int height = rect.Bottom - rect.Top;
        Bitmap bitmap = new Bitmap(width, height);
        using (Graphics graphics = Graphics.FromImage(bitmap)) {
            IntPtr dc = graphics.GetHdc();
            try {
                if (!PrintWindow(hWnd, dc, 2)) { throw new InvalidOperationException("PrintWindow failed"); }
            } finally {
                graphics.ReleaseHdc(dc);
            }
        }
        return bitmap;
    }
}
'@

[Grab]::Raise($process.MainWindowHandle)
[Grab]::Pump(600)

$bitmap = [Grab]::Capture($process.MainWindowHandle)

$destination = Join-Path $root $Out
$directory = Split-Path -Parent $destination
if ($directory -and -not (Test-Path $directory)) {
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
}
$bitmap.Save($destination, [System.Drawing.Imaging.ImageFormat]::Png)
$size = "$($bitmap.Width)x$($bitmap.Height)"
$bitmap.Dispose()

if (-not $Keep) {
    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
}

Write-Host "window captured: $size -> $Out"
