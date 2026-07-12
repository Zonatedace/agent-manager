# Start Agent Manager if it is not listening.
# Prefers the Windows Service when installed; otherwise starts a detached process.
$ErrorActionPreference = "SilentlyContinue"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$exe = Join-Path $here "target\release\agent-manager.exe"
$log = Join-Path $here "agent-manager.log"
$ServiceName = "AgentManager"

function Test-Up {
    try {
        $r = Invoke-WebRequest -Uri "http://127.0.0.1:7878/api/health" -UseBasicParsing -TimeoutSec 2
        return ($r.StatusCode -eq 200)
    } catch {
        return $false
    }
}

if (Test-Up) {
    Write-Host "already up: http://127.0.0.1:7878/"
    exit 0
}

$svc = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
if ($svc) {
    Write-Host "Starting Windows Service $ServiceName ..."
    try {
        Start-Service -Name $ServiceName -ErrorAction Stop
    } catch {
        Write-Host "WARN: could not start service (need admin?): $($_.Exception.Message)"
    }
    for ($i = 0; $i -lt 40; $i++) {
        Start-Sleep -Milliseconds 400
        if (Test-Up) {
            Write-Host "UP http://127.0.0.1:7878/  (service)"
            exit 0
        }
    }
    Write-Host "service present but health not ready - see $log"
    exit 1
}

if (-not (Test-Path $exe)) {
    Write-Host "ERROR: missing $exe - run: cargo build --release  or  .\deploy-service.ps1"
    exit 1
}

$cmd = '"' + $exe + '" --mode server --port 7878 --no-open --log-file "' + $log + '" --log-level info'
$res = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{
    CommandLine      = $cmd
    CurrentDirectory = $here
} -ErrorAction SilentlyContinue

if ($null -eq $res -or $res.ReturnValue -ne 0) {
    Start-Process -FilePath $exe -ArgumentList @(
        "--mode", "server", "--port", "7878", "--no-open", "--log-file", $log, "--log-level", "info"
    ) -WorkingDirectory $here -WindowStyle Hidden
}

for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 400
    if (Test-Up) {
        Write-Host "UP http://127.0.0.1:7878/  (detached process)"
        exit 0
    }
}

Write-Host "started but health not ready - see $log"
exit 1
