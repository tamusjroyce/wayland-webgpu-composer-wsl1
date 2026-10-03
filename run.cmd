@echo off
REM ---------------------------------------------------------------------------
REM Launch the compositor (in WSL) and the WebGPU window. Assumes install.cmd has
REM already installed both halves. Pass a client command to override the demo
REM desktop:
REM   run.cmd "gnome-calculator"
REM ---------------------------------------------------------------------------
setlocal EnableExtensions

set "DISTRO=WWC-WSL1"
set "DEST=%LOCALAPPDATA%\wayland-webgpu-composer"
set "CLIENT=weston --use-pixman --width=1280 --height=800"
if not "%~1"=="" set "CLIENT=%~1"

REM Compositor shared-framebuffer path: C:\...\fb\desktop.fb -> /mnt/c/.../fb/desktop.fb
REM (strip drive colon, backslashes -> slashes, lowercase the drive letter).
if not exist "%DEST%\fb" mkdir "%DEST%\fb"
set "FB_WIN=%DEST%\fb\desktop.fb"
set "FB_REST=%FB_WIN:~2%"
set "FB_REST=%FB_REST:\=/%"
set "FB_DRV=%FB_WIN:~0,1%"
for %%L in (a b c d e f g h i j k l m n o p q r s t u v w x y z) do if /i "%FB_DRV%"=="%%L" set "FB_DRV=%%L"
set "FB_WSL=/mnt/%FB_DRV%%FB_REST%"

echo Starting compositor...
start "wwc-compositor" wsl -d %DISTRO% -u root -- bash -lc "pkill -9 -x wsl-compositor 2>/dev/null; mkdir -p /tmp/wwc-desk; XDG_RUNTIME_DIR=/tmp/wwc-desk /usr/local/bin/wsl-compositor --shm '%FB_WSL%' --width 1440 --height 900 -c '%CLIENT%'"

echo Starting WebGPU window...
taskkill /im win-host.exe /f >nul 2>&1
start "" "%DEST%\win-host.exe"

echo.
echo The compositor (console) and the WebGPU window are running.
exit /b 0
