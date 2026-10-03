#!/bin/sh
# ---------------------------------------------------------------------------
# Launch the compositor (in WSL) and the WebGPU window, from Git Bash on Windows.
# Assumes install.cmd has already installed both halves. Pass a client command to
# override the demo desktop:
#   ./run.sh "gnome-calculator"
#
# The two processes run in the background of this shell; keep it open while using
# the app (closing it stops them).
# ---------------------------------------------------------------------------
DISTRO=WWC-WSL1
CLIENT="${1:-weston --use-pixman --width=1280 --height=800}"

# Windows install dir (LOCALAPPDATA is Windows-style, e.g. C:\Users\..\AppData\Local).
dest_fwd="$(printf '%s' "${LOCALAPPDATA}\\wayland-webgpu-composer" | tr '\\' '/')"
mkdir -p "$dest_fwd/fb"

# Convert C:/path -> /mnt/c/path for the compositor running inside WSL.
fb_fwd="$dest_fwd/fb/desktop.fb"
drive="$(printf '%s' "$fb_fwd" | cut -c1 | tr 'A-Z' 'a-z')"
fb_wsl="/mnt/$drive$(printf '%s' "$fb_fwd" | cut -c3-)"

echo "Starting compositor..."
wsl.exe -d "$DISTRO" -u root -- bash -lc "pkill -9 -x wsl-compositor 2>/dev/null; mkdir -p /tmp/wwc-desk; XDG_RUNTIME_DIR=/tmp/wwc-desk /usr/local/bin/wsl-compositor --shm '$fb_wsl' --width 1440 --height 900 -c '$CLIENT'" &

echo "Starting WebGPU window..."
taskkill //IM win-host.exe //F >/dev/null 2>&1 || true
"$dest_fwd/win-host.exe" &

echo
echo "The compositor and the WebGPU window are running."
