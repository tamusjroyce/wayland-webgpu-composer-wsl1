#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Download, extract, and install the latest WSL1 release of
# wayland-webgpu-composer into a WSL1 distro.
#
# This script ships inside the WSL1 release tarball and is also published as a
# standalone release asset, so it can be fetched and run on its own:
#   curl -fsSL https://github.com/tamusjroyce/wayland-webgpu-composer-wsl1/releases/latest/download/install.sh | bash
#
# Env overrides:
#   PREFIX     install location (default /usr/local -> /usr/local/bin)
#   NO_DEPS=1  skip apt-get runtime-dependency install
# ---------------------------------------------------------------------------
set -eu

REPO="tamusjroyce/wayland-webgpu-composer-wsl1"
ASSET="wayland-webgpu-composer-wsl1-x64.tar.gz"
URL="https://github.com/${REPO}/releases/latest/download/${ASSET}"
PREFIX="${PREFIX:-/usr/local}"

if command -v sudo >/dev/null 2>&1 && [ "$(id -u)" -ne 0 ]; then
	SUDO="sudo"
else
	SUDO=""
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading latest release:"
echo "  $URL"
curl -fL "$URL" -o "$tmp/$ASSET"

echo "Extracting ..."
tar -xzf "$tmp/$ASSET" -C "$tmp"

echo "Installing wsl-compositor to $PREFIX/bin ..."
$SUDO install -Dm755 "$tmp/wsl-compositor" "$PREFIX/bin/wsl-compositor"

if [ "${NO_DEPS:-}" != "1" ] && command -v apt-get >/dev/null 2>&1; then
	echo "Installing runtime dependency (libxkbcommon0) ..."
	$SUDO apt-get update
	$SUDO apt-get install -y libxkbcommon0 || echo "warning: could not install libxkbcommon0; install it manually."
fi

echo
echo "Installed: $(command -v wsl-compositor || echo "$PREFIX/bin/wsl-compositor")"
echo "Run e.g.:"
echo "  XDG_RUNTIME_DIR=/tmp/wwc-xrd wsl-compositor --shm /mnt/c/Users/<you>/wwc/visual.fb -c gnome-calculator"
