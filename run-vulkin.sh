#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Open the KDE / KWin desktop nested on wayland-webgpu-composer (WWC) using the
# raw-Vulkan host backend (ash + gpu-allocator). Run from **Git Bash on Windows**.
#
#   ./run-vulkin.sh
#
# What it does:
#   1. Builds win-host (release) with the `vulkan` feature.
#   2. Installs the prebuilt compositor ELF into the WSL1 distro.
#   3. Launches KWin nested on WWC, which advertises backend=vulkan in the
#      handshake (WWC_BACKEND=vulkan -> wsl-compositor --backend vulkan).
#   4. Presents it on Windows with  win-host --backend vulkan.
#
# Env overrides: WWC_DISTRO WWC_PORT
# ---------------------------------------------------------------------------
set -euo pipefail

DISTRO="${WWC_DISTRO:-Ubuntu-Latest-WSL1}"
PORT="${WWC_PORT:-7900}"

SELFDIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Git Bash paths look like /c/... ; WSL needs /mnt/c/... (drive letter lowercased).
to_wsl() { echo "$1" | sed -E 's#^/([a-zA-Z])/#/mnt/\L\1/#'; }
REPO_WSL="$(to_wsl "$SELFDIR")"

command -v wsl >/dev/null || { echo "ERROR: wsl not found (run this from Git Bash on Windows)."; exit 1; }
# WSL_UTF8 makes `wsl -l` emit plain UTF-8 instead of UTF-16, so grep works.
if ! WSL_UTF8=1 wsl -l -q | tr -d '\r' | grep -qi "^${DISTRO}$"; then
  echo "ERROR: distro ${DISTRO} not found. Set WWC_DISTRO or run install first."; exit 1
fi

echo "=== Building win-host (release, --features vulkan) ==="
cargo build -p win-host --release --features vulkan

WC="$SELFDIR/target/wsl-compositor-linux"
if [ ! -f "$WC" ]; then
  echo "ERROR: $WC not found."
  echo "Build the Linux compositor first in a cargo-capable WSL distro, e.g.:"
  echo "  wsl -d Ubuntu -u root -- bash -lc 'cd /mnt/c/projects/tamus/wayland-webgpu-composer/src/wsl-compositor && ~/.cargo/bin/cargo build --release && cp target/release/wsl-compositor /mnt/c/projects/tamus/wayland-webgpu-composer/target/wsl-compositor-linux'"
  exit 1
fi

echo "=== Installing compositor into ${DISTRO} ==="
wsl -d "$DISTRO" -u root -- cp "$REPO_WSL/target/wsl-compositor-linux" /usr/local/bin/wsl-compositor

echo "=== Launching KDE/KWin on WWC (backend=vulkan, port ${PORT}) ==="
# Background the nested desktop; the host auto-reconnects to it.
wsl -d "$DISTRO" -u root -- env WWC_BACKEND=vulkan WWC_PORT="$PORT" bash "$REPO_WSL/run-desktop.sh" kwin &
WSL_PID=$!

cleanup() {
  wsl -d "$DISTRO" -u root -- bash -lc "pkill -9 -x wsl-compositor 2>/dev/null; pkill -9 -x kwin_wayland 2>/dev/null" >/dev/null 2>&1 || true
  kill "$WSL_PID" >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "=== Starting win-host (--backend vulkan) ==="
"$SELFDIR/target/release/win-host.exe" --host "127.0.0.1:${PORT}" --backend vulkan
