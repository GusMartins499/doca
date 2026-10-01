#!/usr/bin/env bash
#
# The checks no unit test can make: a window that is really running, really
# talking to a daemon over a real bus.
#
# Two things it guards, both of which `cargo test` is blind to because both
# need a daemon, a bus and a window at the same time.
#
# The half of the preferences window that has no controls in it: that the
# window hears a change somebody else made. Every control can work perfectly
# while that is broken, because writing is a method call and hearing is a
# signal, and the two fail apart.
#
# And that a widget setting reaches the widget that is already running. The
# value landing in the config file proves nothing, and restarting the daemon
# to check would hide the entire question — which is the question, because a
# setting that needs a restart is a setting the user thinks did not work.
#
# The readiness signal has to be the subscription and not the window. The
# window is drawn well before it subscribes, so a script that waits for the
# window and writes immediately races it — and reports a working window as
# broken, which is how this script first lied to its author.
#
#   cargo build --release && ./scripts/check-live.sh
#
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DISPLAY_NUM="${DISPLAY_NUM:-:21}"
BUS=io.github.gusmartins499.Doca
OBJECT=/io/github/gusmartins499/Doca
INTERFACE=io.github.gusmartins499.Doca1

for tool in Xvfb dbus-run-session gdbus xwininfo; do
    command -v "$tool" >/dev/null || {
        echo "missing $tool — install with:" >&2
        echo "  sudo apt-get install -y xvfb dbus libglib2.0-bin x11-utils" >&2
        exit 1
    }
done
for binary in docad doca-prefs; do
    [ -x "$ROOT/target/release/$binary" ] || {
        echo "missing $binary — run: cargo build --release" >&2
        exit 1
    }
done

if [ "${INSIDE:-0}" != "1" ]; then
    Xvfb "$DISPLAY_NUM" -screen 0 1280x800x24 >/dev/null 2>&1 &
    SERVER=$!
    trap 'kill $SERVER 2>/dev/null; wait 2>/dev/null' EXIT INT TERM
    sleep 1
    INSIDE=1 DISPLAY="$DISPLAY_NUM" exec dbus-run-session -- "$0" "$@"
fi

WORK="$(mktemp -d)"
export XDG_CONFIG_HOME="$WORK/config"
mkdir -p "$XDG_CONFIG_HOME/doca"
printf '[[environments]]\nname = "Work"\n' > "$XDG_CONFIG_HOME/doca/config.toml"

FAILED=0
ok()   { echo "  ok    $1"; }
fail() { echo "  FAIL  $1"; FAILED=1; }

"$ROOT/target/release/docad" > "$WORK/docad.log" 2>&1 &
DAEMON=$!
for _ in $(seq 1 60); do
    gdbus introspect --session --dest "$BUS" --object-path "$OBJECT" >/dev/null 2>&1 && break
    sleep 0.25
done
gdbus introspect --session --dest "$BUS" --object-path "$OBJECT" >/dev/null 2>&1 \
    && ok "the daemon answers on the bus" \
    || { fail "the daemon never came up"; cat "$WORK/docad.log"; exit 1; }

call() {
    gdbus call --session --dest "$BUS" --object-path "$OBJECT" --method "$INTERFACE.$1" "${@:2}" 2>&1
}

# What the Docks tab reads and writes, checked without the window: the tab can
# only be as right as these are.
call ListEnvironments | grep -q "'Work'" \
    && ok "the docks come back with what they pin and show" \
    || fail "ListEnvironments said: $(call ListEnvironments)"

call AddEnvironment "Studio" >/dev/null
APP=$(call ListApplications | tr '(' '\n' | sed -n "s/^'\([^']*\)'.*/\1/p" | head -1)
[ -n "$APP" ] && ok "there are applications to offer ($APP first)" \
    || fail "no installed applications came back"

# The bar pins where you are looking; a settings window pins where you point.
# Work is the dock on screen here, and Studio is the one being edited.
call PinIn "Studio" "$APP" >/dev/null
AFTER=$(call ListEnvironments)
case "$AFTER" in
    *"'Studio'"*"['$APP']"*) ok "a pin landed in the dock that was named" ;;
    *) fail "the pin did not land where it was aimed: $AFTER" ;;
esac
case "$(call CurrentEnvironment)" in
    *Work*) ok "the dock on screen was left alone" ;;
    *) fail "pinning elsewhere moved the dock on screen" ;;
esac

# A refusal has to come back as a sentence, because a window puts it on screen.
case "$(call PinIn "Studio" "nothing.installed.here")" in
    *InvalidArgs*"no application called"*) ok "a refusal says what was wrong" ;;
    *) fail "the refusal was not something a window could show: $(call PinIn "Studio" "nothing.installed.here")" ;;
