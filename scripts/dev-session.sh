#!/usr/bin/env bash
#
# Runs the dock inside a nested X server.
#
# Never test the dock on the session you are working in. A dock sets a strut,
# which changes the desktop work area, and every other dock and panel on that
# display reacts to it. Xephyr gives the bar a display of its own where it can
# reserve whatever it likes without anything outside the window noticing.
#
#   ./scripts/dev-session.sh
#
set -euo pipefail

DISPLAY_NUM="${DISPLAY_NUM:-:9}"
GEOMETRY="${GEOMETRY:-1280x800}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

for tool in Xephyr; do
    command -v "$tool" >/dev/null || {
        echo "missing $tool — install with: sudo apt-get install -y xserver-xephyr" >&2
        exit 1
    }
done

WM=""
for candidate in openbox marco metacity xfwm4 fluxbox icewm; do
    if command -v "$candidate" >/dev/null; then WM="$candidate"; break; fi
done
if [ -z "$WM" ]; then
    echo "no window manager found — install one with: sudo apt-get install -y openbox" >&2
    echo "without a window manager the bar still draws, but nothing honours its" >&2
    echo "strut and no windows are listed, so most of what you want to see is absent." >&2
    exit 1
fi

pids=()
cleanup() {
    for pid in "${pids[@]:-}"; do kill "$pid" 2>/dev/null || true; done
    wait 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo "nested display $DISPLAY_NUM at $GEOMETRY, window manager: $WM"
Xephyr -br -ac -noreset -screen "$GEOMETRY" "$DISPLAY_NUM" >/dev/null 2>&1 &
pids+=($!)
sleep 1

export DISPLAY="$DISPLAY_NUM"
"$WM" >/dev/null 2>&1 &
pids+=($!)
sleep 1

# A couple of windows so the daemon has something to report.
(command -v xterm >/dev/null && xterm >/dev/null 2>&1 &) || true

"$ROOT/target/debug/primodockd" &
pids+=($!)
sleep 1

"$ROOT/target/debug/primodock-shell" &
pids+=($!)

echo "running — close this terminal or press Ctrl-C to tear the session down"
wait
