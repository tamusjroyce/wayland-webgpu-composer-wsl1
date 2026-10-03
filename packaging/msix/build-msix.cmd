@echo off
REM ---------------------------------------------------------------------------
REM Build (and optionally sign) the Wayland WebGPU Composer MSIX package.
REM
REM Usage:
REM   build-msix.cmd <bin-dir> [out.msix]
REM
REM <bin-dir> must contain: wwc-setup.exe, win-host.exe, fb-dump.exe
REM run.cmd is pulled from the repo root. Set WWC_PFX (+ WWC_PFX_PASS) to sign;
REM without it the package is built unsigned (cannot be installed until signed).
REM
REM Requires the Windows 10/11 SDK (makeappx.exe, signtool.exe).
REM ---------------------------------------------------------------------------
setlocal EnableExtensions EnableDelayedExpansion

set "HERE=%~dp0"
set "BIN=%~1"
if "%BIN%"=="" set "BIN=%HERE%bin"
set "OUT=%~2"
if "%OUT%"=="" set "OUT=%HERE%wayland-webgpu-composer-windows-x64.msix"

REM Locate the newest Windows SDK bin\x64 that has makeappx.exe.
set "SDK="
for /f "delims=" %%D in ('dir /b /ad /o-n "%ProgramFiles(x86)%\Windows Kits\10\bin\10.*" 2^>nul') do (
  if not defined SDK if exist "%ProgramFiles(x86)%\Windows Kits\10\bin\%%D\x64\makeappx.exe" set "SDK=%ProgramFiles(x86)%\Windows Kits\10\bin\%%D\x64"
)
if not defined SDK if exist "%ProgramFiles(x86)%\Windows Kits\10\bin\x64\makeappx.exe" set "SDK=%ProgramFiles(x86)%\Windows Kits\10\bin\x64"
if not defined SDK (echo ERROR: Windows SDK ^(makeappx.exe^) not found. Install the Windows 10/11 SDK.& exit /b 1)
set "MAKEAPPX=%SDK%\makeappx.exe"
set "SIGNTOOL=%SDK%\signtool.exe"

REM Stage the package layout.
set "STAGE=%HERE%stage"
if exist "%STAGE%" rmdir /s /q "%STAGE%"
mkdir "%STAGE%\Assets"
copy /y "%HERE%AppxManifest.xml" "%STAGE%\" >nul
copy /y "%HERE%Assets\*.png" "%STAGE%\Assets\" >nul
copy /y "%BIN%\wwc-setup.exe" "%STAGE%\" >nul || (echo ERROR: wwc-setup.exe not found in "%BIN%".& exit /b 1)
copy /y "%BIN%\win-host.exe"  "%STAGE%\" >nul || (echo ERROR: win-host.exe not found in "%BIN%".& exit /b 1)
copy /y "%BIN%\fb-dump.exe"   "%STAGE%\" >nul || (echo ERROR: fb-dump.exe not found in "%BIN%".& exit /b 1)
copy /y "%HERE%..\..\run.cmd" "%STAGE%\" >nul

echo Packing MSIX...
"%MAKEAPPX%" pack /o /d "%STAGE%" /p "%OUT%" || exit /b 1

if exist "%WWC_PFX%" (
  echo Signing MSIX with "%WWC_PFX%" ...
  "%SIGNTOOL%" sign /fd SHA256 /a /f "%WWC_PFX%" /p "%WWC_PFX_PASS%" "%OUT%" || exit /b 1
) else (
  echo [note] no signing cert ^(WWC_PFX^): the MSIX is UNSIGNED and cannot be installed until signed.
)

echo.
echo Built: %OUT%
exit /b 0
