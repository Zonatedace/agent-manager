# Start Agent Manager and keep it running.
# Prefer: double-click this file, or: powershell -File start.ps1
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $here

$exe = Join-Path $here "target\release\agent-manager.exe"
$log = Join-Path $here "agent-manager.log"
$pidFile = Join-Path $here "agent-manager.pid"

if (-not (Test-Path $exe)) {
    Write-Host "Building release binary (first time)..."
    cargo build --release
    if (-not (Test-Path $exe)) {
        Write-Host "Build failed — is Rust installed? (cargo)"
        pause
        exit 1
    }
}

# Stop previous instance if we know the pid / by name
if (Test-Path $pidFile) {
    $old = Get-Content $pidFile -ErrorAction SilentlyContinue
    if ($old) {
        Stop-Process -Id $old -Force -ErrorAction SilentlyContinue
    }
}
Get-Process -Name "agent-manager" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 400

Write-Host ""
Write-Host "  Agent Manager"
Write-Host "  --------------"
Write-Host "  URL:    http://127.0.0.1:7878/"
Write-Host "  Log:    $log"
Write-Host "  Config: $(Join-Path $here 'agent-manager.config.json')"
Write-Host ""
Write-Host "  Leave this window open while using the dashboard."
Write-Host "  Press Ctrl+C to stop the server."
Write-Host ""

# Run in THIS window so the process cannot silently vanish
& $exe --port 7878 --log-file $log --log-level info
$exit = $LASTEXITCODE
Write-Host ""
Write-Host "Server exited with code $exit"
if (Test-Path $pidFile) { Remove-Item $pidFile -Force -ErrorAction SilentlyContinue }
pause
