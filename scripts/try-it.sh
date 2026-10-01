#!/usr/bin/env bash
#
# Runs Doca on your real session, in place of Plank, and puts Plank back
# when you stop it.
#
# Two docks cannot share an edge: both reserve space through the same strut
# protocol and each reacts to the other's reservation. So Plank is stopped for
# the duration and restarted on the way out — on Ctrl-C, on error, on hangup.
#
#   ./scripts/try-it.sh
#
# If this script is killed outright (kill -9, a crash, the machine sleeping
# badly) Plank will not come back on its own. Start it again with:
#
#   setsid plank >/dev/null 2>&1 &
#
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/doca/config.toml"
PLANK_WAS_RUNNING=0

for binary in docad doca-shell doca-prefs; do
    [ -x "$ROOT/target/release/$binary" ] || {
        echo "missing $binary — run: cargo build --release" >&2
        exit 1
    }
done

# The dock's "Preferences…" spawns doca-prefs by name, on the assumption that
# whatever found doca-shell on PATH finds it too. Nothing here is installed, so
# without this the menu entry fails with "No such file or directory" in a log
# nobody is reading, and the window simply never appears.
export PATH="$ROOT/target/release:$PATH"

if [ ! -f "$CONFIG" ]; then
    echo "no config yet, seeding one from your Plank launchers:"
    echo "  $CONFIG"
    mkdir -p "$(dirname "$CONFIG")"
    cp "$ROOT/examples/config-from-plank.toml" "$CONFIG"
fi

RESTORED=0
restore() {
    local code=$?
    [ "$RESTORED" = "1" ] && exit $code
    RESTORED=1
    echo
    echo "stopping Doca"
    # The preferences window too: it is a separate process the dock spawned,
    # and it would otherwise outlive the dock it was opened to configure.
    pkill -x doca-prefs 2>/dev/null
    pkill -x doca-shell 2>/dev/null
    pkill -x docad 2>/dev/null
    if [ "$PLANK_WAS_RUNNING" = "1" ] && ! pgrep -x plank >/dev/null; then
        echo "restarting Plank"
        setsid plank >/dev/null 2>&1 < /dev/null &
        sleep 1
    fi
    echo "done"
    exit $code
}
trap restore EXIT INT TERM HUP

if pgrep -x plank >/dev/null; then
    PLANK_WAS_RUNNING=1
    echo "stopping Plank (it comes back when you stop this script)"
    pkill -x plank
    sleep 1
fi

cat <<'ESCAPE'

  ------------------------------------------------------------------
  IF THE DOCK EVER COVERS THIS TERMINAL AND YOU CANNOT REACH Ctrl-C:

    press  Ctrl+Alt+F3   to reach a text console, log in, then run

      pkill -x doca-prefs; pkill -x doca-shell; pkill -x docad; setsid plank &

    press  Ctrl+Alt+F2   to come back to the desktop.

  The bar is capped at a fraction of the screen height so this should
  not happen. It is written here because it did once.
  ------------------------------------------------------------------

ESCAPE

# A daemon left over from an earlier run holds the bus name, and the one
# started here loses it and dies. The dock then comes up talking to the old
# daemon, which read the config when *it* started — so config changes appear
# to have been ignored, with the reason buried in the scrollback. Whatever is
# already running is stopped first.
if pgrep -x docad >/dev/null || pgrep -x doca-shell >/dev/null; then
    echo "stopping a Doca that was already running"
    pkill -x doca-prefs 2>/dev/null
    pkill -x doca-shell 2>/dev/null
    pkill -x docad 2>/dev/null
    sleep 1
fi

echo "config: $CONFIG"
"$ROOT/target/release/docad" &
sleep 1
"$ROOT/target/release/doca-shell" &

echo
echo "Doca is running. Left click activates, right click pins or closes —"
echo "or opens Preferences, which edits this same config while it runs."
echo "The environment follows the workspace. Press Ctrl-C to stop and get"
echo "Plank back."
wait
