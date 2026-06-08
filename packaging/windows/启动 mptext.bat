@echo off
chcp 65001 >nul
title mptext-cli
cd /d "%~dp0"

if "%~1"=="" (
  echo.
  mptext.exe
  echo.
  pause
  exit /b 0
)

mptext.exe %*
set ERR=%ERRORLEVEL%
if %ERR% NEQ 0 (
  echo.
  echo [exit code %ERR%]
  pause
)
exit /b %ERR%
