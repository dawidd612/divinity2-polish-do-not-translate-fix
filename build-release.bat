@echo off
setlocal
cd /d "%~dp0"
echo Building Divinity II PL DO NOT TRANSLATE Fix v1.3.0...
cargo build --release
if errorlevel 1 (
  echo.
  echo Build failed.
  pause
  exit /b 1
)
if not exist dist mkdir dist
copy /Y target\release\divinity2-polish-do-not-translate-fix.exe dist\Divinity2-PL-DO-NOT-TRANSLATE-Fix.exe >nul
if errorlevel 1 (
  echo.
  echo Build succeeded, but copying EXE to dist failed.
  pause
  exit /b 1
)
echo.
echo Done: dist\Divinity2-PL-DO-NOT-TRANSLATE-Fix.exe
pause
