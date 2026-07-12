# Build a new release and deploy it to the Windows Service.
#
# Requires Administrator (service stop/start + optional install).
#
#   .\deploy-service.ps1              # cargo build --release, then reinstall/start service
#   .\deploy-service.ps1 -NoBuild     # redeploy current binary only
#   .\deploy-service.ps1 -BindHost 0.0.0.0
#
# This is the intended "on build, deploy service" entrypoint.

[CmdletBinding()]
param(
    [switch]$NoBuild,
    [string]$BindHost = "127.0.0.1",
    [int]$Port = 7878,
    [string]$ServiceName = "AgentManager"
)

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $here

$install = Join-Path $here "install-service.ps1"
if (-not (Test-Path $install)) {
    Write-Host "ERROR: missing $install"
    exit 1
}

$fwd = @()
if (-not $NoBuild) { $fwd += "-Build" }
$fwd += @("-BindHost", $BindHost, "-Port", "$Port", "-ServiceName", $ServiceName)

Write-Host "=== Deploy Agent Manager Windows Service ==="
Write-Host "  build: $(-not $NoBuild)"
Write-Host "  host:  $BindHost"
Write-Host "  port:  $Port"
Write-Host ""

& $install @fwd
exit $LASTEXITCODE
