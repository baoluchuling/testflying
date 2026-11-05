@echo off
setlocal
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1"
set "result=%errorlevel%"
if not "%result%"=="0" echo Connector installation failed.
pause
exit /b %result%
