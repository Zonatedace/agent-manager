# Dev runner: rebuild (if needed) and run the dashboard in THIS window.
# Keep the window open while you use the UI. Ctrl+C to stop.
# For k8s/container later, this is just local dev convenience.
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $here

$exe = Join-Path $here "target\release\todo-dashboard.exe"
$log = Join-Path $here "todo-dashboard.log"

# Free the port if a leftover instance is still bound
Get-Process -Name "todo-dashboard" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 300

if ($args -contains "--build" -or -not (Test-Path $exe)) {
    Write-Host "Building release..."
    cargo build --release
}

if (-not (Test-Path $exe)) {
    Write-Host "ERROR: binary missing after build"
    exit 1
}

Write-Host ""
Write-Host "  TODO Dashboard (dev)"
Write-Host "  http://127.0.0.1:7878/"
Write-Host "  log: $log"
Write-Host "  Ctrl+C to stop"
Write-Host ""

& $exe --port 7878 --log-file $log --log-level info
