<#
.SYNOPSIS
    Drives the settings window and reports what it saved.

.DESCRIPTION
    A layout change can break reading and writing without breaking anything that
    is visible in a capture: a control that is never filled, a check box that comes
    back unchecked, a button that no longer sends the message the window handles.
    This sets the controls directly, presses the buttons, and then reads the
    settings file back.

    It also exercises a scale change with unsaved edits in place, because that is
    the one sequence where the window replaces its controls and has to carry the
    edits across.

.PARAMETER Dpi
    A DPI to claim before saving, to check that a scale change keeps the values.

.PARAMETER Keep
    Do not restore the original settings file afterwards.

.EXAMPLE
    .\tools\ui-roundtrip.ps1 -Dpi 192
#>
[CmdletBinding()]
param(
    [int]$Dpi = 0,
    [switch]$Keep
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root 'target\debug\lasso.exe'
$folder = Join-Path $root 'target\test-output\roundtrip-folder'

# The settings live under %APPDATA%, and this tool is often run from a sandbox
# that only allows writes inside the project. Pointing the window at an
# application-data directory under `target/` exercises the same code and the same
# file format without touching the real one, and without needing a wider sandbox.
$appData = Join-Path $root 'target\test-output\roundtrip-appdata'
$settings = Join-Path $appData 'lasso\lasso.ini'
$backup = Join-Path $root 'target\test-output\settings-before.ini'

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class Drive {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr hWnd, StringBuilder text, int count);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr parent, int id);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, IntPtr pid);
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool PeekMessageW(out MSG msg, IntPtr hWnd, uint min, uint max, uint remove);
    [DllImport("user32.dll")] public static extern bool TranslateMessage(ref MSG msg);
    [DllImport("user32.dll")] public static extern IntPtr DispatchMessageW(ref MSG msg);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageTimeoutW(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam, uint flags, uint timeout, out IntPtr result);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct MSG {
        public IntPtr hwnd; public uint message; public IntPtr wParam; public IntPtr lParam;
        public uint time; public POINT pt;
    }

    public const uint WM_COMMAND = 0x0111;
    public const uint WM_DPICHANGED = 0x02E0;
    public const uint BM_SETCHECK = 0x00F1;
    public const uint BM_GETCHECK = 0x00F0;

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

    public static void Focus(IntPtr hWnd) {
        IntPtr foreground = GetForegroundWindow();
        uint other = GetWindowThreadProcessId(foreground, IntPtr.Zero);
        uint self = GetCurrentThreadId();
        bool attached = other != 0 && other != self && AttachThreadInput(self, other, true);
        SetForegroundWindow(hWnd);
        Pump(300);
        if (attached) { AttachThreadInput(self, other, false); }
    }

    public static void Click(IntPtr parent, int id) {
        IntPtr control = GetDlgItem(parent, id);
        if (control == IntPtr.Zero) { throw new InvalidOperationException("no control " + id); }
        SendMessageW(parent, WM_COMMAND, (IntPtr)id, control);
    }

    public static void SetCheck(IntPtr parent, int id, bool on) {
        IntPtr control = GetDlgItem(parent, id);
        if (control == IntPtr.Zero) { throw new InvalidOperationException("no control " + id); }
        SendMessageW(control, BM_SETCHECK, (IntPtr)(on ? 1 : 0), IntPtr.Zero);
    }

    public static bool GetCheck(IntPtr parent, int id) {
        IntPtr control = GetDlgItem(parent, id);
        return SendMessageW(control, BM_GETCHECK, IntPtr.Zero, IntPtr.Zero) == (IntPtr)1;
    }

    public static string GetText(IntPtr parent, int id) {
        IntPtr control = GetDlgItem(parent, id);
        StringBuilder text = new StringBuilder(512);
        GetWindowTextW(control, text, text.Capacity);
        return text.ToString();
    }

    [DllImport("user32.dll")] public static extern IntPtr SetFocus(IntPtr hWnd);

    // Type into a control the way the keyboard does: focus it, select everything,
    // and send one character at a time. Setting the text directly is a different
    // path - it crosses process boundaries as a single message - and it is not
    // what a person does.
    public static void Type(IntPtr parent, int id, string text) {
        IntPtr control = GetDlgItem(parent, id);
        if (control == IntPtr.Zero) { throw new InvalidOperationException("no control " + id); }
        SetFocus(control);
        Pump(150);
        // EM_SETSEL with 0 and -1 selects the whole contents.
        SendMessageW(control, 0x00B1, IntPtr.Zero, (IntPtr)(-1));
        Pump(100);
        uint WM_CHAR = 0x0102;
        uint WM_KEYDOWN = 0x0100;
        uint VK_DELETE = 0x2E;
        SendMessageW(control, WM_KEYDOWN, (IntPtr)VK_DELETE, IntPtr.Zero);
        Pump(100);
        foreach (char c in text) {
            SendMessageW(control, WM_CHAR, (IntPtr)c, IntPtr.Zero);
            Pump(20);
        }
        Pump(200);
    }

    public static void ChangeDpi(IntPtr hWnd, uint dpi) {
        SendMessageW(hWnd, WM_DPICHANGED, (IntPtr)(long)dpi, IntPtr.Zero);
    }
}
'@

