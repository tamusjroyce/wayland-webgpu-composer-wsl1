#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Install-only (NO launch) for wayland-webgpu-composer. Run from **Git Bash on
# Windows**. Builds the Windows host and builds/installs the Linux compositor +
# runtime deps into a chosen WSL distro. Called by run.sh, or run directly.
#
#   ./install.sh [distro] [backend]
#     distro   WSL distro to install into (asked if omitted).
#     backend  webgpu (default) | vulkan. The host is ALWAYS built with the
#              vulkan feature so one binary supports both; backend is a run-time
#              flag applied by run.sh, not a separate build.
#
# Re-run any time to update: cargo (host + compositor) and apt (deps) are
# incremental, so this doubles as the version check / updater.
#
# Requires: WSL, and cargo/Rust on Windows (https://rustup.rs).
# ---------------------------------------------------------------------------
set -euo pipefail

SELFDIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Git Bash paths look like /c/... ; WSL needs /mnt/c/... (drive letter lowercased).
to_wsl() { echo "$1" | sed -E 's#^/([a-zA-Z])/#/mnt/\L\1/#'; }
REPO_WSL="$(to_wsl "$SELFDIR")"

DISTRO="${1:-}"
BACKEND="${2:-webgpu}"

command -v wsl   >/dev/null || { echo "ERROR: wsl not found (run from Git Bash on Windows)."; exit 1; }
command -v cargo >/dev/null || { echo "ERROR: cargo/Rust not found. Install from https://rustup.rs"; exit 1; }

if [ -z "$DISTRO" ]; then
  echo "Available WSL distros:"; WSL_UTF8=1 wsl -l -v | sed 's/\r$//'
  printf "Install into which distro? "; read -r DISTRO
fi
[ -n "$DISTRO" ] || { echo "ERROR: no distro given."; exit 1; }
if ! WSL_UTF8=1 wsl -l -q | tr -d '\r' | grep -qi "^${DISTRO}$"; then
  echo "ERROR: distro ${DISTRO} not found."; exit 1
fi

echo "=== [1/2] Building win-host (release, --features vulkan: supports webgpu + vulkan) ==="
# Stop a running host so its .exe can be overwritten.
MSYS_NO_PATHCONV=1 taskkill //IM win-host.exe //F >/dev/null 2>&1 || true
cargo build -p win-host --release --features vulkan

echo "=== [2/2] Installing compositor + deps into ${DISTRO} ==="
wsl -d "$DISTRO" -u root -- bash "$REPO_WSL/scripts/wsl1-install.sh"

echo
echo "Install complete. (No launch.) Backend '${BACKEND}' is applied at run time by run.sh."
