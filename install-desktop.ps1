# Install Agent Manager as a Windows app shortcut (Start Menu + optional Desktop).
# Does not use Task Scheduler. Builds release if needed.
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $here

$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path
$exe = Join-Path $here "target\release\agent-manager.exe"

if (-not (Test-Path $exe) -or $args -contains "--build") {
    Write-Host "Building release..."
    cargo build --release
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

if (-not (Test-Path $exe)) {
    Write-Host "ERROR: missing $exe"
    exit 1
}

$startMenu = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
$linkPath = Join-Path $startMenu "Agent Manager.lnk"
$desktop = [Environment]::GetFolderPath("Desktop")
$deskLink = Join-Path $desktop "Agent Manager.lnk"

$w = New-Object -ComObject WScript.Shell

function New-Shortcut([string]$path) {
    $sc = $w.CreateShortcut($path)
    $sc.TargetPath = $exe
    $sc.WorkingDirectory = $here
    $sc.WindowStyle = 1
    $sc.Description = "Agent Manager - multi-repo TODOs, agents, usage meters"
    $ico = Join-Path $here "assets\icon.ico"
    if (Test-Path $ico) { $sc.IconLocation = $ico }
    else { $sc.IconLocation = "$exe,0" }
    $sc.Save()
    Write-Host "Created $path"
}

New-Shortcut $linkPath
if ($args -contains "--desktop" -or $true) {
    New-Shortcut $deskLink
}

Write-Host ""
Write-Host "Installed. Launch from Start Menu or Desktop: Agent Manager"
Write-Host "  exe: $exe"
Write-Host "  mode: native window (WebView2). Server-only: agent-manager.exe --mode server"
Write-Host ""
Write-Host "To run now:"
Write-Host "  Start-Process `"$exe`""
