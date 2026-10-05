@echo off
REM ---------------------------------------------------------------------------
REM Install-only (NO launch) for wayland-webgpu-composer. Builds the Windows host
REM and builds/installs the Linux compositor + runtime deps into a chosen WSL
REM distro. Called by run.cmd, or run directly.
REM
REM   install.cmd [distro] [backend]
REM     distro   WSL distro to install into (asked if omitted).
REM     backend  webgpu (default) | vulkan. The host is ALWAYS built with the
REM              vulkan feature so one binary supports both; backend is a run-time
REM              flag applied by run.cmd, not a separate build.
REM
REM Re-run any time to update: cargo (host + compositor) and apt (deps) are
REM incremental, so this doubles as the version check / updater.
REM
REM Requires: WSL, and cargo/Rust on Windows (https://rustup.rs).
REM ---------------------------------------------------------------------------
setlocal EnableExtensions
REM Make `wsl -l -q` emit UTF-8 so findstr can read it.
set "WSL_UTF8=1"
set "SELFDIR=%~dp0"
set "DISTRO=%~1"
set "BACKEND=%~2"
if "%BACKEND%"=="" set "BACKEND=webgpu"

where wsl   >nul 2>&1 || (echo ERROR: WSL is not installed. Run: wsl --install& exit /b 1)
where cargo >nul 2>&1 || (echo ERROR: cargo/Rust not found on Windows. Install from https://rustup.rs& exit /b 1)

if "%DISTRO%"=="" (
  echo Available WSL distros:
  wsl -l -v
  set /p "DISTRO=Install into which distro? "
)
if "%DISTRO%"=="" (echo ERROR: no distro given.& exit /b 1)
wsl -l -q | findstr /i /c:"%DISTRO%" >nul 2>&1 || (echo ERROR: distro %DISTRO% not found.& exit /b 1)

echo === [1/2] Building win-host ^(release, --features vulkan: supports webgpu + vulkan^) ===
REM Stop a running host so its .exe can be overwritten.
taskkill /im win-host.exe /f >nul 2>&1
cargo build -p win-host --release --features vulkan || (echo ERROR: win-host build failed.& exit /b 1)

echo === [2/2] Installing compositor + deps into %DISTRO% ===
for /f "usebackq delims=" %%p in (`wsl -d %DISTRO% wslpath "%SELFDIR%scripts\wsl1-install.sh"`) do set "SH=%%p"
if not defined SH (echo ERROR: could not locate scripts\wsl1-install.sh.& exit /b 1)
wsl -d %DISTRO% -u root -- bash "%SH%" || (echo ERROR: compositor install failed.& exit /b 1)

echo.
echo Install complete. (No launch.) Backend '%BACKEND%' is applied at run time by run.cmd.
exit /b 0
