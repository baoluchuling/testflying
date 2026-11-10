@echo off
if /i not "%~1"=="--background" cd /d "%~dp0"
if not exist connector.json (
  echo Download the node installation config from the center as connector.json first.
  if /i not "%~1"=="--background" pause
  exit /b 1
)
if not defined SSL_CERT_FILE if exist "%~dp0testflying-prod-ca.crt" set "SSL_CERT_FILE=%~dp0testflying-prod-ca.crt"
"%~dp0store-connector.exe" connector.json
set "result=%errorlevel%"
if /i not "%~1"=="--background" if not "%result%"=="0" pause
exit /b %result%
