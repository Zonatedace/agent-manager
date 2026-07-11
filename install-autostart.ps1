# Deprecated for this project.
# We do NOT use Windows Task Scheduler to keep the dashboard up.
#
# Local dev:
#   .\restart.ps1          # build + restart after code changes
#   .\run.ps1              # run in foreground (keep window open)
#   .\ensure-running.ps1   # start only if down
#
# Production: container / k8s Deployment with restartPolicy.
Write-Host "Task Scheduler autostart is disabled for this project."
Write-Host "Use .\restart.ps1 after code changes, or deploy as a container/k8s pod."
exit 0
