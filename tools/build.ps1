<#
.SYNOPSIS
    Builds lasso so the binary actually works as a tray application.

.DESCRIPTION
    The directory this project lives in carries a "Low" mandatory integrity
    label, and Windows applies the label of a file to the process started from
    it. A low-integrity process cannot send messages to a higher-integrity
    window, and the notification area runs at Medium, so registering an icon
    fails with ERROR_ACCESS_DENIED (5) and no icon appears - no matter who
    launches it, from where, or how. It is a property of the file, not of the
    program.

    Building into a directory outside that tree avoids the label completely:
    the binary has none, the process runs at Medium, and the icon registers.
    The output lands under %LOCALAPPDATA%\lasso-target.

.PARAMETER Run
    Start the resident mode once the build finishes.

.PARAMETER Release
    Build with optimisations.

.EXAMPLE
    .\tools\build.ps1 -Run
#>
[CmdletBinding()]
param(
    [switch]$Run,
    [switch]$Release
)

$ErrorActionPreference = 'Stop'

# Outside the labelled tree, which is the whole point of this script.
$env:CARGO_TARGET_DIR = Join-Path $env:LOCALAPPDATA 'lasso-target'

$cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
if (-not (Test-Path $cargo)) {
    $cargo = 'cargo'
}

$arguments = @('build')
if ($Release) {
    $arguments += '--release'
}

& $cargo @arguments
if ($LASTEXITCODE -ne 0) {
    throw "cargo build failed with exit code $LASTEXITCODE"
}

$flavour = if ($Release) { 'release' } else { 'debug' }
$exe = Join-Path $env:CARGO_TARGET_DIR "$flavour\lasso.exe"
Write-Host "built: $exe"

# Worth checking rather than assuming: a labelled binary still runs, it just
# never gets a notification-area icon.
$label = (& icacls $exe) -match 'Mandatory Label'
if ($label) {
    Write-Warning "the binary carries an integrity label, so the tray icon will be refused: $label"
} else {
    Write-Host 'no integrity label: the tray icon will register'
}

if ($Run) {
    Start-Process -FilePath $exe
}
