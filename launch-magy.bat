@echo off
setlocal
cd /d "%~dp0"

where cargo >nul 2>nul
if errorlevel 1 (
  echo Rust Cargo was not found on PATH.
  exit /b 1
)

echo Building Magy...
cargo build --workspace
if errorlevel 1 exit /b %errorlevel%

echo Starting Magy at http://127.0.0.1:3000
start "" http://127.0.0.1:3000
cargo run -p magy-app
