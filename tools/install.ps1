# Builds Life HUD (release), installs it for the current user, and adds Desktop + Start menu shortcuts.
# Usage:  powershell -ExecutionPolicy Bypass -File tools\install.ps1 [-NoBuild]
param([switch]$NoBuild)
$ErrorActionPreference = "Stop"

$repo = Split-Path -Parent $PSScriptRoot
$installDir = Join-Path $env:LOCALAPPDATA "Programs\LifeHUD"
$exe = Join-Path $installDir "lifehud.exe"

if (-not $NoBuild) {
    Push-Location $repo
    try { cargo build --release } finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}

# Close a running copy cleanly (it releases its reserved screen space) before replacing the binary.
if (Test-Path $exe) {
    & $exe --exit
    $deadline = (Get-Date).AddSeconds(5)
    while ((Get-Process lifehud -ErrorAction SilentlyContinue) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 200 }
}

New-Item -ItemType Directory -Force $installDir | Out-Null
Copy-Item (Join-Path $repo "target\release\lifehud.exe") $exe -Force

$shell = New-Object -ComObject WScript.Shell
$targets = @(
    (Join-Path ([Environment]::GetFolderPath("Desktop")) "Life HUD.lnk"),
    (Join-Path ([Environment]::GetFolderPath("Programs")) "Life HUD.lnk")
)
foreach ($lnkPath in $targets) {
    $lnk = $shell.CreateShortcut($lnkPath)
    $lnk.TargetPath = $exe
    $lnk.WorkingDirectory = $installDir
    $lnk.IconLocation = "$exe,0"
    $lnk.Description = "Life HUD"
    $lnk.Save()
}

Write-Output "Installed to $exe"
Write-Output "Shortcuts: $($targets -join ', ')"
