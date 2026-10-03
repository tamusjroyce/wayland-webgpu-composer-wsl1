@echo off
REM ---------------------------------------------------------------------------
REM Trust the Wayland WebGPU Composer signing certificate so the (self-signed,
REM beta) MSIX can be installed. Download this next to `wayland-webgpu-composer.cer`
REM and run it as Administrator.
REM ---------------------------------------------------------------------------
setlocal EnableExtensions

set "CER=%~dp0wayland-webgpu-composer.cer"
if not exist "%CER%" set "CER=%CD%\wayland-webgpu-composer.cer"
if not exist "%CER%" (
  echo ERROR: wayland-webgpu-composer.cer not found next to this script.
  echo Download it from the release and place it in the same folder.
  exit /b 1
)

REM Sideloaded MSIX trust uses LocalMachine\TrustedPeople, which needs elevation.
net session >nul 2>&1 || (
  echo Please run this script as Administrator ^(right-click -^> Run as administrator^).
  exit /b 1
)

certutil -addstore -f TrustedPeople "%CER%" || exit /b 1

echo.
echo Certificate trusted. You can now install wayland-webgpu-composer-windows-x64.msix.
exit /b 0
