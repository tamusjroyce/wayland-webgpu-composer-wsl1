#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Install nested Wayland desktops / compositors into a Debian/Ubuntu system so
# they can be run on top of wayland-webgpu-composer (WWC).
#
# Dual-mode:
#   * Run inside WSL/Linux  -> installs directly with apt-get.
#   * Run from Git Bash on Windows -> re-dispatches into the WWC-WSL1 distro.
#
# Each compositor runs as a nested Wayland client of WWC, e.g.:
#   XDG_RUNTIME_DIR=/tmp/wwc-xrd wsl-compositor --shm /mnt/c/.../visual.fb -c sway
#
# Env overrides:
#   WWC_DISTRO   WSL distro to target when launched from Windows (default WWC-WSL1)
#   ONLY="a b"   install only these package names (space separated)
# ---------------------------------------------------------------------------
set -u

DISTRO="${WWC_DISTRO:-WWC-WSL1}"

# If apt-get is unavailable we are probably in Git Bash on Windows: hand the
# very same script to bash inside the WSL1 distro (where apt-get exists).
if ! command -v apt-get >/dev/null 2>&1; then
	if command -v wsl.exe >/dev/null 2>&1; then
		echo "No apt-get here; running inside WSL distro: $DISTRO"
		exec wsl.exe -d "$DISTRO" -u root bash -s < "$0"
	fi
	echo "error: apt-get not found and no wsl.exe available." >&2
	echo "Run this on a Debian/Ubuntu system (or install WSL)." >&2
	exit 1
fi

if command -v sudo >/dev/null 2>&1 && [ "$(id -u)" -ne 0 ]; then
	SUDO="sudo"
else
	SUDO=""
fi

export DEBIAN_FRONTEND=noninteractive
# `APT::Sandbox::User=root` keeps apt working on WSL1 (the _apt sandbox user
# cannot drop privileges there). ForceIPv4 avoids hangs on IPv6-less WSL1.
APT="$SUDO apt-get -o APT::Sandbox::User=root -o Acquire::ForceIPv4=true -y"

# Friendly name | apt package. Labwc is listed once (the request repeated it).
NAMES="Labwc|Sway|Hyprland|KDE-Plasma(KWin)|Wayfire|Weston|River|Cage|dwl|Niri"
PKGS="labwc|sway|hyprland|kwin-wayland|wayfire|weston|river|cage|dwl|niri"

IFS='|' read -r -a NAME_ARR <<< "$NAMES"
IFS='|' read -r -a PKG_ARR <<< "$PKGS"

# Interactive chooser when launched without ONLY and attached to a terminal.
if [ -z "${ONLY:-}" ] && [ -t 0 ]; then
	echo "Which desktops to install into this distro?"
	for i in "${!NAME_ARR[@]}"; do
		printf '  %2d) %-16s (%s)\n' "$((i + 1))" "${NAME_ARR[$i]}" "${PKG_ARR[$i]}"
	done
	echo "   a) all        r) WSL1-recommended: sway labwc weston cage"
	printf 'Enter numbers (space/comma separated), a, or r [r]: '
	read -r reply
	reply="${reply:-r}"
	case "$reply" in
		a|A) ONLY="" ;;                                   # empty ONLY installs all
		r|R) ONLY="sway labwc weston cage" ;;            # the ones that run on WSL1
		*)
			sel=""
			for tok in $(printf '%s' "$reply" | tr ',' ' '); do
				idx=$((tok - 1))
				if [ "$tok" -ge 1 ] 2>/dev/null && [ -n "${PKG_ARR[$idx]:-}" ]; then
					sel="$sel ${PKG_ARR[$idx]}"
				fi
			done
			ONLY="${sel# }"
			[ -z "$ONLY" ] && { echo "No valid selection; exiting."; exit 1; }
			;;
	esac
	export ONLY
	echo "Selected: ${ONLY:-all}"
fi

echo "=== Updating package index ==="
$APT update || echo "warning: apt-get update reported errors; continuing."

installed=""
missing=""
for i in "${!PKG_ARR[@]}"; do
	pkg="${PKG_ARR[$i]}"
	name="${NAME_ARR[$i]}"
	if [ -n "${ONLY:-}" ] && ! printf '%s\n' $ONLY | grep -qx "$pkg"; then
		continue
	fi
	echo
	echo ">>> $name ($pkg)"
	if $APT install "$pkg"; then
		installed="$installed $name"
	else
		# Not every compositor is packaged on every release (e.g. hyprland, niri,
		# dwl, river on older Ubuntu). Record and keep going.
		missing="$missing $name($pkg)"
	fi
done

echo
echo "============================================================"
echo "Installed:   ${installed:-(none)}"
echo "Unavailable: ${missing:-(none)}"
echo "============================================================"
echo "On WSL1 these run nested on WWC: sway, labwc, weston, cage."
echo "(wayfire needs a GPU; KWin needs memfd_create - both fail on WSL1.)"
echo "Launch one with the chooser:"
echo "  bash run-desktop.sh            # pick a desktop, then run win-host on Windows"
echo "  bash run-desktop.sh sway      # or name it directly"
