@echo off
REM ---------------------------------------------------------------------------
REM One-shot installer / updater / launcher for wayland-webgpu-composer.
REM Pure batch, no PowerShell.
REM
REM Installs BOTH halves, then hands off to run.cmd to launch them:
REM   1. Downloads/updates the Windows host (win-host.exe, fb-dump.exe, run.cmd).
REM   2. Ensures a WSL1 distro (imports Ubuntu 22.04 on first run).
REM   3. Installs/updates the Linux compositor + a demo desktop inside it.
REM   -> run.cmd launches the compositor and the WebGPU window.
REM
REM Re-run any time to update to the latest release and relaunch. Pass a client
REM command to change what runs inside the compositor:
REM   install.cmd "gnome-calculator"
REM
REM Requires Windows 10 1803+ (bundled curl.exe and tar.exe) and WSL.
REM ---------------------------------------------------------------------------
setlocal EnableExtensions

set "REPO=tamusjroyce/wayland-webgpu-composer-wsl1"
set "WIN_ASSET=wayland-webgpu-composer-windows-x64.zip"
set "WSL_ASSET=wayland-webgpu-composer-wsl1-x64.tar.gz"
set "WIN_URL=https://github.com/%REPO%/releases/latest/download/%WIN_ASSET%"
set "WSL_URL=https://github.com/%REPO%/releases/latest/download/%WSL_ASSET%"
set "ROOTFS_URL=https://cloud-images.ubuntu.com/wsl/jammy/current/ubuntu-jammy-wsl-amd64-ubuntu22.04lts.rootfs.tar.gz"
set "DISTRO=WWC-WSL1"
set "DEST=%LOCALAPPDATA%\wayland-webgpu-composer"
set "DISTRO_DIR=%LOCALAPPDATA%\WSL\%DISTRO%"
REM Make `wsl -l -q` emit UTF-8 so findstr can read it.
set "WSL_UTF8=1"

echo === wayland-webgpu-composer installer ===

REM [1/3] Windows binaries -----------------------------------------------------
echo [1/3] Downloading Windows host...
where curl >nul 2>&1 || (echo ERROR: curl.exe not found ^(needs Windows 10 1803+^).& goto :fail)
where tar  >nul 2>&1 || (echo ERROR: tar.exe not found ^(needs Windows 10 1803+^).& goto :fail)
if not exist "%DEST%" mkdir "%DEST%"
REM Stop a running host so its .exe can be overwritten on update.
taskkill /im win-host.exe /f >nul 2>&1
taskkill /im fb-dump.exe /f >nul 2>&1
curl -fL "%WIN_URL%" -o "%TEMP%\%WIN_ASSET%" || goto :fail
tar -xf "%TEMP%\%WIN_ASSET%" -C "%DEST%" || goto :fail
del "%TEMP%\%WIN_ASSET%" >nul 2>&1
echo       installed to %DEST%

REM [2/3] WSL distro -----------------------------------------------------------
where wsl >nul 2>&1 || (echo ERROR: WSL is not installed. Run: wsl --install& goto :fail)
wsl -l -q | findstr /i /c:"%DISTRO%" >nul 2>&1
if errorlevel 1 (
  echo [2/3] Creating WSL1 distro %DISTRO% ^(first run; downloads Ubuntu rootfs^)...
  if not exist "%DISTRO_DIR%" mkdir "%DISTRO_DIR%"
  curl -fL "%ROOTFS_URL%" -o "%TEMP%\wwc-rootfs.tar.gz" || goto :fail
  wsl --import %DISTRO% "%DISTRO_DIR%" "%TEMP%\wwc-rootfs.tar.gz" --version 1 || goto :fail
  del "%TEMP%\wwc-rootfs.tar.gz" >nul 2>&1
) else (
  echo [2/3] Using existing WSL distro %DISTRO%
)

REM [3/3] Compositor install/update inside the distro --------------------------
echo [3/3] Installing compositor into %DISTRO%...
wsl -d %DISTRO% -u root -- bash -lc "set -e; cd /tmp; curl -fL '%WSL_URL%' -o wwc.tgz; tar -xzf wwc.tgz; install -Dm755 wsl-compositor /usr/local/bin/wsl-compositor; export DEBIAN_FRONTEND=noninteractive; apt-get -o APT::Sandbox::User=root update -qq || true; apt-get -o APT::Sandbox::User=root install -y libxkbcommon0 weston dmz-cursor-theme >/dev/null 2>&1 || true; rm -f wwc.tgz" || goto :fail

echo.
echo Launching...
call "%DEST%\run.cmd" %*
exit /b %ERRORLEVEL%

:fail
echo.
echo Installation failed. See the messages above.
exit /b 1
