# Dump the real geometry of every control, which is what a capture shows. The
# layout's arithmetic is covered by tests; where Windows actually put each control
# is a separate question, and the two differing is the whole bug.
$env:LASSO_TRACE_LAYOUT = '1'
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root 'target\debug\lasso.exe'
$err = Join-Path $root 'target\test-output\layout-err.txt'
$out = Join-Path $root 'target\test-output\layout-out.txt'

$arguments = @('settings')
if ($args.Count -gt 0) { $arguments += $args }

$process = Start-Process -FilePath $exe -ArgumentList $arguments -PassThru `
    -RedirectStandardError $err -RedirectStandardOutput $out
Start-Sleep -Milliseconds 2500
if ($process -and -not $process.HasExited) { $process.Kill() }
Start-Sleep -Milliseconds 200

Get-Content $err
