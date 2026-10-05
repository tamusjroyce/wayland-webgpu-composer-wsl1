@echo off
REM ---------------------------------------------------------------------------
REM Interactive launcher for wayland-webgpu-composer. Asks which WSL distro and
REM render backend (webgpu or vulkan/ash), ensures both halves are installed and
REM up to date (install.cmd), then launches the Windows host + a desktop nested
REM on the compositor. The desktop is auto-detected in the chosen distro.
REM
REM   run.cmd [distro] [backend] [desktop]
REM     distro   WSL distro (asked if omitted)
REM     backend  webgpu | vulkan (asked if omitted)
REM     desktop  sway|labwc|weston|cage|kwin|... (chooser if omitted)
REM
REM Env: WWC_PORT (default 7900)
REM ---------------------------------------------------------------------------
setlocal EnableExtensions EnableDelayedExpansion
set "WSL_UTF8=1"
set "SELFDIR=%~dp0"
if not defined WWC_PORT set "WWC_PORT=7900"

where wsl >nul 2>&1 || (echo ERROR: WSL is not installed.& exit /b 1)

set "DISTRO=%~1"
set "BACKEND=%~2"
set "DESKTOP=%~3"

if "%DISTRO%"=="" (
  echo Available WSL distros ^(pick a WSL1 one for this project^):
  wsl -l -v
  set /p "DISTRO=Which distro? "
)
if "%DISTRO%"=="" (echo ERROR: no distro.& exit /b 1)

if "%BACKEND%"=="" (
  echo.
  echo Render backend:
  echo   1^) webgpu  ^(wgpu^)
  echo   2^) vulkan  ^(ash + gpu-allocator^)
  set /p "BSEL=Choose [1-2] ^(default 1^): "
  if "!BSEL!"=="2" (set "BACKEND=vulkan") else (set "BACKEND=webgpu")
)

echo.
echo === Ensuring install in %DISTRO% ^(backend %BACKEND%^) ===
call "%SELFDIR%install.cmd" "%DISTRO%" "%BACKEND%" || (echo ERROR: install failed.& exit /b 1)

echo.
echo === Starting win-host ^(--backend %BACKEND%^) ===
taskkill /im win-host.exe /f >nul 2>&1
start "" "%SELFDIR%target\release\win-host.exe" --host 127.0.0.1:%WWC_PORT% --backend %BACKEND%

echo.
echo === Launching desktop on %DISTRO% ^(backend %BACKEND%, port %WWC_PORT%^) ===
for /f "usebackq delims=" %%p in (`wsl -d %DISTRO% wslpath "%SELFDIR%run-desktop.sh"`) do set "RDWSL=%%p"
wsl -d %DISTRO% -u root -- env WWC_BACKEND=%BACKEND% WWC_PORT=%WWC_PORT% bash "%RDWSL%" %DESKTOP%
endlocal
