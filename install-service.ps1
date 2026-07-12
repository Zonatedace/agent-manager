# Install / update / uninstall the Agent Manager Windows Service.
#
# Requires Administrator.
#
#   .\install-service.ps1              # install or reconfigure + start
#   .\install-service.ps1 -Uninstall
#   .\install-service.ps1 -NoStart     # install only
#   .\install-service.ps1 -BindHost 0.0.0.0 -Port 7878
#
# The service binary is target\release\agent-manager.exe --mode service.
# Config, .env, and logs live in this repo directory (working directory).
# USERPROFILE of the installing user is injected so agent CLIs/tokens resolve.

[CmdletBinding()]
param(
    [switch]$Uninstall,
    [switch]$NoStart,
    [switch]$Build,
    [string]$BindHost = "127.0.0.1",
    [int]$Port = 7878,
    [string]$ServiceName = "AgentManager"
)

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $here
$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path

function Test-Admin {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    $p = New-Object Security.Principal.WindowsPrincipal($id)
    return $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

if (-not (Test-Admin)) {
    Write-Host "ERROR: install-service.ps1 must run as Administrator."
    Write-Host "  Right-click PowerShell → Run as administrator, then re-run."
    exit 1
}

$exe = Join-Path $here "target\release\agent-manager.exe"
$config = Join-Path $here "agent-manager.config.json"
$log = Join-Path $here "agent-manager.log"
$displayName = "Agent Manager"
$description = "Agent Manager HTTP API — multi-repo TODOs, agents, and usage meters"

function Get-ServiceObj {
    Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
}

function Stop-AgentService {
    $svc = Get-ServiceObj
    if (-not $svc) { return }
    if ($svc.Status -ne "Stopped") {
        Write-Host "Stopping service $ServiceName ..."
        Stop-Service -Name $ServiceName -Force -ErrorAction SilentlyContinue
        $svc.WaitForStatus("Stopped", "00:00:30")
    }
    # Also kill any non-service stray processes holding the port
    Get-Process -Name "agent-manager" -ErrorAction SilentlyContinue |
        Where-Object { $_.Id -ne 0 } |
        ForEach-Object {
            try { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue } catch {}
        }
    Start-Sleep -Milliseconds 500
}

function Remove-AgentService {
    Stop-AgentService
    $svc = Get-ServiceObj
    if ($svc) {
        Write-Host "Deleting service $ServiceName ..."
        sc.exe delete $ServiceName | Out-Null
        Start-Sleep -Seconds 1
    }
}

function Set-ServiceEnvironment {
    # Inject the installing user's profile so usage meters / CLIs find credentials.
    # Also put cargo/user bin on PATH for grok/claude/codex discovery.
    $userProfile = $env:USERPROFILE
    $home = if ($env:HOME) { $env:HOME } else { $userProfile }
    $pathExtra = @(
        (Join-Path $userProfile ".cargo\bin"),
        (Join-Path $userProfile "AppData\Local\Programs\Python\Python312"),
        (Join-Path $userProfile "AppData\Local\Microsoft\WinGet\Links")
    ) -join ";"

    $envMulti = @(
        "USERPROFILE=$userProfile",
        "HOME=$home",
        "APPDATA=$env:APPDATA",
        "LOCALAPPDATA=$env:LOCALAPPDATA",
        "AGENT_MANAGER_ROOT=$(if ($env:AGENT_MANAGER_ROOT) { $env:AGENT_MANAGER_ROOT } else { '' })"
    )

    $regPath = "HKLM:\SYSTEM\CurrentControlSet\Services\$ServiceName"
    if (Test-Path $regPath) {
        # Merge PATH: service gets system path + user cargo bin
        $sysPath = [Environment]::GetEnvironmentVariable("Path", "Machine")
        $envMulti += "Path=$pathExtra;$sysPath"
        Set-ItemProperty -Path $regPath -Name "Environment" -Value $envMulti -Type MultiString
        Write-Host "Service environment: USERPROFILE=$userProfile"
    }
}

function Install-AgentService {
    if ($Build -or -not (Test-Path $exe)) {
        Write-Host "Building release..."
        cargo build --release
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
    if (-not (Test-Path $exe)) {
        Write-Host "ERROR: missing $exe"
        exit 1
    }

    # Absolute paths in ImagePath so SCM/System32 cwd is irrelevant
    $binPath = @(
        "`"$exe`""
        "--mode", "service"
        "--no-open"
        "--host", $BindHost
        "--port", "$Port"
        "--config", "`"$config`""
        "--log-file", "`"$log`""
        "--log-level", "info"
    ) -join " "

    $existing = Get-ServiceObj
    if ($existing) {
        Write-Host "Updating existing service $ServiceName ..."
        Stop-AgentService
        sc.exe config $ServiceName binPath= $binPath start= auto | Out-Null
        sc.exe description $ServiceName $description | Out-Null
    } else {
        Write-Host "Creating service $ServiceName ..."
        New-Service `
            -Name $ServiceName `
            -BinaryPathName $binPath `
            -DisplayName $displayName `
            -Description $description `
            -StartupType Automatic | Out-Null
    }

    # Delayed auto-start so user profile / network are more likely ready
    sc.exe config $ServiceName start= delayed-auto | Out-Null
    Set-ServiceEnvironment

    # Recovery: restart on failure
    sc.exe failure $ServiceName reset= 86400 actions= restart/5000/restart/10000/restart/30000 | Out-Null

    Write-Host "Service binary: $exe"
    Write-Host "ImagePath:      $binPath"
    Write-Host "Config:         $config"
    Write-Host "Log:            $log"
    Write-Host "Bind:           http://${BindHost}:${Port}/"

    if (-not $NoStart) {
        Write-Host "Starting service..."
        Start-Service -Name $ServiceName
        $svc = Get-ServiceObj
        $svc.WaitForStatus("Running", "00:01:00")
        Write-Host "Service status: $($svc.Status)"

        $ok = $false
        for ($i = 0; $i -lt 40; $i++) {
            Start-Sleep -Milliseconds 400
            try {
                $h = Invoke-RestMethod "http://127.0.0.1:$Port/api/health" -TimeoutSec 2
                if ($h.ok) {
                    Write-Host "HEALTH OK  projects=$($h.projects) open=$($h.open)"
                    $ok = $true
                    break
                }
            } catch {}
        }
        if (-not $ok) {
            Write-Host "WARN: service started but /api/health not ready — check $log"
            exit 1
        }
    }
}

if ($Uninstall) {
    Remove-AgentService
    Write-Host "Uninstalled $ServiceName"
    exit 0
}

Install-AgentService
Write-Host ""
Write-Host "Done. Manage with:"
Write-Host "  Get-Service $ServiceName"
Write-Host "  Restart-Service $ServiceName"
Write-Host "  .\deploy-service.ps1     # build + redeploy"
Write-Host "  .\install-service.ps1 -Uninstall"
