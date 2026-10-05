@echo off
REM ---------------------------------------------------------------------------
REM Open the KDE / KWin desktop nested on wayland-webgpu-composer (WWC) using the
REM raw-Vulkan host backend (ash + gpu-allocator).
REM
REM   run-vulkin.cmd
REM
REM What it does:
REM   1. Builds win-host (release) with the `vulkan` feature.
REM   2. Installs the prebuilt compositor ELF into the WSL1 distro.
REM   3. Launches KWin nested on WWC, which advertises backend=vulkan in the
REM      handshake (WWC_BACKEND=vulkan -> wsl-compositor --backend vulkan).
REM   4. Presents it on Windows with  win-host --backend vulkan.
REM
REM Env overrides: WWC_DISTRO WWC_PORT
REM ---------------------------------------------------------------------------
setlocal EnableExtensions

REM Make `wsl -l` emit UTF-8 (not UTF-16) so findstr can match the distro name.
set "WSL_UTF8=1"

if not defined WWC_DISTRO set "WWC_DISTRO=Ubuntu-Latest-WSL1"
if not defined WWC_PORT   set "WWC_PORT=7900"
set "SELFDIR=%~dp0"

where wsl >nul 2>&1 || (echo ERROR: WSL is not installed.& exit /b 1)
wsl -l -q | findstr /i /c:"%WWC_DISTRO%" >nul 2>&1 || (echo ERROR: distro %WWC_DISTRO% not found.& exit /b 1)

echo === Building win-host (release, --features vulkan) ===
cargo build -p win-host --release --features vulkan || (echo ERROR: win-host build failed.& exit /b 1)

set "WC=%SELFDIR%target\wsl-compositor-linux"
if not exist "%WC%" (
  echo ERROR: %WC% not found.
  echo Build the Linux compositor first in a cargo-capable WSL distro, e.g.:
  echo   wsl -d Ubuntu -u root -- bash -lc "cd /mnt/c/projects/tamus/wayland-webgpu-composer/src/wsl-compositor ^&^& ~/.cargo/bin/cargo build --release ^&^& cp target/release/wsl-compositor /mnt/c/projects/tamus/wayland-webgpu-composer/target/wsl-compositor-linux"
  exit /b 1
)

REM Translate repo paths to ones WSL can read (/mnt/...).
for /f "usebackq delims=" %%p in (`wsl -d %WWC_DISTRO% wslpath "%WC%"`) do set "WCWSL=%%p"
for /f "usebackq delims=" %%p in (`wsl -d %WWC_DISTRO% wslpath "%SELFDIR%run-desktop.sh"`) do set "RDWSL=%%p"

echo === Installing compositor into %WWC_DISTRO% ===
wsl -d %WWC_DISTRO% -u root -- cp "%WCWSL%" /usr/local/bin/wsl-compositor || (echo ERROR: could not stage the compositor.& exit /b 1)

echo === Launching KDE/KWin on WWC (backend=vulkan, port %WWC_PORT%) ===
start "WWC KDE (vulkan)" wsl -d %WWC_DISTRO% -u root -- env WWC_BACKEND=vulkan WWC_PORT=%WWC_PORT% bash "%RDWSL%" kwin

echo === Starting win-host (--backend vulkan) ===
echo (The host auto-reconnects, so it is fine if the desktop is still starting up.)
timeout /t 2 /nobreak >nul
"%SELFDIR%target\release\win-host.exe" --host 127.0.0.1:%WWC_PORT% --backend vulkan

REM Host window closed: stop the nested desktop + compositor.
wsl -d %WWC_DISTRO% -u root -- bash -lc "pkill -9 -x wsl-compositor 2>/dev/null; pkill -9 -x kwin_wayland 2>/dev/null" >nul 2>&1
endlocal
