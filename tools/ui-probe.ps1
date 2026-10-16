<#
.SYNOPSIS
    Captures the settings window with both methods, for comparison.

.DESCRIPTION
    There are two ways to capture a window. `PrintWindow` asks the window to draw
    itself into a bitmap of its own size, so it works for a window that is off the
    display. A screen copy reads the pixels the display actually holds, so it works
    whatever the window thinks it is drawing, and fails only for the part of the
    window that is not on screen.

    Printing and screen-copying the same window tells the two apart: a difference
    between them is a difference between what the window says it draws and what is
    on the display, and a black region that only one of them has belongs to that
    one method rather than to the window.

    The window is moved to the top-left corner first so that as much of it as the
    display allows is inside the screen.

.PARAMETER Dpi
    The DPI to claim before capturing. Omit to capture at the display's own scale.

.PARAMETER Arguments
    Extra arguments for `lasso settings`, such as `--dpi 192`.

.PARAMETER Out
    Prefix for the two PNGs, relative to the project root.

.EXAMPLE
    .\tools\ui-probe.ps1 -Dpi 192 -Out target\test-output\probe
#>
[CmdletBinding()]
param(
    [int]$Dpi = 0,
    [string]$Arguments = '',
    [string]$Out = 'target\test-output\probe'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root 'target\debug\lasso.exe'

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
$window = $process.MainWindowHandle

Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.Runtime.InteropServices;
public static class Probe {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, IntPtr pid);
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
    [DllImport("user32.dll")] public static extern bool PeekMessageW(out MSG msg, IntPtr hWnd, uint min, uint max, uint remove);
    [DllImport("user32.dll")] public static extern bool TranslateMessage(ref MSG msg);
    [DllImport("user32.dll")] public static extern IntPtr DispatchMessageW(ref MSG msg);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct MSG {
        public IntPtr hwnd; public uint message; public IntPtr wParam; public IntPtr lParam;
        public uint time; public POINT pt;
    }

    public static void Raise(IntPtr hWnd) {
        IntPtr foreground = GetForegroundWindow();
        uint other = GetWindowThreadProcessId(foreground, IntPtr.Zero);
        uint self = GetCurrentThreadId();
        bool attached = other != 0 && other != self && AttachThreadInput(self, other, true);
        SetWindowPos(hWnd, IntPtr.Zero, 0, 0, 0, 0, 0x0001 | 0x0004 | 0x0010);
        SetForegroundWindow(hWnd);
        Pump(500);
        if (attached) { AttachThreadInput(self, other, false); }
    }

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

    public static void ChangeDpi(IntPtr hWnd, uint dpi) {
        RECT rect;
        if (!GetWindowRect(hWnd, out rect)) { throw new InvalidOperationException("GetWindowRect failed"); }
        IntPtr block = Marshal.AllocHGlobal(Marshal.SizeOf(typeof(RECT)));
        try {
            Marshal.StructureToPtr(rect, block, false);
            SendMessageW(hWnd, 0x02E0, (IntPtr)(long)dpi, block);
        } finally {
            Marshal.FreeHGlobal(block);
        }
    }

    public static Bitmap Printed(IntPtr hWnd) {
        RECT rect;
        if (!GetWindowRect(hWnd, out rect)) { throw new InvalidOperationException("GetWindowRect failed"); }
        Bitmap bitmap = new Bitmap(rect.Right - rect.Left, rect.Bottom - rect.Top);
        using (Graphics graphics = Graphics.FromImage(bitmap)) {
            graphics.Clear(Color.Magenta);
            IntPtr dc = graphics.GetHdc();
            try {
                if (!PrintWindow(hWnd, dc, 2)) { throw new InvalidOperationException("PrintWindow failed"); }
            } finally {
                graphics.ReleaseHdc(dc);
            }
        }
        return bitmap;
    }

    // A copy of what the display holds, clipped to the part that is on screen.
    public static Bitmap FromScreen(IntPtr hWnd, int screenWidth, int screenHeight) {
        RECT rect;
        if (!GetWindowRect(hWnd, out rect)) { throw new InvalidOperationException("GetWindowRect failed"); }
        int left = Math.Max(rect.Left, 0);
        int top = Math.Max(rect.Top, 0);
        int right = Math.Min(rect.Right, screenWidth);
        int bottom = Math.Min(rect.Bottom, screenHeight);
        Bitmap bitmap = new Bitmap(Math.Max(right - left, 1), Math.Max(bottom - top, 1));
        using (Graphics graphics = Graphics.FromImage(bitmap)) {
            graphics.CopyFromScreen(left, top, 0, 0, bitmap.Size);
        }
        return bitmap;
    }
}
'@

[Probe]::Raise($window)
if ($Dpi -gt 0) {
    [Probe]::ChangeDpi($window, [uint32]$Dpi)
    [Probe]::Pump(800)
}

Add-Type -AssemblyName System.Windows.Forms
$bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
Write-Host "screen: $($bounds.Width)x$($bounds.Height)"

$printed = [Probe]::Printed($window)
$printedSize = "$($printed.Width)x$($printed.Height)"
$printed.Save((Join-Path $root "$Out-print.png"), [System.Drawing.Imaging.ImageFormat]::Png)
$printed.Dispose()

$copied = [Probe]::FromScreen($window, $bounds.Width, $bounds.Height)
$copiedSize = "$($copied.Width)x$($copied.Height)"
$copied.Save((Join-Path $root "$Out-screen.png"), [System.Drawing.Imaging.ImageFormat]::Png)
$copied.Dispose()

Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
Write-Host "printed: $printedSize  screen copy: $copiedSize -> ${Out}-print.png / ${Out}-screen.png"
