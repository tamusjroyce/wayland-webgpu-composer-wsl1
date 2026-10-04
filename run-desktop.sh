#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Desktop chooser / launcher for wayland-webgpu-composer (WWC).
#
# Launches a chosen Wayland desktop/compositor *nested* on WWC (software
# rendering, no GPU) and streams it to the Windows host. Run inside the WSL1
# distro (e.g. Ubuntu-Latest-WSL1).
#
#   run-desktop.sh                      # interactive chooser (installed desktops)
#   run-desktop.sh sway|labwc|weston|cage|kwin|wayfire
#
# Then on Windows:
#   win-host --host 127.0.0.1:7900      # or: cargo run -p win-host -- --host 127.0.0.1:7900
#
# Env overrides: WWC_PORT WWC_WIDTH WWC_HEIGHT WWC_FB WWC_BIN
# ---------------------------------------------------------------------------
set -u

PORT="${WWC_PORT:-7900}"
W="${WWC_WIDTH:-1280}"
H="${WWC_HEIGHT:-800}"
FB="${WWC_FB:-/mnt/c/projects/tamus/wayland-webgpu-composer/target/desktop.fb}"
WWC="${WWC_BIN:-/usr/local/bin/wsl-compositor}"

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/wwc-xrd}"
mkdir -p "$XDG_RUNTIME_DIR"; chmod 700 "$XDG_RUNTIME_DIR"
# No GPU on WSL1: software renderers + nested wayland backends.
export WLR_RENDERER=pixman WLR_BACKENDS=wayland WLR_NO_HARDWARE_CURSORS=1
export KWIN_COMPOSE=Q
export XCURSOR_THEME=Adwaita XCURSOR_SIZE=24

# WSL1 lacks memfd_create(); KWin/foot/Qt need it. Build the shim on demand and
# LD_PRELOAD it so those clients fall back to an unlinked temp file.
SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
SHIM=/usr/local/lib/wwc-memfd-shim.so
ensure_shim() {
	if [ ! -e "$SHIM" ] && command -v gcc >/dev/null 2>&1 && [ -f "$SCRIPT_DIR/shims/memfd_shim.c" ]; then
		gcc -shared -fPIC -O2 -o "$SHIM" "$SCRIPT_DIR/shims/memfd_shim.c" 2>/dev/null || true
	fi
	[ -e "$SHIM" ] && export LD_PRELOAD="$SHIM${LD_PRELOAD:+:$LD_PRELOAD}"
}

ALL="sway labwc weston cage kwin wayfire"

compositor_bin() {
	case "$1" in
		sway) echo sway ;; labwc) echo labwc ;; weston) echo weston ;;
		cage) echo cage ;; kwin) echo kwin_wayland ;; wayfire) echo wayfire ;;
	esac
}

compositor_cmd() {
	case "$1" in
		sway)    echo "dbus-run-session -- sway" ;;
		labwc)   echo "dbus-run-session -- labwc" ;;
		weston)  echo "weston --backend=wayland --use-pixman --width=$W --height=$H" ;;
		cage)    echo "dbus-run-session -- cage -- xfce4-terminal" ;;
		kwin)    echo "dbus-run-session -- kwin_wayland --width $W --height $H weston-terminal" ;;
		wayfire) echo "dbus-run-session -- wayfire" ;;
	esac
}

compositor_note() {
	case "$1" in
		cage)    echo " (kiosk: one fullscreen app)" ;;
		kwin)    echo " (KDE/KWin; software via KWIN_COMPOSE=Q + memfd shim)" ;;
		wayfire) echo " (needs DRM/GPU; NOT available on WSL1)" ;;
		*)       echo "" ;;
	esac
}

ensure_machine_id() {
	if [ ! -s /etc/machine-id ]; then
		id=$(dbus-uuidgen)
		printf '%s\n' "$id" > /etc/machine-id
		mkdir -p /var/lib/dbus && printf '%s\n' "$id" > /var/lib/dbus/machine-id
	fi
}

