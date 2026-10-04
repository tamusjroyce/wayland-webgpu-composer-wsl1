#!/bin/sh
# Protocol-coverage e2e check: start the compositor headless, run `wayland-info`, and print
# the advertised global interfaces (one per line, sorted). Used by plan.md Phase 6.
#
# Usage: protocol-check.sh [path-to-wsl-compositor]
# Requires: wayland-info (apt package: wayland-utils).
set -e

BIN="${1:-/usr/local/bin/wsl-compositor}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/wwc-protocol-check}"
mkdir -p "$XDG_RUNTIME_DIR"
FB="$XDG_RUNTIME_DIR/check.fb"

"$BIN" --shm "$FB" --listen 127.0.0.1:0 --width 64 --height 64 >/tmp/wwc-pc.log 2>&1 &
CPID=$!
trap 'kill "$CPID" 2>/dev/null || true' EXIT

# Wait for the wayland socket.
SOCK=""
i=0
while [ "$i" -lt 50 ]; do
	SOCK="$(ls "$XDG_RUNTIME_DIR"/wayland-* 2>/dev/null | grep -v '\.lock$' | head -n1 || true)"
	[ -n "$SOCK" ] && break
	i=$((i + 1))
	sleep 0.1
done
if [ -z "$SOCK" ]; then
	echo "ERROR: compositor socket did not appear; see /tmp/wwc-pc.log" >&2
	exit 1
fi

# `wayland-info` (wayland-utils) is preferred; `weston-info` (weston) is a fallback.
INFO="wayland-info"
command -v "$INFO" >/dev/null 2>&1 || INFO="weston-info"

WAYLAND_DISPLAY="$(basename "$SOCK")" "$INFO" 2>/dev/null \
	| grep -oE "interface: '[a-zA-Z_0-9]+'" \
	| sed "s/interface: '//; s/'//" \
	| sort -u
