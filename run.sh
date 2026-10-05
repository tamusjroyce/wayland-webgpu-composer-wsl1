#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Interactive launcher for wayland-webgpu-composer. Run from **Git Bash on
# Windows**. Asks which WSL distro and render backend (webgpu or vulkan/ash),
# ensures both halves are installed and up to date (install.sh), then launches
# the Windows host + a desktop nested on the compositor (auto-detected in the
# chosen distro).
#
#   ./run.sh [distro] [backend] [desktop]
#     distro   WSL distro (asked if omitted)
#     backend  webgpu | vulkan (asked if omitted)
#     desktop  sway|labwc|weston|cage|kwin|... (chooser if omitted)
#
# Env: WWC_PORT (default 7900)
# ---------------------------------------------------------------------------
set -euo pipefail

SELFDIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
to_wsl() { echo "$1" | sed -E 's#^/([a-zA-Z])/#/mnt/\L\1/#'; }
REPO_WSL="$(to_wsl "$SELFDIR")"
PORT="${WWC_PORT:-7900}"

command -v wsl >/dev/null || { echo "ERROR: wsl not found (run from Git Bash on Windows)."; exit 1; }

DISTRO="${1:-}"; BACKEND="${2:-}"; DESKTOP="${3:-}"

if [ -z "$DISTRO" ]; then
  echo "Available WSL distros (pick a WSL1 one for this project):"
  WSL_UTF8=1 wsl -l -v | sed 's/\r$//'
  printf "Which distro? "; read -r DISTRO
fi
[ -n "$DISTRO" ] || { echo "ERROR: no distro."; exit 1; }

if [ -z "$BACKEND" ]; then
  echo
  echo "Render backend:"
  echo "  1) webgpu  (wgpu)"
  echo "  2) vulkan  (ash + gpu-allocator)"
  printf "Choose [1-2] (default 1): "; read -r b
  case "${b:-1}" in 2) BACKEND=vulkan;; *) BACKEND=webgpu;; esac
fi
case "$BACKEND" in webgpu|vulkan) ;; *) echo "ERROR: backend must be webgpu|vulkan"; exit 1;; esac

echo
echo "=== Ensuring install in $DISTRO (backend $BACKEND) ==="
WWC_PORT="$PORT" bash "$SELFDIR/install.sh" "$DISTRO" "$BACKEND"

echo
echo "=== Starting win-host (--backend $BACKEND) ==="
MSYS_NO_PATHCONV=1 taskkill //IM win-host.exe //F >/dev/null 2>&1 || true
"$SELFDIR/target/release/win-host.exe" --host "127.0.0.1:$PORT" --backend "$BACKEND" &
HOST_PID=$!
trap 'kill "$HOST_PID" >/dev/null 2>&1 || true' EXIT

echo
echo "=== Launching desktop on $DISTRO (backend $BACKEND, port $PORT) ==="
# run-desktop.sh detects installed desktops and shows a chooser if no name is given.
wsl -d "$DISTRO" -u root -- env WWC_BACKEND="$BACKEND" WWC_PORT="$PORT" bash "$REPO_WSL/run-desktop.sh" $DESKTOP
