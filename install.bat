@echo off
rem Installs AWorld for the current user and puts a shortcut on the desktop and in the Start menu.
rem   install.bat              build, install, create the shortcuts
rem   install.bat uninstall    remove them again
setlocal
cd /d "%~dp0"
set HOME_DIR=%LOCALAPPDATA%\AWorld

if /i "%1"=="uninstall" (
  powershell -NoProfile -ExecutionPolicy Bypass -Command "$d=[Environment]::GetFolderPath('Desktop'); $m=Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'; Remove-Item -ErrorAction SilentlyContinue (Join-Path $d 'AWorld.lnk'), (Join-Path $m 'AWorld.lnk')"
  if exist "%HOME_DIR%\aworld.exe" del /q "%HOME_DIR%\aworld.exe"
  if exist "%HOME_DIR%\icon.ico" del /q "%HOME_DIR%\icon.ico"
  echo Raccourcis et programme retires ^(les mondes sauvegardes restent dans %HOME_DIR%\saves^).
  exit /b 0
)

call build.bat app || exit /b 1
if not exist "%HOME_DIR%" mkdir "%HOME_DIR%"
copy /y target\release\aworld.exe "%HOME_DIR%\aworld.exe" >nul || exit /b 1
copy /y assets\icon.ico "%HOME_DIR%\icon.ico" >nul || exit /b 1

powershell -NoProfile -ExecutionPolicy Bypass -Command ^
  "$exe=Join-Path $env:LOCALAPPDATA 'AWorld\aworld.exe'; $ico=Join-Path $env:LOCALAPPDATA 'AWorld\icon.ico'; $dir=Join-Path $env:LOCALAPPDATA 'AWorld';" ^
  "$sh=New-Object -ComObject WScript.Shell;" ^
  "foreach ($folder in @([Environment]::GetFolderPath('Desktop'), (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'))) {" ^
  "  $l=$sh.CreateShortcut((Join-Path $folder 'AWorld.lnk')); $l.TargetPath=$exe; $l.WorkingDirectory=$dir; $l.IconLocation=$ico; $l.Description='AWorld : bac a sable educatif'; $l.Save() }"
if errorlevel 1 (
  echo Echec de la creation du raccourci.
  exit /b 1
)
echo OK : AWorld installe dans %HOME_DIR%
echo Raccourcis : bureau et menu Demarrer.
exit /b 0
