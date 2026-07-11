# Install Agent Manager as a Windows app shortcut (Start Menu + optional Desktop).
# Does not use Task Scheduler. Builds release if needed.
#
#   .\install-desktop.ps1              # full app (local server + window)
#   .\install-desktop.ps1 --client     # also add "Agent Manager Client" shortcut
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

function New-Shortcut([string]$path, [string]$arguments = "", [string]$description = "Agent Manager - multi-repo TODOs, agents, usage meters") {
    $sc = $w.CreateShortcut($path)
    $sc.TargetPath = $exe
    $sc.Arguments = $arguments
    $sc.WorkingDirectory = $here
    $sc.WindowStyle = 1
    $sc.Description = $description
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

if ($args -contains "--client") {
    $clientDesc = "Agent Manager Client - connect to a Server URL (prompted until set)"
    New-Shortcut (Join-Path $startMenu "Agent Manager Client.lnk") "--mode client" $clientDesc
    New-Shortcut (Join-Path $desktop "Agent Manager Client.lnk") "--mode client" $clientDesc
}

Write-Host ""
Write-Host "Installed. Launch from Start Menu or Desktop: Agent Manager"
Write-Host "  exe: $exe"
Write-Host "  mode: native window (WebView2). Server-only: agent-manager.exe --mode server"
Write-Host "  thin client: agent-manager.exe --mode client  (or .\install-desktop.ps1 --client)"
Write-Host ""
Write-Host "To run now:"
Write-Host "  Start-Process `"$exe`""
