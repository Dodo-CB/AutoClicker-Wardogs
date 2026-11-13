@echo off
cd /d "%~dp0"
where cargo >nul 2>nul
if errorlevel 1 (
  echo Rust ist nicht installiert. Bitte zuerst https://rustup.rs installieren.
  pause
  exit /b 1
)
cargo build --release
if errorlevel 1 (
  echo.
  echo Build fehlgeschlagen - Fehlermeldung oben ansehen.
  pause
  exit /b 1
)
if not exist dist mkdir dist
copy /y target\release\wardogs-autoklicker.exe dist\WardogsAutoklicker.exe >nul
echo.
echo Fertig! Deine EXE liegt in: dist\WardogsAutoklicker.exe
pause
