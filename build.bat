@echo off
rem Builds AWorld.
rem   build.bat          the desktop app          -> target\release\aworld.exe
rem   build.bat web      the web package (wasm)   -> dist\web\  (with a tiny web server)
rem   build.bat web run  same, then serves it on http://localhost:8765
setlocal
cd /d "%~dp0"

set MODE=%1
if "%MODE%"=="" set MODE=app

if /i "%MODE%"=="app" (
  cargo build --release --bin aworld || exit /b 1
  echo OK : target\release\aworld.exe
  exit /b 0
)

if /i "%MODE%"=="web" (
  rustup target add wasm32-unknown-unknown >nul 2>&1
  cargo build --release --target wasm32-unknown-unknown --bin aworld || exit /b 1
  cargo build --release --bin aworld-serve || exit /b 1
  if exist dist\web rmdir /s /q dist\web
  mkdir dist\web\site
  copy /y web\index.html dist\web\site\ >nul
  copy /y web\mq_js_bundle.js dist\web\site\ >nul
  copy /y target\wasm32-unknown-unknown\release\aworld.wasm dist\web\site\ >nul
  copy /y target\release\aworld-serve.exe dist\web\ >nul
  > dist\web\serve.bat (
    echo @echo off
    echo cd /d "%%~dp0"
    echo set PORT=%%1
    echo if "%%PORT%%"=="" set PORT=8765
    echo aworld-serve.exe site %%PORT%%
  )
  echo OK : dist\web  ^(lance dist\web\serve.bat puis ouvre http://localhost:8765/^)
  if /i "%2"=="run" call dist\web\serve.bat
  exit /b 0
)

echo usage : build.bat [app^|web [run]]
exit /b 1
