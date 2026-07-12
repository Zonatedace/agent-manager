# Install Agent Manager as Windows app shortcuts (Start Menu + Desktop).
# Does not use Task Scheduler. Builds release if needed.
#
#   .\install-desktop.ps1              # full app + client exe shortcuts
#   .\install-desktop.ps1 --build      # force rebuild first
#   .\install-desktop.ps1 --no-client  # skip Agent Manager Client shortcuts
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $here

$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path
$exe = Join-Path $here "target\release\agent-manager.exe"
$clientExe = Join-Path $here "target\release\agent-manager-client.exe"

if (-not (Test-Path $exe) -or -not (Test-Path $clientExe) -or $args -contains "--build") {
    Write-Host "Building release (agent-manager + agent-manager-client)..."
    cargo build --release
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

if (-not (Test-Path $exe)) {
    Write-Host "ERROR: missing $exe"
    exit 1
}
if (-not (Test-Path $clientExe)) {
    Write-Host "ERROR: missing $clientExe (client binary)"
    exit 1
}

$startMenu = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
$desktop = [Environment]::GetFolderPath("Desktop")

$w = New-Object -ComObject WScript.Shell

function New-Shortcut(
    [string]$path,
    [string]$target,
    [string]$arguments = "",
    [string]$description = "Agent Manager - multi-repo TODOs, agents, usage meters"
) {
    $sc = $w.CreateShortcut($path)
    $sc.TargetPath = $target
    $sc.Arguments = $arguments
    $sc.WorkingDirectory = $here
    $sc.WindowStyle = 1
    $sc.Description = $description
    $ico = Join-Path $here "assets\icon.ico"
    if (Test-Path $ico) { $sc.IconLocation = $ico }
    else { $sc.IconLocation = "$target,0" }
    $sc.Save()
    Write-Host "Created $path"
}

# Full app (local server + window)
New-Shortcut (Join-Path $startMenu "Agent Manager.lnk") $exe
New-Shortcut (Join-Path $desktop "Agent Manager.lnk") $exe

# Thin Windows client (connect to Server URL — dedicated exe)
if (-not ($args -contains "--no-client")) {
    $clientDesc = "Agent Manager Client - connect to a Server URL (prompted until set)"
    New-Shortcut (Join-Path $startMenu "Agent Manager Client.lnk") $clientExe "" $clientDesc
    New-Shortcut (Join-Path $desktop "Agent Manager Client.lnk") $clientExe "" $clientDesc
}

Write-Host ""
Write-Host "Installed. Launch from Start Menu or Desktop:"
Write-Host "  Agent Manager         -> $exe"
Write-Host "  Agent Manager Client  -> $clientExe"
Write-Host ""
Write-Host "Server-only:  agent-manager.exe --mode server"
Write-Host "Client exe:   agent-manager-client.exe   (or agent-manager.exe --mode client)"
Write-Host ""
Write-Host "To run now:"
Write-Host "  Start-Process `"$exe`""
Write-Host "  Start-Process `"$clientExe`""
