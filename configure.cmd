@echo off
REM Fetch the Smithay library the WSL1 compositor builds against. Smithay is not
REM vendored in this repo; it is cloned into .\smithay (git-ignored). Pinned to a
REM known-good commit so local and CI builds are reproducible.
set SMITHAY_COMMIT=118e34ffc9b99854a2230c4805e1f37dc9029edb

if not exist smithay\.git git clone https://github.com/Smithay/smithay.git smithay || exit /b 1
git -C smithay fetch origin %SMITHAY_COMMIT% || exit /b 1
git -C smithay checkout %SMITHAY_COMMIT% || exit /b 1

REM Apply the WSL1 keymap fix (sealed memfd -> shm fallback) unless already applied.
findstr /c:"fn with_shm" smithay\src\utils\sealed_file.rs >nul 2>&1 || git -C smithay apply ../patches/smithay-sealed-file-wsl1.patch || exit /b 1