# Rebuild + restart for local dev (no Task Scheduler).
# Usage:
#   .\restart.ps1           # rebuild release, restart
#   .\restart.ps1 --no-build  # just restart existing binary
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $here

$exe = Join-Path $here "target\release\todo-dashboard.exe"
$log = Join-Path $here "todo-dashboard.log"
$doBuild = -not ($args -contains "--no-build")

Write-Host "Stopping any running todo-dashboard..."
Get-Process -Name "todo-dashboard" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 400

if ($doBuild) {
    Write-Host "cargo build --release ..."
    cargo build --release
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

if (-not (Test-Path $exe)) {
    Write-Host "ERROR: missing $exe"
    exit 1
}

# Start detached from this script so restart.ps1 can exit,
# but NOT via schtasks. Parent is WMI/Win32_Process (survives this shell).
$cmd = '"' + $exe + '" --port 7878 --no-open --log-file "' + $log + '" --log-level info'
$res = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{
    CommandLine      = $cmd
    CurrentDirectory = $here
}

if ($null -eq $res -or $res.ReturnValue -ne 0) {
    Write-Host "WMI start failed (code $($res.ReturnValue)); trying Start-Process..."
    Start-Process -FilePath $exe -ArgumentList @(
        "--port", "7878", "--no-open", "--log-file", $log, "--log-level", "info"
    ) -WorkingDirectory $here -WindowStyle Hidden
} else {
    Write-Host "Started pid $($res.ProcessId)"
}

for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 400
    try {
        $h = Invoke-RestMethod "http://127.0.0.1:7878/api/health" -TimeoutSec 2
        if ($h.ok) {
            Write-Host "UP  http://127.0.0.1:7878/  projects=$($h.projects) open=$($h.open)"
            exit 0
        }
    } catch {}
}

Write-Host "Process started but health not ready - check $log"
exit 1
