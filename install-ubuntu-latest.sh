#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Install the latest Ubuntu LTS as a NEW WSL1 distro, alongside existing WSL
# instances. For Git Bash on Windows (also works from inside a WSL distro via
# wsl.exe interop). Does not touch any existing distro.
#
# Usage:
#   ./install-ubuntu-latest.sh [DistroName] [codename] [version]
#   ./install-ubuntu-latest.sh                      -> Ubuntu-Latest-WSL1 (noble 24.04)
#   ./install-ubuntu-latest.sh Ubuntu-Noble-WSL1    -> custom distro name
#   ./install-ubuntu-latest.sh U2410 oracular 24.10 -> a different release
#
# Rationale: newer compositors (labwc, wayfire, river, niri, ...) are only
# packaged on newer Ubuntu than the jammy base used by install.sh.
# ---------------------------------------------------------------------------
set -eu

DISTRO="${1:-Ubuntu-Latest-WSL1}"
CODENAME="${2:-noble}"
VERSION="${3:-24.04}"
ROOTFS="ubuntu-${VERSION}-server-cloudimg-amd64-root.tar.xz"
URL="https://cloud-images.ubuntu.com/releases/${CODENAME}/release/${ROOTFS}"

command -v wsl.exe >/dev/null 2>&1 || { echo "error: wsl.exe not found; run from Git Bash on Windows." >&2; exit 1; }
command -v curl    >/dev/null 2>&1 || { echo "error: curl not found." >&2; exit 1; }

# Convert a unix path to a Windows path (Git Bash -> cygpath, WSL -> wslpath).
to_win() {
	if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"
	elif command -v wslpath >/dev/null 2>&1; then wslpath -w "$1"
	else echo "$1"; fi
}
to_unix() {
	if command -v cygpath >/dev/null 2>&1; then cygpath -u "$1"
	elif command -v wslpath >/dev/null 2>&1; then wslpath -u "$1"
	else echo "$1"; fi
}

echo "=== Install latest Ubuntu ($CODENAME $VERSION) as WSL1 distro \"$DISTRO\" ==="

# Refuse to clobber an existing distro of the same name (keep others intact).
if wsl.exe -l -q 2>/dev/null | tr -d '\r' | grep -qix "$DISTRO"; then
	echo "error: a WSL distro named '$DISTRO' already exists." >&2
	echo "Pick another name, e.g.:  ./install-ubuntu-latest.sh Ubuntu-Noble-WSL1" >&2
	exit 1
fi

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT
tarball="$workdir/$ROOTFS"

echo "[1/3] Downloading $URL"
curl -fL "$URL" -o "$tarball"

# Install under %LOCALAPPDATA%\WSL\<distro> when available, else the home dir.
if [ -n "${LOCALAPPDATA:-}" ]; then
	base_unix="$(to_unix "$LOCALAPPDATA")"
else
	base_unix="$HOME"
fi
distro_dir_unix="$base_unix/WSL/$DISTRO"
mkdir -p "$distro_dir_unix"

echo "[2/3] Importing as WSL1 (existing distros untouched)..."
# Current WSL imports xz-compressed rootfs tarballs directly.
wsl.exe --import "$DISTRO" "$(to_win "$distro_dir_unix")" "$(to_win "$tarball")" --version 1

echo "[3/3] Verifying..."
wsl.exe -d "$DISTRO" -- cat /etc/os-release | tr -d '\r' | grep -i PRETTY_NAME || true

echo
echo "Done. New WSL1 distro: $DISTRO  (your other distros are unchanged)"
echo "  Open a shell:    wsl -d $DISTRO"
echo "  Remove it later: wsl --unregister $DISTRO"
