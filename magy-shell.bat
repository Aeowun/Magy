@echo off
cd /d "%~dp0"
powershell.exe -NoLogo -NoExit -ExecutionPolicy Bypass -Command "Set-Location -LiteralPath '%~dp0'; Write-Host 'Magy developer shell' -ForegroundColor Cyan; Write-Host 'Commands: .\magy-shell.ps1 -Command test | build | run | cli' -ForegroundColor DarkGray"
