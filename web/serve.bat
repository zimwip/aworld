@echo off
rem Serves the game on http://localhost:8765/ (or the port given as the first argument).
cd /d "%~dp0"
set PORT=%1
if "%PORT%"=="" set PORT=8765
aworld-serve.exe site %PORT%
