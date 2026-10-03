@echo off
REM ---------------------------------------------------------------------------
REM Build the Windows host binaries (win-host, fb-dump) from the Cargo workspace.
REM
REM Usage:
REM   build.cmd          release build (default)
REM   build.cmd debug    debug build
REM   build.cmd clean    cargo clean, then release build
REM
REM The Linux compositor builds separately inside WSL with build.sh.
REM ---------------------------------------------------------------------------
setlocal EnableExtensions

set "PROFILE=--release"
set "OUTDIR=release"
if /i "%~1"=="debug" (
  set "PROFILE="
  set "OUTDIR=debug"
)
if /i "%~1"=="clean" cargo clean || exit /b 1

echo Building Windows host (%OUTDIR%)...
cargo build %PROFILE% --workspace || exit /b 1

echo.
echo Built: target\%OUTDIR%\win-host.exe, target\%OUTDIR%\fb-dump.exe
