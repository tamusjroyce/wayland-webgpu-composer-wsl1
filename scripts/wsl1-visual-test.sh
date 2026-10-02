#!/usr/bin/env bash
# Start the compositor and a Wayland client so the composited output can be viewed (via the
# fb-dump PNG snapshot or the Windows win-host). Runs for DURATION seconds then stops.
#
# Usage: wsl1-visual-test.sh [SHM_PATH] [PORT] [CLIENT] [DURATION]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

SHM="${1:-$REPO_ROOT/target/visual.fb}"
PORT="${2:-7900}"
CLIENT="${3:-weston-simple-shm}"
DURATION="${4:-30}"
WIDTH=800
HEIGHT=600

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/wwc-xrd}"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
# Remove any stale sockets so the compositor deterministically picks wayland-1.
rm -f "$XDG_RUNTIME_DIR"/wayland-* 2>/dev/null || true

BIN="$(command -v wsl-compositor || echo /usr/local/bin/wsl-compositor)"
if [ ! -x "$BIN" ]; then
  echo "compositor not installed; run scripts/wsl1-install.sh first" >&2
  exit 1
fi

rm -f "$SHM"
echo "Starting compositor -> $SHM  (listen 127.0.0.1:$PORT, ${WIDTH}x${HEIGHT})"
RUST_LOG=warn "$BIN" --shm "$SHM" --listen "127.0.0.1:$PORT" --width "$WIDTH" --height "$HEIGHT" &
COMP=$!

CLIENT_PID=""
cleanup() {
  kill "$COMP" 2>/dev/null || true
  [ -n "$CLIENT_PID" ] && kill "$CLIENT_PID" 2>/dev/null || true
}
trap cleanup EXIT

export WAYLAND_DISPLAY=wayland-1
# Wait for the Wayland socket to appear.
for _ in $(seq 1 50); do
  [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break
  sleep 0.1
done

if ! command -v "$CLIENT" >/dev/null 2>&1; then
  echo "client '$CLIENT' not found; falling back to weston-terminal" >&2
  CLIENT=weston-terminal
fi
echo "Launching client: $CLIENT"
"$CLIENT" &
CLIENT_PID=$!

echo
echo "Running for ${DURATION}s. While it runs you can:"
echo "  * snapshot:  cargo run -p fb-dump -- <windows-path-to-SHM> out.png"
echo "  * live view: cargo run -p win-host -- --host 127.0.0.1:$PORT"
sleep "$DURATION"
