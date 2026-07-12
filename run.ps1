# Launch Agent Manager as a Windows app (native WebView2 window).
# For HTTP-only mode:  .\run.ps1 --server
# For thin client:     .\run.ps1 --client
#                      .\run.ps1 --client --server-url http://127.0.0.1:7878/
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $here
$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path

$exe = Join-Path $here "target\release\agent-manager.exe"
$clientExe = Join-Path $here "target\release\agent-manager-client.exe"
$log = Join-Path $here "agent-manager.log"
$clientLog = Join-Path $here "agent-manager-client.log"

$wantClient = $args -contains "--client"
$needBuild = $args -contains "--build" -or -not (Test-Path $exe) -or ($wantClient -and -not (Test-Path $clientExe))

if ($needBuild) {
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
    if ($a -eq "--client") { continue } # dedicated client exe
    $pass += $a
}

Write-Host "Starting Agent Manager..."
if ($pass -contains "server") {
    Write-Host "  log: $log"
    Write-Host "  mode: server  http://127.0.0.1:7878/"
    & $exe @pass --log-file $log --log-level info
} elseif ($wantClient) {
    if (-not (Test-Path $clientExe)) {
        Write-Host "ERROR: missing client exe: $clientExe"
        exit 1
    }
    Write-Host "  log: $clientLog"
    Write-Host "  mode: client exe (Server URL prompted until set)"
    Write-Host "  exe: $clientExe"
    Start-Process -FilePath $clientExe -ArgumentList (@("--log-file", $clientLog, "--log-level", "info") + $pass) -WorkingDirectory $here
} else {
    Write-Host "  log: $log"
    Write-Host "  mode: desktop window (local server)"
    # Start detached so this shell can return; window is the app UI
    Start-Process -FilePath $exe -ArgumentList (@("--log-file", $log, "--log-level", "info") + $pass) -WorkingDirectory $here
}
