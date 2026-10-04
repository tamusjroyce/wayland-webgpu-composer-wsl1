@echo off
REM ---------------------------------------------------------------------------
REM Install nested Wayland desktops / compositors into the WWC-WSL1 distro.
REM Pure batch, no PowerShell. Hands off to install-desktops.sh inside WSL.
REM
REM   install-desktops.cmd            installs all supported compositors
REM   install-desktops.cmd sway       installs only the named package(s)
REM
REM Each compositor runs as a nested Wayland client of wayland-webgpu-composer.
REM ---------------------------------------------------------------------------
setlocal EnableExtensions

set "DISTRO=Ubuntu-Latest-WSL1"
set "SELFDIR=%~dp0"

where wsl >nul 2>&1 || (echo ERROR: WSL is not installed.& goto :fail)
wsl -l -q | findstr /i /c:"%DISTRO%" >nul 2>&1 || (echo ERROR: distro %DISTRO% not found. Run install.cmd first.& goto :fail)

REM Translate this folder's install-desktops.sh to a path WSL can read (/mnt/...).
for /f "usebackq delims=" %%p in (`wsl -d %DISTRO% wslpath "%SELFDIR%install-desktops.sh"`) do set "SHPATH=%%p"
if not defined SHPATH (echo ERROR: could not locate install-desktops.sh.& goto :fail)

echo === Installing nested compositors into %DISTRO% ===
wsl -d %DISTRO% -u root -- env ONLY="%*" bash "%SHPATH%" || goto :fail

echo.
echo Done. Launch one on top of WWC, e.g.:
echo   wsl -d %DISTRO% -u root -- bash -lc "XDG_RUNTIME_DIR=/tmp/wwc-xrd wsl-compositor --shm /mnt/c/Users/%%USERNAME%%/wwc/visual.fb -c sway"
goto :eof

:fail
echo Installation failed.
exit /b 1
