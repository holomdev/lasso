<#
.SYNOPSIS
    Reports the settings window's frame and client geometry.

.DESCRIPTION
    A capture shows what the window looks like; it does not say whether the window
    is the size the layout asked for. This prints both rectangles and the layout's
    numbers so one can be checked against the other, which is what catches a
    window that was sized from the layout before the display's scale was known.

    The window is started and closed in this same process, because a window started
    by one command belongs to a process group that is gone by the time the next one
    runs.

.EXAMPLE
    .\tools\ui-size.ps1
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root 'target\debug\lasso.exe'

$process = Start-Process $exe -ArgumentList 'settings' `
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

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Geo {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hWnd);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
'@

$window = $process.MainWindowHandle
$frame = New-Object Geo+RECT
$client = New-Object Geo+RECT
[void][Geo]::GetWindowRect($window, [ref]$frame)
[void][Geo]::GetClientRect($window, [ref]$client)
$dpi = [Geo]::GetDpiForWindow($window)

Write-Host "dpi          : $dpi"
Write-Host "window rect  : $($frame.Right - $frame.Left) x $($frame.Bottom - $frame.Top) at ($($frame.Left), $($frame.Top))"
Write-Host "client rect  : $($client.Right - $client.Left) x $($client.Bottom - $client.Top)"
Write-Host "border       : $((($frame.Right - $frame.Left) - ($client.Right - $client.Left))) horizontal, $((($frame.Bottom - $frame.Top) - ($client.Bottom - $client.Top))) vertical"

Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
