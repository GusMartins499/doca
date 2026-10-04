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
# And that scripts/bind-key.sh and the window's Shortcuts tab write the same
# slot, label and command. They are two implementations of one algorithm, and
# only safe while they agree: a slot named differently would sit beside the
# other's binding rather than replace it, and one key would fire twice.
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
for binary in docad doca-shell doca-prefs; do
    [ -x "$ROOT/target/release/$binary" ] || {
        echo "missing $binary — run: cargo build --release" >&2
        exit 1
    }
done

# Set up and exported *before* dbus-run-session, and that order is the whole
# point. dconf writes do not go to the file the writing process points at:
# they go over the session bus to dconf-service, which inherits its
# environment from the bus daemon. Exporting XDG_CONFIG_HOME after the bus has
# started gives a session that *reads* this temporary database and *writes*
# the real one — which is how a run of this script once replaced its author's
# own keyboard shortcuts. The bus has to be born already pointing here.
WORK="${WORK:-$(mktemp -d)}"
export WORK
export XDG_CONFIG_HOME="$WORK/config"
mkdir -p "$XDG_CONFIG_HOME/doca" "$XDG_CONFIG_HOME/dconf"

if [ "${INSIDE:-0}" != "1" ]; then
    Xvfb "$DISPLAY_NUM" -screen 0 1280x800x24 >/dev/null 2>&1 &
    SERVER=$!
    trap 'kill $SERVER 2>/dev/null; wait 2>/dev/null' EXIT INT TERM
    sleep 1
    INSIDE=1 DISPLAY="$DISPLAY_NUM" exec dbus-run-session -- "$0" "$@"
fi

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

# What the Shortcuts tab reads, and the one thing no unit test can reach: the
# window and scripts/bind-key.sh write the *same* slot. Two implementations of
# one algorithm is the price of the script staying useful, and the price is
# only safe while they agree — a slot named differently would not replace the
# script's binding, it would sit beside it, and one key would fire twice.
#
# dconf is written here, not on your desktop: XDG_CONFIG_HOME was exported
# before the bus was started, so dconf-service has this session's own
# database. That is load-bearing rather than tidy, so it is not assumed —
# `isolated` below proves it before anything shared is touched.
SLOT_SCHEMA=org.gnome.settings-daemon.plugins.media-keys.custom-keybinding
SLOT_ROOT=/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings

# Whether what this session writes is what this session reads.
#
# The failure being guarded is silent and asymmetric: writes landing in the
# real dconf while reads come back from the empty one. A sentinel on a slot of
# our own catches it, and catches it *before* the shared list of keybindings
# is written, which is the part that cannot be guessed back afterwards.
isolated() {
    local probe="$SLOT_ROOT/doca-selftest/"
    gsettings set "$SLOT_SCHEMA:$probe" name "isolation probe" >/dev/null 2>&1
    local back
    back=$(gsettings get "$SLOT_SCHEMA:$probe" name 2>/dev/null)
    dconf reset -f "$probe" >/dev/null 2>&1
    [ "$back" = "'isolation probe'" ]
}

# The slot schema is relocatable — one copy per keybinding path — so it is
# listed by list-relocatable-schemas and not by list-schemas.
if ! gsettings list-schemas 2>/dev/null | grep -qx org.gnome.settings-daemon.plugins.media-keys \
    || ! gsettings list-relocatable-schemas 2>/dev/null | grep -qx "$SLOT_SCHEMA"; then
    echo "  skip  GNOME's keybinding schemas are not installed here"
    KEYS_EXPECTED=0
elif ! isolated; then
    fail "dconf here is not this session's own — refusing to write keybindings"
    KEYS_EXPECTED=0
else
    # A keybinding of somebody else's, put there first: what the script and the
    # window must both leave alone.
    THEIRS="$SLOT_ROOT/someone-else/"
    gsettings set "$SLOT_SCHEMA:$THEIRS" name "Open terminal" >/dev/null
    gsettings set "$SLOT_SCHEMA:$THEIRS" binding '<Super>t' >/dev/null
    gsettings set org.gnome.settings-daemon.plugins.media-keys custom-keybindings \
        "['$THEIRS']" >/dev/null

    "$ROOT/scripts/bind-key.sh" '<Super>e' >/dev/null
    "$ROOT/scripts/bind-key.sh" '<Super>1' Work >/dev/null

    LIST=$(gsettings get org.gnome.settings-daemon.plugins.media-keys custom-keybindings)
    case "$LIST" in
        *"$THEIRS"*) ok "the keybinding list kept somebody else's slot" ;;
        *) fail "binding dropped another application's key: $LIST" ;;
    esac

    # The three strings keys.rs pins in its own tests, asserted here against
    # what the script actually wrote. Whoever changes one side fails this line.
    CYCLE="$SLOT_ROOT/doca-cycle/"
    WORK_SLOT="$SLOT_ROOT/doca-work/"
    case "$LIST" in
        *"$CYCLE"*"$WORK_SLOT"*|*"$WORK_SLOT"*"$CYCLE"*)
            ok "the script writes the slots the window writes" ;;
        *) fail "the slot names have drifted apart: $LIST" ;;
    esac
    [ "$(gsettings get "$SLOT_SCHEMA:$CYCLE" name)" = "'Doca: cycle environment'" ] \
        && ok "the binding is labelled the way the window labels it" \
        || fail "the label drifted: $(gsettings get "$SLOT_SCHEMA:$CYCLE" name)"
    # The dock's name is quoted inside the command, because GNOME hands it to
    # a shell and a dock called `My Work` has to stay one argument. Matched
    # with fgrep rather than a shell pattern so the quotes are compared, not
    # re-interpreted on the way in.
    WANT_COMMAND="$INTERFACE.SetEnvironment \"Work\""
    gsettings get "$SLOT_SCHEMA:$WORK_SLOT" command | grep -qF -- "$WANT_COMMAND" \
        && ok "the command is the one the window would write" \
        || fail "the command drifted: $(gsettings get "$SLOT_SCHEMA:$WORK_SLOT" command)"
    KEYS_EXPECTED=2
