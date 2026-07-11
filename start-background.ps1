# Start TODO Dashboard minimized in the background (survives closing this shell).
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$exe = Join-Path $here "target\release\todo-dashboard.exe"
$log = Join-Path $here "todo-dashboard.log"
$pidFile = Join-Path $here "todo-dashboard.pid"

if (-not (Test-Path $exe)) {
    Write-Error "Missing $exe — run: cargo build --release"
    exit 1
}

Get-Process -Name "todo-dashboard" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 400

# Launch as a separate process with its own console (minimized)
$p = Start-Process `
    -FilePath $exe `
    -ArgumentList @(
        "--port", "7878",
        "--log-file", $log,
        "--log-level", "info"
    ) `
    -WorkingDirectory $here `
    -WindowStyle Minimized `
    -PassThru

$p.Id | Set-Content -Path $pidFile -Encoding ascii

# Wait until health responds (scan is fast, but give it a few seconds)
$ok = $false
for ($i = 0; $i -lt 30; $i++) {
    Start-Sleep -Milliseconds 500
    if ($p.HasExited) {
        Write-Host "ERROR: process exited immediately (code $($p.ExitCode)). See $log"
        exit 1
    }
    try {
        $h = Invoke-RestMethod "http://127.0.0.1:7878/api/health" -TimeoutSec 1
        if ($h.ok) { $ok = $true; break }
    } catch {}
}

if (-not $ok) {
    Write-Host "Started pid $($p.Id) but health check not ready yet. Open http://127.0.0.1:7878/"
} else {
    Write-Host "UP  http://127.0.0.1:7878/  (pid $($p.Id))"
}

Start-Process "http://127.0.0.1:7878/"
