# Ask the live settings window whether its controls are themed.
#
# `GetWindowTheme` returns null for a control with no theme, which is the only
# direct answer to "did the manifest take effect in this process". Everything else
# is inference from how the controls look.
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root 'target\debug\lasso.exe'
$err = Join-Path $root 'target\test-output\theme-err.txt'
$out = Join-Path $root 'target\test-output\theme-out.txt'

$process = Start-Process -FilePath $exe -ArgumentList @('settings') -PassThru `
    -RedirectStandardError $err -RedirectStandardOutput $out
Start-Sleep -Seconds 2

$python = 'C:\Users\spacejockey\.dsh\dsh-runtimes\dsh-primary-runtime\dependencies\python\python.exe'
& $python (Join-Path $PSScriptRoot 'theme-probe.py')

if ($process -and -not $process.HasExited) { $process.Kill() }
Start-Sleep -Milliseconds 200
Get-Content $err -ErrorAction SilentlyContinue