fi

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

# The window's end of the same question: the keys the script wrote are the
# keys the Shortcuts tab came up holding.
if [ "$KEYS_EXPECTED" != "0" ]; then
    grep -aq "the desktop keeps custom keybindings" "$WORK/prefs.log" \
        && ok "the window found the desktop's keybindings" \
        || fail "the window decided this desktop has no custom keybindings"
    # The count out of the last such line. tracing writes the fields after the
    # message, so the number is dug out rather than matched as part of a
    # sentence — and the colour codes are stripped first, because `ESC[0m` is
    # a digit as far as a pattern is concerned and this read 0 until it was.
    SAW=$(grep -a "showing the keys the desktop holds" "$WORK/prefs.log" |
        tail -1 | sed 's/\x1b\[[0-9;]*m//g' | sed 's/.*keys=\([0-9]\+\).*/\1/')
    [ "$SAW" = "$KEYS_EXPECTED" ] \
        && ok "the window read back both keys the script bound" \
        || fail "the tab read $SAW keys, not $KEYS_EXPECTED"
fi

# The bar itself, for the one thing only it can answer: that a rebuild keeps
# the widget tiles it already had. The bar is rebuilt whenever a window opens,
# closes or takes focus — and now on every config change too, which is every
# frame of a slider being dragged in the window above. Tiles remade at that
# rate are a visible flicker, and a shelf that quietly went back to remaking
# them looks exactly like one that did not. The counts are what tell them
# apart, so they are read out of the bar's own log.
call SetEnvironmentWidgets "Work" "['water', 'clock']" >/dev/null
RUST_LOG=doca_shell=debug "$ROOT/target/release/doca-shell" > "$WORK/shell.log" 2>&1 &
BAR=$!
BAR_UP=0
for _ in $(seq 1 60); do
    grep -aq "the widget shelf" "$WORK/shell.log" && { BAR_UP=1; break; }
    sleep 0.5
done
if [ "$BAR_UP" = "1" ]; then
    ok "the bar came up and put its widget tiles on"

    # A change of appearance rebuilds the bar without touching the widgets, so
    # every tile should be carried over and none made.
    call SetAppearance "{'icon_size': <int32 56>}" >/dev/null
    sleep 2
    SHELF=$(grep -a "the widget shelf" "$WORK/shell.log" | tail -1 |
        sed 's/\x1b\[[0-9;]*m//g')
    case "$SHELF" in
        *made=0*gone=0*) ok "a rebuild carried the tiles over instead of remaking them" ;;
        *) fail "the bar rebuilt its tiles: ${SHELF##*the widget shelf}" ;;
    esac
    case "$SHELF" in
        *kept=2*) ok "both tiles were the ones already on the bar" ;;
        *) fail "the wrong number of tiles was carried over: ${SHELF##*the widget shelf}" ;;
    esac

    # Dropping one widget must take one tile and leave the other alone — the
    # case where remaking everything is easiest and least visible.
    call SetEnvironmentWidgets "Work" "['clock']" >/dev/null
    sleep 2
    SHELF=$(grep -a "the widget shelf" "$WORK/shell.log" | tail -1 |
        sed 's/\x1b\[[0-9;]*m//g')
    case "$SHELF" in
        *kept=1*made=0*gone=1*) ok "one widget left and took only its own tile" ;;
        *) fail "dropping a widget disturbed the rest: ${SHELF##*the widget shelf}" ;;
    esac
else
    fail "the bar never drew its widgets"
    tail -5 "$WORK/shell.log"
fi
kill "$BAR" 2>/dev/null

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

kill "$WINDOW" "$DAEMON" "$BAR" 2>/dev/null
wait 2>/dev/null
rm -rf "$WORK"
[ "$FAILED" = "0" ] && echo "all live checks passed" || echo "live checks failed"
exit "$FAILED"
