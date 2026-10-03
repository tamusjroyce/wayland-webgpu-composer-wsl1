@echo off
REM ---------------------------------------------------------------------------
REM One-shot installer / updater / launcher for wayland-webgpu-composer.
REM Pure batch, no PowerShell.
REM
REM Sets up BOTH halves and launches them:
REM   1. Downloads/updates the Windows host (win-host.exe, fb-dump.exe).
REM   2. Ensures a WSL1 distro (imports Ubuntu 22.04 on first run).
REM   3. Installs/updates the Linux compositor + a demo desktop inside it.
REM   4. Launches the compositor and the WebGPU window.
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

REM Default client; override via first argument.
set "CLIENT=weston --use-pixman --width=1280 --height=800"
if not "%~1"=="" set "CLIENT=%~1"

echo === wayland-webgpu-composer installer ===

REM [1/5] Windows binaries -----------------------------------------------------
echo [1/5] Downloading Windows host...
where curl >nul 2>&1 || (echo ERROR: curl.exe not found ^(needs Windows 10 1803+^).& goto :fail)
where tar  >nul 2>&1 || (echo ERROR: tar.exe not found ^(needs Windows 10 1803+^).& goto :fail)
if not exist "%DEST%" mkdir "%DEST%"
curl -fL "%WIN_URL%" -o "%TEMP%\%WIN_ASSET%" || goto :fail
tar -xf "%TEMP%\%WIN_ASSET%" -C "%DEST%" || goto :fail
del "%TEMP%\%WIN_ASSET%" >nul 2>&1
echo       installed to %DEST%

REM [2/5] WSL distro -----------------------------------------------------------
where wsl >nul 2>&1 || (echo ERROR: WSL is not installed. Run: wsl --install& goto :fail)
wsl -l -q | findstr /i /c:"%DISTRO%" >nul 2>&1
if errorlevel 1 (
  echo [2/5] Creating WSL1 distro %DISTRO% ^(first run; downloads Ubuntu rootfs^)...
  if not exist "%DISTRO_DIR%" mkdir "%DISTRO_DIR%"
  curl -fL "%ROOTFS_URL%" -o "%TEMP%\wwc-rootfs.tar.gz" || goto :fail
  wsl --import %DISTRO% "%DISTRO_DIR%" "%TEMP%\wwc-rootfs.tar.gz" --version 1 || goto :fail
  del "%TEMP%\wwc-rootfs.tar.gz" >nul 2>&1
) else (
  echo [2/5] Using existing WSL distro %DISTRO%
)

REM [3/5] Compositor install/update inside the distro --------------------------
echo [3/5] Installing compositor into %DISTRO%...
wsl -d %DISTRO% -u root -- bash -lc "set -e; cd /tmp; curl -fL '%WSL_URL%' -o wwc.tgz; tar -xzf wwc.tgz; install -Dm755 wsl-compositor /usr/local/bin/wsl-compositor; export DEBIAN_FRONTEND=noninteractive; apt-get -o APT::Sandbox::User=root update -qq || true; apt-get -o APT::Sandbox::User=root install -y libxkbcommon0 weston dmz-cursor-theme >/dev/null 2>&1 || true; rm -f wwc.tgz" || goto :fail

REM [4/5] Launch the compositor ------------------------------------------------
echo [4/5] Starting compositor...
if not exist "%DEST%\fb" mkdir "%DEST%\fb"
set "FB_WIN=%DEST%\fb\desktop.fb"
REM Convert C:\path\file -> /mnt/c/path/file (strip drive colon, backslashes -> slashes,
REM lowercase the drive letter).
set "FB_REST=%FB_WIN:~2%"
set "FB_REST=%FB_REST:\=/%"
set "FB_DRV=%FB_WIN:~0,1%"
for %%L in (a b c d e f g h i j k l m n o p q r s t u v w x y z) do if /i "%FB_DRV%"=="%%L" set "FB_DRV=%%L"
set "FB_WSL=/mnt/%FB_DRV%%FB_REST%"
start "wwc-compositor" wsl -d %DISTRO% -u root -- bash -lc "pkill -9 -x wsl-compositor 2>/dev/null; mkdir -p /tmp/wwc-desk; XDG_RUNTIME_DIR=/tmp/wwc-desk /usr/local/bin/wsl-compositor --shm '%FB_WSL%' --width 1440 --height 900 -c '%CLIENT%'"

REM [5/5] Launch the WebGPU window ---------------------------------------------
echo [5/5] Starting WebGPU window...
taskkill /im win-host.exe /f >nul 2>&1
start "" "%DEST%\win-host.exe"

echo.
echo Done. The compositor (console) and the WebGPU window are running.
echo Re-run install.cmd any time to update to the latest release and relaunch.
exit /b 0

:fail
echo.
echo Installation failed. See the messages above.
exit /b 1
