# Local dev helper only: start the dashboard if it is not listening.
# No Task Scheduler. For k8s/container, use the image entrypoint instead.
$ErrorActionPreference = "SilentlyContinue"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$exe = Join-Path $here "target\release\todo-dashboard.exe"
$log = Join-Path $here "todo-dashboard.log"

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

if (-not (Test-Path $exe)) {
    Write-Host "ERROR: missing $exe - run: cargo build --release"
    exit 1
}

$cmd = '"' + $exe + '" --port 7878 --no-open --log-file "' + $log + '" --log-level info'
$res = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{
    CommandLine      = $cmd
    CurrentDirectory = $here
} -ErrorAction SilentlyContinue

if ($null -eq $res -or $res.ReturnValue -ne 0) {
    Start-Process -FilePath $exe -ArgumentList @(
        "--port", "7878", "--no-open", "--log-file", $log, "--log-level", "info"
    ) -WorkingDirectory $here -WindowStyle Hidden
}

for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 400
    if (Test-Up) {
        Write-Host "UP http://127.0.0.1:7878/"
        exit 0
    }
}

Write-Host "started but health not ready - see $log"
exit 1
