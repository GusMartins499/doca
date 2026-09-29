#!/usr/bin/env bash
#
# Runs the dock inside a nested X server on a session bus of its own.
#
# Never test the dock on the session you are working in. A dock sets a strut,
# which changes the desktop work area, and every other dock and panel on that
# display reacts to it.
#
# A nested X server alone is not enough. Launching an app goes through the
# session bus, and single-instance apps such as gedit are D-Bus activated: the
# copy already running on your real display answers the request and opens its
# window there, outside the nesting. dbus-run-session gives this session a bus
# of its own so a launch stays where it belongs.
#
#   ./scripts/dev-session.sh
#
set -euo pipefail

DISPLAY_NUM="${DISPLAY_NUM:-:9}"
GEOMETRY="${GEOMETRY:-1280x800}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

for tool in Xephyr dbus-run-session; do
    command -v "$tool" >/dev/null || {
        echo "missing $tool — install with:" >&2
        echo "  sudo apt-get install -y xserver-xephyr dbus openbox" >&2
        exit 1
    }
done

WM=""
for candidate in openbox marco metacity xfwm4 fluxbox icewm; do
    if command -v "$candidate" >/dev/null; then WM="$candidate"; break; fi
done
if [ -z "$WM" ]; then
    echo "no window manager found — install one with: sudo apt-get install -y openbox" >&2
    echo "without one nothing honours the strut and no windows are listed, so most" >&2
    echo "of what you want to look at is absent." >&2
    exit 1
fi

for binary in primodockd primodock-shell; do
    [ -x "$ROOT/target/debug/$binary" ] || {
        echo "missing $binary — run: cargo build" >&2
        exit 1
    }
done

XEPHYR_PID=""
cleanup() {
    [ -n "$XEPHYR_PID" ] && kill "$XEPHYR_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo "display $DISPLAY_NUM at $GEOMETRY, window manager $WM, private session bus"
Xephyr -br -ac -noreset -screen "$GEOMETRY" "$DISPLAY_NUM" >/dev/null 2>&1 &
XEPHYR_PID=$!
sleep 1

export DISPLAY="$DISPLAY_NUM"
export XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$ROOT/.dev-session/config}"
mkdir -p "$XDG_CONFIG_HOME"

echo "config: $XDG_CONFIG_HOME"
echo "running — press Ctrl-C to tear the session down"

dbus-run-session -- bash -c '
    set -u
    "$1" >/dev/null 2>&1 &
    sleep 1
    "$2" &
    sleep 1
    exec "$3"
' _ "$WM" "$ROOT/target/debug/primodockd" "$ROOT/target/debug/primodock-shell"
