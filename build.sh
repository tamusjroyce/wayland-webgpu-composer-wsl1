#!/bin/sh
# ---------------------------------------------------------------------------
# Build the WSL1 Wayland compositor (Linux only). Run ./configure.sh first to
# fetch Smithay into ./smithay.
#
# Usage:
#   ./build.sh          release build (default)
#   ./build.sh debug    debug build
#   ./build.sh clean    cargo clean, then release build
#
# The Windows host binaries build separately on Windows with build.cmd.
# ---------------------------------------------------------------------------
set -e

if [ ! -d smithay ]; then
	echo "smithay/ not found. Run ./configure.sh first." >&2
	exit 1
fi

PROFILE=--release
OUTDIR=release
case "$1" in
	debug) PROFILE=; OUTDIR=debug ;;
	clean) (cd src/wsl-compositor && cargo clean) ;;
esac

echo "Building WSL1 compositor ($OUTDIR)..."
(cd src/wsl-compositor && cargo build $PROFILE)

echo
echo "Built: src/wsl-compositor/target/$OUTDIR/wsl-compositor"
