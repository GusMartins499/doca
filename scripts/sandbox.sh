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

for binary in docad doca-shell doca-prefs; do
    [ -x "$ROOT/target/release/$binary" ] || {
        echo "missing $binary — run: cargo build --release" >&2
        exit 1
    }
done

SANDBOX="$ROOT/.sandbox"
mkdir -p "$SANDBOX/config/doca"
if [ ! -f "$SANDBOX/config/doca/config.toml" ]; then
    cp "$ROOT/examples/config-from-plank.toml" "$SANDBOX/config/doca/config.toml"
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
    Xephyr -br -ac -noreset -title "Doca sandbox — close this window to stop" \
        -screen "$GEOMETRY" "$DISPLAY_NUM" >/dev/null 2>&1 &
fi
SERVER_PID=$!
sleep 2

export DISPLAY="$DISPLAY_NUM"
# All exported before dbus-run-session below, because a service the bus
# activates takes the *bus daemon's* environment, not the dock's — the lesson
# check-live.sh learned from dconf. The calendar widget (#31) makes it bite
# again: it activates Evolution Data Server, which would otherwise open your
# real calendars, cache and local calendar file from a second process beside
# the one your session already runs.
export XDG_CONFIG_HOME="$SANDBOX/config"
export XDG_STATE_HOME="$SANDBOX/state"
export XDG_CACHE_HOME="$SANDBOX/cache"
export XDG_DATA_HOME="$SANDBOX/data"
mkdir -p "$XDG_STATE_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME"
# Your own apps and icons, so the bar still shows what yours does. Linked, not
# copied, and only these two: everything else under ~/.local/share stays out.
for shared in applications icons; do
    if [ -d "$HOME/.local/share/$shared" ] && [ ! -e "$XDG_DATA_HOME/$shared" ]; then
        ln -s "$HOME/.local/share/$shared" "$XDG_DATA_HOME/$shared"
    fi
done
# The dock's "Preferences…" spawns doca-prefs by name, so the sandbox has to
# find the one that was just built rather than one installed on the system.
export PATH="$ROOT/target/release:$PATH"

cat <<INFO

  Doca sandbox on $DISPLAY_NUM at $GEOMETRY
  config: $XDG_CONFIG_HOME/doca/config.toml

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
' _ "$ROOT/target/release/docad" "$ROOT/target/release/doca-shell" \
    2> >(grep -vE "$NOISE" >&2)
