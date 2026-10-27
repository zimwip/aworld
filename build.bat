@echo off
rem Builds AWorld.
rem   build.bat          the desktop app          -> target\release\aworld.exe
rem   build.bat web      the web package (wasm)   -> dist\web\  (with a tiny web server)
rem To build and start it at once, use run.bat. To put a shortcut on the desktop, install.bat.
setlocal
cd /d "%~dp0"

set MODE=%1
if "%MODE%"=="" set MODE=app

where cargo >nul 2>&1
if errorlevel 1 (
  echo Il manque "cargo" : installe Rust ^(https://rustup.rs^) puis relance.
  exit /b 1
)

if /i "%MODE%"=="app" goto app
if /i "%MODE%"=="web" goto web
echo usage : build.bat [app^|web]
exit /b 1

:app
cargo build --release --bin aworld || exit /b 1
echo OK : target\release\aworld.exe
exit /b 0

:web
where rustup >nul 2>&1
if errorlevel 1 (
  echo Il manque "rustup" : la version web a besoin de la cible wasm32-unknown-unknown de Rust ^(https://rustup.rs^).
  exit /b 1
)
rustup target list --installed | findstr /x "wasm32-unknown-unknown" >nul 2>&1
if errorlevel 1 (
  echo Installation de la cible wasm32-unknown-unknown...
  rustup target add wasm32-unknown-unknown || exit /b 1
)
rem The glue between the page and the game: the same version as the wasm-bindgen of Cargo.toml.
rem The line reads: wasm-bindgen = "=0.2.x" (its third piece between the "=" is 0.2.x").
for /f "tokens=3 delims==" %%v in ('findstr /b /c:"wasm-bindgen = " Cargo.toml') do set WBG=%%v
set WBG=%WBG:"=%
set HAVE=
for /f "tokens=2" %%v in ('wasm-bindgen --version 2^>nul') do set HAVE=%%v
if not "%HAVE%"=="%WBG%" (
  echo Installation de wasm-bindgen %WBG% ^(une seule fois^)...
  cargo install wasm-bindgen-cli --version %WBG% --locked || exit /b 1
)
cargo build --release --target wasm32-unknown-unknown --bin aworld || exit /b 1
cargo build --release --bin aworld-serve || exit /b 1
if exist dist\web rmdir /s /q dist\web
mkdir dist\web\site || exit /b 1
copy /y web\index.html dist\web\site\ >nul || exit /b 1
wasm-bindgen --target web --no-typescript --out-dir dist\web\site target\wasm32-unknown-unknown\release\aworld.wasm || exit /b 1
copy /y target\release\aworld-serve.exe dist\web\ >nul || exit /b 1
copy /y web\serve.bat dist\web\ >nul || exit /b 1
echo OK : dist\web
echo Pour le lancer : run.bat web   ^(ou dist\web\serve.bat puis http://localhost:8765/^)
exit /b 0