write_configs() {
	cfg="$HOME/.config"
	mkdir -p "$cfg/sway" "$cfg/waybar" "$cfg/labwc"

	cat > "$cfg/sway/config" <<'EOF'
set $mod Mod4
font pango:DejaVu Sans 10
default_border normal 2
default_floating_border normal 2
titlebar_padding 8 4
gaps inner 4
output * bg #203a5c solid_color
seat * xcursor_theme Adwaita 24
for_window [app_id=".*"] floating enable, border normal
for_window [class=".*"] floating enable, border normal
floating_modifier $mod normal
exec waybar
exec weston-terminal
bindsym $mod+Return exec xfce4-terminal
bindsym $mod+d exec wofi --show drun
bindsym Menu exec wofi --show drun
bindsym $mod+q kill
bindsym $mod+Shift+e exit
bindsym $mod+1 workspace number 1
bindsym $mod+2 workspace number 2
bindsym $mod+3 workspace number 3
EOF

	cat > "$cfg/waybar/config" <<'EOF'
{
  "layer": "top", "position": "top", "height": 30,
  "modules-left": ["custom/apps", "sway/workspaces"],
  "modules-center": ["clock"],
  "modules-right": ["cpu", "memory"],
  "custom/apps": { "format": "\u2630 Apps", "on-click": "wofi --show drun", "tooltip": false },
  "sway/workspaces": { "all-outputs": true },
  "clock": { "format": "{:%a %d %b  %H:%M}" },
  "cpu": { "format": "CPU {usage}%", "interval": 2 },
  "memory": { "format": "RAM {percentage}%", "interval": 5 }
}
EOF

	cat > "$cfg/waybar/style.css" <<'EOF'
* { font-family: "DejaVu Sans"; font-size: 13px; }
window#waybar { background: #1b2733; color: #e8eef5; }
#custom-apps { background:#2d6cdf; color:#fff; padding:0 14px; margin:3px; border-radius:4px; font-weight:bold; }
#workspaces button { padding:0 8px; color:#cdd6e0; background:transparent; }
#workspaces button.focused { background:#2d6cdf; color:#fff; border-radius:4px; }
#clock { font-weight:bold; }
#clock,#cpu,#memory { padding:0 10px; }
EOF

	# labwc: native floating WM with a right-click "start" menu + panel autostart.
	cat > "$cfg/labwc/menu.xml" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<openbox_menu>
  <menu id="root-menu" label="Apps">
    <item label="Terminal"><action name="Execute"><command>xfce4-terminal</command></action></item>
    <item label="Files"><action name="Execute"><command>thunar</command></action></item>
    <item label="Calculator"><action name="Execute"><command>galculator</command></action></item>
    <item label="Text Editor"><action name="Execute"><command>mousepad</command></action></item>
    <item label="Image Viewer"><action name="Execute"><command>ristretto</command></action></item>
    <item label="App Menu"><action name="Execute"><command>wofi --show drun</command></action></item>
    <separator/>
    <item label="Reconfigure"><action name="Reconfigure"/></item>
    <item label="Exit"><action name="Exit"/></item>
  </menu>
</openbox_menu>
EOF
	cat > "$cfg/labwc/autostart" <<'EOF'
swaybg -c 203a5c &
waybar &
xfce4-terminal &
EOF
}

choose() {
	echo "Desktops installed on this distro:" >&2
	i=0; MAP=""
	for n in $ALL; do
		if command -v "$(compositor_bin "$n")" >/dev/null 2>&1; then
			i=$((i + 1)); MAP="$MAP $i:$n"
			printf '  %d) %s%s\n' "$i" "$n" "$(compositor_note "$n")" >&2
		fi
	done
	if [ "$i" -eq 0 ]; then
		echo "None installed. Run install-desktops.sh (or install-desktops.cmd) first." >&2
		return 1
	fi
	printf 'Choose a desktop [1-%d]: ' "$i" >&2
	read -r sel
	for pair in $MAP; do
		[ "${pair%%:*}" = "$sel" ] && { echo "${pair#*:}"; return 0; }
	done
	echo "Invalid selection." >&2
	return 1
}

NAME="${1:-}"
if [ -z "$NAME" ]; then
	NAME="$(choose)" || exit 1
fi
BIN="$(compositor_bin "$NAME")"
CMD="$(compositor_cmd "$NAME")"
if [ -z "$BIN" ] || [ -z "$CMD" ]; then
	echo "Unknown desktop: '$NAME' (expected one of: $ALL)" >&2
	exit 1
fi
if ! command -v "$BIN" >/dev/null 2>&1; then
	echo "'$NAME' is not installed ($BIN missing). Install it first:" >&2
	echo "  ONLY=\"$NAME\" bash install-desktops.sh" >&2
	exit 1
fi

ensure_shim
ensure_machine_id
write_configs
rm -f "$XDG_RUNTIME_DIR"/wayland-*
pkill -9 -x wsl-compositor 2>/dev/null
for p in sway labwc weston cage kwin_wayland wayfire waybar swaybg weston-terminal xfce4-terminal; do
	pkill -9 -x "$p" 2>/dev/null
done
sleep 1

echo "Launching '$NAME' on WWC (${W}x${H}, port $PORT)."
echo "On Windows, view it with:  win-host --host 127.0.0.1:$PORT"
exec "$WWC" --shm "$FB" --listen "127.0.0.1:$PORT" --width "$W" --height "$H" -c "$CMD"
