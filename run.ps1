<#
.SYNOPSIS
Builds Trayulator and runs it with package identity through the winapp CLI.

.DESCRIPTION
`winapp run` copies the whole folder it's given into a loose-layout package, so this stages just
the exe and Assets instead of cargo's target folder. A running dev instance is stopped first so
its exe can be replaced.

.EXAMPLE
.\run.ps1            # debug build, register, launch
.\run.ps1 -Clean     # same, starting from first-run state
.\run.ps1 -Release   # release build
#>
param(
    # Build and run the release profile.
    [switch] $Release,
    # Remove the package's settings first, to see first-run behavior.
    [switch] $Clean
)

$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

$buildProfile = if ($Release) { 'release' } else { 'debug' }
cargo build --profile $(if ($Release) { 'release' } else { 'dev' })
if ($LASTEXITCODE) { exit $LASTEXITCODE }

$layout = Join-Path $PSScriptRoot "target\$buildProfile\layout"
Get-Process trayulator -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -and $_.Path.StartsWith($layout, [StringComparison]::OrdinalIgnoreCase) } |
    Stop-Process -Force

New-Item -ItemType Directory -Force $layout | Out-Null
Copy-Item "target\$buildProfile\trayulator.exe" $layout -Force
Copy-Item Assets $layout -Recurse -Force

$arguments = @('run', $layout, '--manifest', 'Package.appxmanifest', '--detach')
if ($Clean) { $arguments += '--clean' }
winapp @arguments
exit $LASTEXITCODE
