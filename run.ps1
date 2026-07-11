# Launch TODO Dashboard as a Windows app (native WebView2 window).
# For HTTP-only mode: .\run.ps1 --server
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $here
$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path

$exe = Join-Path $here "target\release\todo-dashboard.exe"
$log = Join-Path $here "todo-dashboard.log"

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
    $pass += $a
}

Write-Host "Starting TODO Dashboard..."
Write-Host "  log: $log"
if ($pass -contains "server") {
    Write-Host "  mode: server  http://127.0.0.1:7878/"
    & $exe @pass --log-file $log --log-level info
} else {
    Write-Host "  mode: desktop window"
    # Start detached so this shell can return; window is the app UI
    Start-Process -FilePath $exe -ArgumentList (@("--log-file", $log, "--log-level", "info") + $pass) -WorkingDirectory $here
}
