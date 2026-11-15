# Builds Life HUD and installs it for the current user, exactly like running a downloaded release:
# %LOCALAPPDATA%\Programs\LifeHUD, a Start menu shortcut, and an entry in Settings > Apps.
# Usage:  powershell -ExecutionPolicy Bypass -File tools\install.ps1 [-NoBuild]
param([switch]$NoBuild)
$ErrorActionPreference = "Stop"

$repo = Split-Path -Parent $PSScriptRoot
if (-not $NoBuild) {
    Push-Location $repo
    try { cargo build --release } finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}

# The exe installs itself (and replaces a running copy cleanly), then starts the installed copy.
& (Join-Path $repo "target\release\lifehud.exe") --install
Write-Output "Installed to $(Join-Path $env:LOCALAPPDATA 'Programs\LifeHUD\lifehud.exe')"
