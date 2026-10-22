@echo off
rem Builds AWorld, then starts it.
rem   run.bat                the desktop app (extra arguments go to the game, e.g. --mute)
rem   run.bat web [port]     the web version: builds the package and serves it on http://localhost:8765/
setlocal
cd /d "%~dp0"

if /i "%1"=="web" (
  call build.bat web || exit /b 1
  set PORT=%2
  if "%PORT%"=="" set PORT=8765
  echo AWorld web : http://localhost:%PORT%/   ^(Ctrl+C pour arreter^)
  call dist\web\serve.bat %PORT%
  exit /b 0
)

call build.bat app || exit /b 1
target\release\aworld.exe %*
exit /b 0