esac
call RemoveEnvironment "Studio" >/dev/null

# What the Widgets tab reads and writes. The point of these is that the widget
# already running hears it: the value landing in the file proves nothing, and
# a restart would hide the whole question.
call SetEnvironmentWidgets "Work" "['water', 'note']" >/dev/null
RUNNING=$(call ListWidgets)
case "$RUNNING" in
    *"'water'"*"'note'"*) ok "the widgets a dock asked for are the ones running" ;;
    *) fail "the hub did not pick up the new widget list: $RUNNING" ;;
esac

call InvokeWidget "water" "drink" >/dev/null
call InvokeWidget "water" "drink" >/dev/null
case "$(call ListWidgets)" in
    *"'2/8'"*) ok "two glasses were counted" ;;
    *) fail "the water widget did not count: $(call ListWidgets)" ;;
esac

# The whole of F4 in one assertion: a setting written now, to a widget that
# has been running for a while, keeping what it was counting.
call SetWidgetSetting "water" "goal" "<uint32 4>" >/dev/null
case "$(call ListWidgets)" in
    *"'2/4'"*) ok "a new goal reached the running widget and kept today's count" ;;
    *) fail "the running widget did not take the new goal: $(call ListWidgets)" ;;
esac

call SetWidgetSetting "note" "text" "<'milk\nand bread'>" >/dev/null
case "$(call ListWidgets)" in
    *"'milk'"*"'and bread'"*) ok "a note written now is the note the bar shows" ;;
    *) fail "the note widget kept the old text: $(call ListWidgets)" ;;
esac

case "$(call WidgetSettings)" in
    *"'milk\nand bread'"*"uint32 4"*) ok "the settings read back the way they were written" ;;
    *) fail "WidgetSettings said: $(call WidgetSettings)" ;;
esac

# Dropping one widget must not rebuild the others — the water count is the
# only visible proof that the rest were carried over rather than made again.
call SetEnvironmentWidgets "Work" "['water']" >/dev/null
DROPPED=$(call ListWidgets)
case "$DROPPED" in
    *"'note'"*) fail "a widget no dock asks for is still running: $DROPPED" ;;
    *"'2/4'"*) ok "dropping a widget left the others counting where they were" ;;
    *) fail "the surviving widget was rebuilt: $DROPPED" ;;
esac

case "$(call SetWidgetSetting "water" "litres" "<uint32 2>")" in
    *"no setting litres on widget water"*) ok "an unknown setting is refused in words" ;;
    *) fail "the refusal was not showable: $(call SetWidgetSetting "water" "litres" "<uint32 2>")" ;;
esac

RUST_LOG=doca_prefs=info "$ROOT/target/release/doca-prefs" > "$WORK/prefs.log" 2>&1 &
WINDOW=$!
# GtkApplication blocks its own registration until the session's
# xdg-desktop-portal is up, which on a bus this cold is tens of seconds. On a
# real desktop the portal is already running and the window is immediate.
UP=0
for i in $(seq 1 90); do
    # Both conditions: the window is drawn before it subscribes, so waiting on
    # the window alone races the thing this script is here to check.
    if xwininfo -root -children 2>/dev/null | grep -q "Doca Preferences" \
        && grep -aq "following the config" "$WORK/prefs.log"; then
        UP=$i
        break
    fi
    sleep 1
done
[ "$UP" != "0" ] && ok "the window opened and is listening (after ${UP}s on a cold bus)" \
    || { fail "the window never opened, or never subscribed"; cat "$WORK/prefs.log"; }

# The point of the whole script: somebody else writes, and the window follows.
# A keybinding, the bar's own menu and a second window all look like this.
gdbus call --session --dest "$BUS" --object-path "$OBJECT" \
    --method "$INTERFACE.AddEnvironment" "Studio" >/dev/null 2>&1 \
    && ok "a dock was added from outside the window" \
    || fail "the write from outside was refused"
sleep 2

# Asserted on the window's own log, for want of anything else to watch from
# out here. Whoever changes that sentence has to change this line with it.
grep -aq "the config moved" "$WORK/prefs.log" \
    && ok "the window heard it and re-read the config" \
    || fail "the window did not hear ConfigChanged — it is not following the bus"

kill "$WINDOW" "$DAEMON" 2>/dev/null
wait 2>/dev/null
rm -rf "$WORK"
[ "$FAILED" = "0" ] && echo "all live checks passed" || echo "live checks failed"
exit "$FAILED"
