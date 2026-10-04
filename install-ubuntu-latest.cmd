@echo off
REM ---------------------------------------------------------------------------
REM Install the latest Ubuntu LTS as a NEW WSL1 distro, alongside existing WSL
REM instances. Pure batch, no PowerShell. Does not touch any existing distro.
REM
REM Usage:
REM   install-ubuntu-latest.cmd [DistroName] [codename] [version]
REM   install-ubuntu-latest.cmd                      -> Ubuntu-Latest-WSL1 (noble 24.04)
REM   install-ubuntu-latest.cmd Ubuntu-Noble-WSL1    -> custom distro name
REM   install-ubuntu-latest.cmd U2410 oracular 24.10 -> a different release
REM
REM Rationale: newer compositors (labwc, wayfire, river, niri, ...) are only
REM packaged on newer Ubuntu than the jammy base used by install.cmd.
REM Requires Windows 10 1803+ (bundled curl.exe) and WSL.
REM ---------------------------------------------------------------------------
setlocal EnableExtensions

set "DISTRO=%~1"
if "%DISTRO%"=="" set "DISTRO=Ubuntu-Latest-WSL1"
set "CODENAME=%~2"
if "%CODENAME%"=="" set "CODENAME=noble"
set "VERSION=%~3"
if "%VERSION%"=="" set "VERSION=24.04"

set "ROOTFS=ubuntu-%VERSION%-server-cloudimg-amd64-root.tar.xz"
set "URL=https://cloud-images.ubuntu.com/releases/%CODENAME%/release/%ROOTFS%"
set "DISTRO_DIR=%LOCALAPPDATA%\WSL\%DISTRO%"
set "TARBALL=%TEMP%\%ROOTFS%"
REM Make `wsl -l -q` emit UTF-8 so findstr can read it.
set "WSL_UTF8=1"

echo === Install latest Ubuntu (%CODENAME% %VERSION%) as WSL1 distro "%DISTRO%" ===

where wsl  >nul 2>&1 || (echo ERROR: WSL is not installed. Run: wsl --install& goto :fail)
where curl >nul 2>&1 || (echo ERROR: curl.exe not found ^(needs Windows 10 1803+^).& goto :fail)

REM Refuse to clobber an existing distro of the same name (keep others intact).
wsl -l -q | findstr /i /x /c:"%DISTRO%" >nul 2>&1
if not errorlevel 1 (
  echo ERROR: a WSL distro named "%DISTRO%" already exists.
  echo Pick another name, e.g.:  install-ubuntu-latest.cmd Ubuntu-Noble-WSL1
  goto :fail
)

echo [1/3] Downloading %URL%
curl -fL "%URL%" -o "%TARBALL%" || goto :fail

echo [2/3] Importing as WSL1 ^(existing distros are untouched^)...
if not exist "%DISTRO_DIR%" mkdir "%DISTRO_DIR%"
REM Current WSL imports xz-compressed rootfs tarballs directly.
wsl --import "%DISTRO%" "%DISTRO_DIR%" "%TARBALL%" --version 1 || goto :fail
del "%TARBALL%" >nul 2>&1

echo [3/3] Verifying...
wsl -d "%DISTRO%" -- cat /etc/os-release | findstr /i PRETTY_NAME

echo.
echo Done. New WSL1 distro: %DISTRO%  (your other distros are unchanged)
echo   Open a shell:    wsl -d %DISTRO%
echo   Install desktops: install-desktops.cmd   ^(after editing its DISTRO, or run inside^)
echo   Remove it later: wsl --unregister %DISTRO%
goto :eof

:fail
echo Installation failed.
exit /b 1
