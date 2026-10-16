<#
.SYNOPSIS
    Checks that `--cursor` draws the mouse pointer, on both backends.

.DESCRIPTION
    Comparing two captures of a live desktop compares the desktop as well: a
    clock, a spinner or a window repainting between the two shows up as a diff
    over the whole screen, which is what the first version of this check found.
    So it puts a flat, still window of its own on screen first. The background is
    then constant by construction, and everything that differs between a capture
    with the pointer and one without it is the pointer.

    The pointer is not moved. Wherever it is, it is drawn at its position, and the
    check reads that position rather than assuming one - a sandboxed process is
    allowed to read the pointer and refused permission to set it. The window is
    placed around the pointer instead, which needs no permission at all.

    What this can see that a unit test cannot: whether the shape survives being
    read out of the system and drawn into a real frame, whether it lands on the
    pointer's own position rather than one hotspot away, and whether the two
    backends agree.

.PARAMETER Out
    Prefix for the PNGs, relative to the project root.

.EXAMPLE
    .\tools\cursor-check.ps1
#>
[CmdletBinding()]
param(
    [string]$Out = 'target\test-output\cursor'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root 'target\debug\lasso.exe'

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Pointer {
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT point);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
}
'@
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

$at = New-Object Pointer+POINT
if (-not [Pointer]::GetCursorPos([ref]$at)) { throw 'GetCursorPos failed' }

$screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$side = 320
# Centred on the pointer, then pulled inside the screen so the whole window is
# visible whatever corner the pointer happens to be in.
$left = [Math]::Min([Math]::Max($at.X - $side / 2, 0), $screen.Width - $side)
$top = [Math]::Min([Math]::Max($at.Y - $side / 2, 0), $screen.Height - $side)
Write-Host "pointer at ($($at.X), $($at.Y)); backdrop at ($left, $top) ${side}x${side}"

# A plain window of a colour nothing else on screen uses, so a pixel that is not
# this colour is either the pointer or something that moved in front of it.
$backdrop = New-Object System.Windows.Forms.Form
$backdrop.FormBorderStyle = 'None'
$backdrop.StartPosition = 'Manual'
$backdrop.Location = New-Object System.Drawing.Point $left, $top
$backdrop.Size = New-Object System.Drawing.Size $side, $side
$backdrop.BackColor = [System.Drawing.Color]::FromArgb(16, 16, 64)
$backdrop.TopMost = $true
$backdrop.ShowInTaskbar = $false
$backdrop.Show()
[void][Pointer]::SetForegroundWindow($backdrop.Handle)

# The application has to pump messages or the window never paints.
$deadline = (Get-Date).AddMilliseconds(700)
while ((Get-Date) -lt $deadline) {
    [System.Windows.Forms.Application]::DoEvents()
    Start-Sleep -Milliseconds 20
}

$results = @()
try {
    foreach ($backend in @('gdi', 'dxgi')) {
        foreach ($cursor in @('plain', 'with')) {
            $path = Join-Path $root "$Out-$backend-$cursor.png"
            $arguments = @('capture', '--out', $path, '--backend', $backend)
            if ($cursor -eq 'with') { $arguments += '--cursor' }

            $previous = $ErrorActionPreference
            $ErrorActionPreference = 'Continue'
            & $exe @arguments 2>&1 | Out-Null
            $ErrorActionPreference = $previous

            if (-not (Test-Path $path)) { throw "$backend/$cursor was not written" }
            $results += [pscustomobject]@{ Backend = $backend; Kind = $cursor; Path = $path }
        }
    }
} finally {
    $backdrop.Close()
    $backdrop.Dispose()
}

$python = Join-Path $env:USERPROFILE '.dsh\dsh-runtimes\dsh-primary-runtime\dependencies\python\python.exe'
if (-not (Test-Path $python)) { $python = 'python' }

$files = @{}
foreach ($result in $results) {
    $files["$($result.Backend)-$($result.Kind)"] = $result.Path
}

$script = @"
import sys
from PIL import Image
import numpy as np

BACKDROP = (16, 16, 64)
want = ($($at.X), $($at.Y))
origin = ($($screen.X), $($screen.Y))

# The backdrop's rectangle in the capture's own coordinates. Only this region is
# compared: the rest of the screen is whatever was there.
rx, ry, rw, rh = $left - $($screen.X), $top - $($screen.Y), $side, $side

def load(path):
    return np.array(Image.open(path).convert('RGB')).astype(int)

failures = 0
for backend in ('gdi', 'dxgi'):
    print(f'--- {backend} ---')
    plain = load(r'$($files["$backend-plain"])')[ry:ry + rh, rx:rx + rw]
    drawn = load(r'$($files["$backend-with"])')[ry:ry + rh, rx:rx + rw]

    if plain.shape != drawn.shape:
        print('  FAIL the two captures have different sizes')
        failures += 1
        continue

    # The backdrop has to be flat in the capture without the pointer, or
    # something moved in front of it and the comparison is meaningless.
    flat = (np.abs(plain - np.array(BACKDROP)).sum(axis=2) <= 6)
    coverage = flat.sum() / flat.size
    print(f'  backdrop still : {coverage * 100:.1f}% of the {rw}x{rh} window')
    if coverage < 0.98:
        print('  FAIL the backdrop is not still, so nothing can be told from this capture')
        failures += 1
        continue

    delta = np.abs(plain - drawn).sum(axis=2)
    changed = delta > 8
    count = int(changed.sum())
    print(f'  changed pixels : {count}')
    if count == 0:
        print('  FAIL nothing was drawn for --cursor')
        failures += 1
        continue

    ys, xs = np.where(changed)
    left = int(xs.min()) + rx + origin[0]
    top = int(ys.min()) + ry + origin[1]
    right = int(xs.max()) + rx + origin[0]
    bottom = int(ys.max()) + ry + origin[1]
    print(f'  changed area   : x {left}..{right} ({right - left + 1})  y {top}..{bottom} ({bottom - top + 1})')
    print(f'  pointer at     : {want}')

    # A hotspot error moves the shape by up to the hotspot size, so the cluster is
    # allowed to be the shape's size but must cover the pointer's own position.
    if not (left <= want[0] <= right and top <= want[1] <= bottom):
        print('  FAIL the drawn pointer does not cover the position the pointer is at')
        failures += 1
    if right - left + 1 > 64 or bottom - top + 1 > 64:
        print('  FAIL the changed area is too large to be a pointer')
        failures += 1

    # A pointer is drawn, not painted over: some pixels come out partly
    # transparent over the backdrop rather than replaced outright.
    blended = int((changed & (delta < 700)).sum())
    solid = int((changed & (delta >= 700)).sum())
    print(f'  blended pixels : {blended}   fully replaced: {solid}')
    if blended == 0:
        print('  FAIL every changed pixel was fully replaced, so nothing was alpha blended')
        failures += 1

if failures:
    print(f'{failures} check(s) failed')
    sys.exit(1)
print('cursor check ok')
"@

& $python -c $script
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Write-Host "captures under $Out-<backend>-<plain|with>.png"
