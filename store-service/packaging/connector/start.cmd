@echo off
cd /d "%~dp0"
if not exist connector.json (
  echo Download the node installation config from the center as connector.json first.
  pause
  exit /b 1
)
if not defined SSL_CERT_FILE if exist testflying-prod-ca.crt set "SSL_CERT_FILE=%~dp0testflying-prod-ca.crt"
store-connector.exe connector.json
if errorlevel 1 pause
