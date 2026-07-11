# Launch Agent Manager as a Windows app (native WebView2 window).
# For HTTP-only mode:  .\run.ps1 --server
# For thin client:     .\run.ps1 --client
#                      .\run.ps1 --client --server-url http://127.0.0.1:7878/
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $here
$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path

$exe = Join-Path $here "target\release\agent-manager.exe"
$log = Join-Path $here "agent-manager.log"

if ($args -contains "--build" -or -not (Test-Path $exe)) {
    Write-Host "Building release..."
    cargo build --release
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

if (-not (Test-Path $exe)) {
    Write-Host "ERROR: binary missing after build"
    exit 1
}

$pass = @()
foreach ($a in $args) {
    if ($a -eq "--build") { continue }
    if ($a -eq "--server") { $pass += @("--mode", "server"); continue }
    if ($a -eq "--client") { $pass += @("--mode", "client"); continue }
    $pass += $a
}

Write-Host "Starting Agent Manager..."
Write-Host "  log: $log"
if ($pass -contains "server") {
    Write-Host "  mode: server  http://127.0.0.1:7878/"
    & $exe @pass --log-file $log --log-level info
} elseif ($pass -contains "client") {
    Write-Host "  mode: client (server URL prompted until set)"
    Start-Process -FilePath $exe -ArgumentList (@("--log-file", $log, "--log-level", "info") + $pass) -WorkingDirectory $here
} else {
    Write-Host "  mode: desktop window (local server)"
    # Start detached so this shell can return; window is the app UI
    Start-Process -FilePath $exe -ArgumentList (@("--log-file", $log, "--log-level", "info") + $pass) -WorkingDirectory $here
}
