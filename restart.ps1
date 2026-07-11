# Rebuild + restart Agent Manager (Windows app by default).
#   .\restart.ps1
#   .\restart.ps1 --no-build
#   .\restart.ps1 --server          # headless HTTP instead of window
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $here
$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path

$exe = Join-Path $here "target\release\agent-manager.exe"
$log = Join-Path $here "agent-manager.log"
$doBuild = -not ($args -contains "--no-build")
$serverMode = $args -contains "--server"

Write-Host "Stopping any running agent-manager..."
Get-Process -Name "agent-manager" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
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

$argList = @(
    "--port", "7878",
    "--log-file", $log,
    "--log-level", "info"
)
if ($serverMode) {
    $argList = @("--mode", "server", "--no-open") + $argList
}

# Detached start via WMI so this script can exit (survives shell close).
$cmd = '"' + $exe + '" ' + ($argList | ForEach-Object {
    if ($_ -match '\s') { '"' + $_ + '"' } else { $_ }
}) -join ' '

$res = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{
    CommandLine      = $cmd
    CurrentDirectory = $here
}

if ($null -eq $res -or $res.ReturnValue -ne 0) {
    Write-Host "WMI start failed (code $($res.ReturnValue)); trying Start-Process..."
    Start-Process -FilePath $exe -ArgumentList $argList -WorkingDirectory $here
} else {
    Write-Host "Started pid $($res.ProcessId)"
}

for ($i = 0; $i -lt 50; $i++) {
    Start-Sleep -Milliseconds 400
    try {
        $h = Invoke-RestMethod "http://127.0.0.1:7878/api/health" -TimeoutSec 2
        if ($h.ok) {
            if ($serverMode) {
                Write-Host "UP  http://127.0.0.1:7878/  (server mode) projects=$($h.projects) open=$($h.open)"
            } else {
                Write-Host "UP  desktop app + http://127.0.0.1:7878/  projects=$($h.projects) open=$($h.open)"
            }
            exit 0
        }
    } catch {}
}

Write-Host "Process started but health not ready - check $log"
exit 1
