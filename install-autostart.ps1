# Prefer the Windows Service for always-on Agent Manager.
# Task Scheduler is not used.
#
#   .\deploy-service.ps1          # build + install/update service (admin)
#   .\install-service.ps1         # same without forcing rebuild unless needed
#   .\install-service.ps1 -Uninstall
#
Write-Host "Agent Manager uses a Windows Service (not Task Scheduler)."
Write-Host ""
Write-Host "  Elevated PowerShell:"
Write-Host "    .\deploy-service.ps1"
Write-Host ""
Write-Host "  Then connect with the desktop client:"
Write-Host "    .\run.ps1 --client --server-url http://127.0.0.1:7878/"
Write-Host ""
exit 0
