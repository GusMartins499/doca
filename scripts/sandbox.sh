#!/usr/bin/env bash
#
# A dock you can click around in, inside a window you can close.
#
# Everything runs on a nested X server with its own session bus and its own
# config: a display of its own, a bus of its own, a config of its own. Nothing
# it does reaches your desktop. The dock inside reserves space on the nested
# screen, not yours. Closing the window ends it.
#
#   ./scripts/sandbox.sh              a window you can see and click
#   ./scripts/sandbox.sh --headless   no window at all, screenshots only
#
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DISPLAY_NUM="${DISPLAY_NUM:-:9}"
HEADLESS=0
[ "${1:-}" = "--headless" ] && HEADLESS=1

if [ "$HEADLESS" = "1" ]; then
    SERVER=Xvfb
    GEOMETRY="${GEOMETRY:-1920x1080}"
else
    SERVER=Xephyr
    GEOMETRY="${GEOMETRY:-1500x820}"
fi

for tool in "$SERVER" dbus-run-session openbox; do
    command -v "$tool" >/dev/null || {
        echo "missing $tool — install with:" >&2
        echo "  sudo apt-get install -y xserver-xephyr xvfb openbox dbus" >&2
        exit 1
    }
done

for binary in primodockd primodock-shell; do
    [ -x "$ROOT/target/release/$binary" ] || {
        echo "missing $binary — run: cargo build --release" >&2
        exit 1
    }
done

SANDBOX="$ROOT/.sandbox"
mkdir -p "$SANDBOX/config/primodock"
if [ ! -f "$SANDBOX/config/primodock/config.toml" ]; then
    cp "$ROOT/examples/config-from-plank.toml" "$SANDBOX/config/primodock/config.toml"
fi

SERVER_PID=""
cleanup() {
    [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null
    wait 2>/dev/null
}
trap cleanup EXIT INT TERM HUP

if [ "$HEADLESS" = "1" ]; then
    Xvfb "$DISPLAY_NUM" -screen 0 "${GEOMETRY}x24" >/dev/null 2>&1 &
else
    Xephyr -br -ac -noreset -title "PrimoDock sandbox — close this window to stop" \
        -screen "$GEOMETRY" "$DISPLAY_NUM" >/dev/null 2>&1 &
fi
SERVER_PID=$!
sleep 2

export DISPLAY="$DISPLAY_NUM"
export XDG_CONFIG_HOME="$SANDBOX/config"

cat <<INFO

  PrimoDock sandbox on $DISPLAY_NUM at $GEOMETRY
  config: $XDG_CONFIG_HOME/primodock/config.toml

  Nothing here touches your desktop. Your Plank keeps running.
  Close the window, or press Ctrl-C here, to stop.

INFO

NOISE='dbus-daemon|fusermount|gvfs|Activating service|Successfully activated'

dbus-run-session -- bash -c '
    set -u
    openbox >/dev/null 2>&1 &
    sleep 1
    "$1" >/dev/null 2>&1 &
    sleep 1
    "$2" >/dev/null 2>&1 &
    sleep 2
    (gnome-calculator >/dev/null 2>&1 &) || true
    (gedit >/dev/null 2>&1 &) || true
    wait
' _ "$ROOT/target/release/primodockd" "$ROOT/target/release/primodock-shell" \
    2> >(grep -vE "$NOISE" >&2)
