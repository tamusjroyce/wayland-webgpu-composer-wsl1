@echo off
REM ---------------------------------------------------------------------------
REM One-shot installer / updater / launcher for wayland-webgpu-composer.
REM
REM Sets up BOTH halves and launches them: downloads the Windows host, ensures a
REM WSL1 distro with the Linux compositor, then starts the compositor (with a
REM demo desktop) and the WebGPU window. Re-run any time to update and relaunch.
REM
REM Delegates to install.ps1 (next to this file, or downloaded from the latest
REM release). Any arguments are passed through (e.g. a custom client command):
REM   install.cmd "gnome-calculator"
REM ---------------------------------------------------------------------------
setlocal
set "REPO=tamusjroyce/wayland-webgpu-composer-wsl1"
set "PS1=%~dp0install.ps1"

if not exist "%PS1%" (
  set "PS1=%TEMP%\wwc-install.ps1"
  echo Downloading installer...
  curl -fL "https://github.com/%REPO%/releases/latest/download/install.ps1" -o "%PS1%" || (
    echo Failed to download installer.
    exit /b 1
  )
)

powershell -NoProfile -ExecutionPolicy Bypass -File "%PS1%" %*
exit /b %ERRORLEVEL%
