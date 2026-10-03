@echo off
REM Fetch the Smithay library the WSL1 compositor builds against. Smithay is not
REM vendored in this repo; it is cloned into .\smithay (git-ignored). Pinned to a
REM known-good commit so local and CI builds are reproducible.
set SMITHAY_COMMIT=118e34ffc9b99854a2230c4805e1f37dc9029edb

if not exist smithay\.git git clone https://github.com/Smithay/smithay.git smithay || exit /b 1
git -C smithay fetch origin %SMITHAY_COMMIT% || exit /b 1
git -C smithay checkout %SMITHAY_COMMIT% || exit /b 1