New-Item -ItemType Directory -Force -Path $folder | Out-Null
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $backup) | Out-Null
if (Test-Path $settings) {
    Copy-Item $settings $backup -Force
} else {
    Set-Content -Path $backup -Value '' -NoNewline
}

$failures = 0
function Check([string]$what, $expected, $actual) {
    if ("$expected" -eq "$actual") {
        Write-Host "  ok   $what = $actual"
    } else {
        Write-Host "  FAIL $what expected '$expected', got '$actual'"
        $script:failures++
    }
}

# The child inherits this process's environment, which is how the window is
# pointed at the application-data directory above.
$previousAppData = $env:APPDATA
$env:APPDATA = $appData
$process = Start-Process $exe -ArgumentList 'settings' `
    -RedirectStandardOutput (Join-Path $root 'target\test-output\roundtrip-out.txt') `
    -RedirectStandardError (Join-Path $root 'target\test-output\roundtrip-err.txt') `
    -PassThru
$env:APPDATA = $previousAppData

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
[Drive]::Focus($window)

Write-Host 'loaded values'
# These read what the window is showing, from outside it. That is what a capture
# does, and it is enough to tell a control that was never filled from one that was.
# The values the window actually hands on are checked in the saved file at the end,
# which is the only place a cross-process read cannot mislead.
Check 'clipboard hint is shown' $true ([Drive]::GetText($window, 101).Length -gt 0)
Check 'tray click button is filled' $true ([Drive]::GetText($window, 107).Length -gt 0)
# A fresh profile has no output folder yet, so the field is empty until one is
# chosen; what matters here is that the hint says so rather than that it is blank.
Check 'output hint is shown' $true ([Drive]::GetText($window, 106).Length -gt 0)

Write-Host 'edits before a scale change'
# The folder is typed rather than set. A cross-process `SetWindowTextW` is a single
# message carrying a marshalled string, which is not the path a person takes and
# not a path this window has to support; keystrokes are.
#
# The folder is also not read back from here. Reading a control's text from outside
# the process is the harness talking to itself, and it does not reliably see what
# the window sees; the value is checked in the saved file instead, which is what
# the window actually handed on.
[Drive]::Type($window, 104, $folder)
[Drive]::SetCheck($window, 102, $false)
# The pointer check box starts off, so turning it on is a change this can see.
[Drive]::SetCheck($window, 109, $true)
[Drive]::SetCheck($window, 103, $true)

if ($Dpi -gt 0) {
    Write-Host 'changing scale with unsaved edits'
    [Drive]::ChangeDpi($window, [uint32]$Dpi)
    [Drive]::Pump(600)
    Check 'clipboard survived the scale change' $false ([Drive]::GetCheck($window, 102))
    Check 'the pointer survived the scale change' $true ([Drive]::GetCheck($window, 109))
    Check 'save-to-file survived the scale change' $true ([Drive]::GetCheck($window, 103))
}

Write-Host 'changing what a left click does, then saving'
[Drive]::Click($window, 107)
[Drive]::Click($window, 1)
[Drive]::Pump(600)

for ($attempt = 0; $attempt -lt 20; $attempt++) {
    $process.Refresh()
    if ($process.HasExited) { break }
    Start-Sleep -Milliseconds 200
}
if (-not $process.HasExited) {
    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
}

Write-Host "settings file: $settings"
$saved = Get-Content $settings -Raw
Write-Host $saved.Trim()
$savedLines = $saved -split "`r?`n"

function ValueOf([string]$key) {
    foreach ($line in $savedLines) {
        if ($line -match "^\s*$key\s*=\s*(.*)$") { return $Matches[1].Trim() }
    }
    return '<missing>'
}

Check 'output_dir saved' $folder (ValueOf 'output_dir')
Check 'clipboard saved' 'no' (ValueOf 'clipboard')
Check 'cursor saved' 'yes' (ValueOf 'cursor')
Check 'save_file saved' 'yes' (ValueOf 'save_file')
Check 'tray_left_click toggled' 'capture' (ValueOf 'tray_left_click')
Check 'hotkey untouched' 'Ctrl+Shift+X' (ValueOf 'hotkey')

if (-not $Keep) {
    if ((Get-Content $backup -Raw) -ne '') {
        Copy-Item $backup $settings -Force
    } else {
        Remove-Item $settings -Force -ErrorAction SilentlyContinue
    }
    Write-Host 'settings restored'
}

Write-Output ''
if ($failures -eq 0) {
    Write-Host 'round trip ok'
    exit 0
} else {
    Write-Host "$failures check(s) failed"
    exit 1
}
