@echo off
REM ---------------------------------------------------------------------------
REM Download, extract, and run the latest Windows release of
REM wayland-webgpu-composer. Any arguments are passed through to win-host.exe
REM (e.g. install.cmd --host 127.0.0.1:7777).
REM
REM This script ships inside the Windows release zip and is also published as a
REM standalone release asset, so it can be fetched and run on its own:
REM   curl -L -o install.cmd https://github.com/tamusjroyce/wayland-webgpu-composer-wsl1/releases/latest/download/install.cmd
REM   install.cmd
REM ---------------------------------------------------------------------------
setlocal
set "REPO=tamusjroyce/wayland-webgpu-composer-wsl1"
set "ASSET=wayland-webgpu-composer-windows-x64.zip"
set "URL=https://github.com/%REPO%/releases/latest/download/%ASSET%"
set "DEST=%LOCALAPPDATA%\wayland-webgpu-composer"
set "TMPZIP=%TEMP%\%ASSET%"

echo Downloading latest release:
echo   %URL%
curl -fL "%URL%" -o "%TMPZIP%"
if errorlevel 1 (
  echo Download failed.
  exit /b 1
)

echo Extracting to "%DEST%" ...
powershell -NoProfile -Command "Expand-Archive -LiteralPath '%TMPZIP%' -DestinationPath '%DEST%' -Force"
if errorlevel 1 (
  echo Extraction failed.
  exit /b 1
)
del "%TMPZIP%" >nul 2>&1

echo.
echo Installed to: %DEST%
echo Starting win-host.exe ...
"%DEST%\win-host.exe" %*
