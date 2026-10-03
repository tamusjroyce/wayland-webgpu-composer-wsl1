#!/bin/sh
# ---------------------------------------------------------------------------
# Trust the Wayland WebGPU Composer signing certificate so the (self-signed,
# beta) MSIX can be installed. For Git Bash on Windows. Download this next to
# `wayland-webgpu-composer.cer` and run from an elevated (Administrator) shell.
# ---------------------------------------------------------------------------
dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
cer="$dir/wayland-webgpu-composer.cer"
[ -f "$cer" ] || cer="./wayland-webgpu-composer.cer"
if [ ! -f "$cer" ]; then
	echo "ERROR: wayland-webgpu-composer.cer not found next to this script." >&2
	echo "Download it from the release and place it in the same folder." >&2
	exit 1
fi

# certutil wants a Windows path.
cer_win="$(cygpath -w "$cer" 2>/dev/null || echo "$cer")"

# Sideloaded MSIX trust uses LocalMachine\TrustedPeople, which needs elevation.
if ! net session >/dev/null 2>&1; then
	echo "Please run from an elevated (Administrator) Git Bash." >&2
	exit 1
fi

certutil -addstore -f TrustedPeople "$cer_win" || exit 1

echo
echo "Certificate trusted. You can now install wayland-webgpu-composer-windows-x64.msix."
