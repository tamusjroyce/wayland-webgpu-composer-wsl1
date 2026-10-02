#!/usr/bin/env bash
# Build and install the WSL1 Wayland -> WebGPU compositor into the current (WSL1) distro,
# and install Wayland client libraries + demo clients to test against it.
#
# Run inside the WSL1 distro, e.g.:
#   wsl -d WWC-WSL1 -u root -- bash /mnt/c/projects/tamus/wayland-webgpu-composer/scripts/wsl1-install.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
export DEBIAN_FRONTEND=noninteractive

SUDO=""
if [ "$(id -u)" -ne 0 ]; then SUDO="sudo"; fi

echo "== [1/3] Installing system packages =="
$SUDO apt-get update -qq
# Build deps for the compositor + Wayland client libraries + demo clients.
# The `weston` package provides weston-terminal, weston-simple-shm, weston-flower, etc.
$SUDO apt-get install -y -qq \
  build-essential pkg-config libxkbcommon-dev curl ca-certificates \
  libwayland-client0 libwayland-server0 libxkbcommon0 \
  weston dmz-cursor-theme
# `foot` is a lightweight real Wayland terminal (universe); optional.
$SUDO apt-get install -y -qq foot 2>/dev/null || echo "(foot unavailable; using weston demo clients)"

echo "== [2/3] Ensuring Rust toolchain =="
if ! command -v cargo >/dev/null 2>&1 && [ ! -x "$HOME/.cargo/bin/cargo" ]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal
fi
# shellcheck disable=SC1091
source "$HOME/.cargo/env"

echo "== [3/3] Building and installing the compositor =="
cd "$REPO_ROOT/src/wsl-compositor"
# Keep the build cache on the Linux filesystem (fast) and off the Windows target/ dir.
CARGO_TARGET_DIR="$HOME/wwc-target" cargo build --release
$SUDO install -Dm755 "$HOME/wwc-target/release/wsl-compositor" /usr/local/bin/wsl-compositor

echo
echo "Installed: $(command -v wsl-compositor)"
echo "Wayland clients available:"
for c in weston-terminal weston-simple-shm weston-flower foot; do
  command -v "$c" >/dev/null 2>&1 && echo "  - $c"
done
echo
echo "Next: scripts/wsl1-visual-test.sh   (or run scripts/visual-test.ps1 from Windows)"
