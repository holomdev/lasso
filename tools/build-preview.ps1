# Rebuild, then open the settings window and report whether its controls are themed.
#
# This is the check for "the window does not look modern". A control's theme is not
# visible in the source and not decided by the window: it comes from the manifest
# attached to the executable, and `GetWindowTheme` is the only direct answer.
#
# Run it after a build. If the executable was running while `cargo build` ran, the
# linker could not replace it and what opens is the previous one, which is the
# usual reason a change appears not to have taken.
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (-not (Test-Path $cargo)) { $cargo = 'cargo' }

    Write-Host 'building...'
    & $cargo build 2>&1 | Where-Object { $_ -match 'error|warning|Finished' } | ForEach-Object { Write-Host $_ }

    $exe = Join-Path $root 'target\debug\lasso.exe'
    $item = Get-Item $exe
    Write-Host "executable: $($item.FullName)"
    Write-Host "  built: $($item.LastWriteTime)  size: $($item.Length)"

    $err = Join-Path $root 'target\test-output\theme-err.txt'
    $out = Join-Path $root 'target\test-output\theme-out.txt'
    $process = Start-Process -FilePath $exe -ArgumentList @('settings') -PassThru `
        -RedirectStandardError $err -RedirectStandardOutput $out
    Start-Sleep -Seconds 2

    $python = 'C:\Users\spacejockey\.dsh\dsh-runtimes\dsh-primary-runtime\dependencies\python\python.exe'
    if (Test-Path $python) {
        & $python (Join-Path $PSScriptRoot 'theme-probe.py')
    } else {
        Write-Host 'no python found, so the live controls could not be asked'
    }

    if ($process -and -not $process.HasExited) { $process.Kill() }
    Start-Sleep -Milliseconds 200
    Get-Content $err -ErrorAction SilentlyContinue | Write-Host
} finally {
    Pop-Location
}
