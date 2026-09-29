#!/usr/bin/env bash
#
# Runs PrimoDock on your real session, in place of Plank, and puts Plank back
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
CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/primodock/config.toml"
PLANK_WAS_RUNNING=0

for binary in primodockd primodock-shell; do
    [ -x "$ROOT/target/release/$binary" ] || {
        echo "missing $binary — run: cargo build --release" >&2
        exit 1
    }
done

if [ ! -f "$CONFIG" ]; then
    echo "no config yet, seeding one from your Plank launchers:"
    echo "  $CONFIG"
    mkdir -p "$(dirname "$CONFIG")"
    cp "$ROOT/examples/config-from-plank.toml" "$CONFIG"
fi

restore() {
    local code=$?
    echo
    echo "stopping PrimoDock"
    pkill -x primodock-shell 2>/dev/null
    pkill -x primodockd 2>/dev/null
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

echo "config: $CONFIG"
"$ROOT/target/release/primodockd" &
sleep 1
"$ROOT/target/release/primodock-shell" &

echo
echo "PrimoDock is running. Left click activates, right click pins or closes."
echo "The chip on the left switches environment. Press Ctrl-C to stop and get"
echo "Plank back."
wait
