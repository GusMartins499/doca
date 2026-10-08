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
# The counters go here, and for the same reason the config does: a run of
# this script must not touch the state of the desktop it runs on.
export XDG_STATE_HOME="$WORK/state"
mkdir -p "$XDG_CONFIG_HOME/doca" "$XDG_CONFIG_HOME/dconf" "$XDG_STATE_HOME"

if [ "${INSIDE:-0}" != "1" ]; then
    Xvfb "$DISPLAY_NUM" -screen 0 1280x800x24 >/dev/null 2>&1 &
    SERVER=$!
    trap 'kill $SERVER 2>/dev/null; wait 2>/dev/null' EXIT INT TERM
    sleep 1
    INSIDE=1 DISPLAY="$DISPLAY_NUM" exec dbus-run-session -- "$0" "$@"
fi

# One folder and nothing else on the bar, so the one icon is dead centre of
# the dock window and a click needs no arithmetic to find it.
mkdir -p "$WORK/folder"
for n in 1 2 3 4 5 6 7; do : > "$WORK/folder/file-$n.txt"; done
# Written the way a person writes it, comments and all. The daemon rewrites
# this file every time anything is set, and what it does to the parts it was
# not asked about is only visible here — a unit test can check the merge, but
# only a running daemon proves the merge is the thing that runs.
cat > "$XDG_CONFIG_HOME/doca/config.toml" <<CONFIG
# Kept by hand. The daemon must not eat this.
[appearance]
show_trash = false   # the trash is somebody else's business

[[environments]]
name = "Work"
folders = ["$WORK/folder"]
CONFIG

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

# The file is the user's, not ours. A setting written over the bus has to land
# in it without taking the comments and the hand spacing with it — this is the
# one thing `Config::save_to` can get wrong where nothing else would notice,
# because every reader goes through the struct and the struct never had them.
call SetAppearance "{'icon_size': <int32 64>}" >/dev/null
KEPT=$(cat "$XDG_CONFIG_HOME/doca/config.toml")
case "$KEPT" in
    *"icon_size = 64"*) ok "the setting reached the file" ;;
    *) fail "the setting never landed: $KEPT" ;;
esac
case "$KEPT" in
    *"# Kept by hand. The daemon must not eat this."*)
        ok "a comment above a table outlived a write" ;;
    *) fail "the daemon ate a comment it was not asked about: $KEPT" ;;
esac
case "$KEPT" in
    *"show_trash = false   # the trash is somebody else's business"*)
        ok "a line it was not asked about was not reflowed" ;;
    *) fail "the daemon reformatted a line nobody touched: $KEPT" ;;
esac

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
# The cycle order, which is the order of this list. Only an order: a name
# nobody knows is refused, and a dock left out keeps its place at the end, so a
# window working from a list drawn before a rename cannot delete a dock.
call AddEnvironment "Studio" >/dev/null 2>&1
case "$(call ReorderEnvironments "['Studio']")" in
    *error*) fail "reordering the docks was refused: $(call ReorderEnvironments "['Studio']")" ;;
    *) ok "the docks can be put in a new order" ;;
esac
# gdbus wraps the reply in a tuple, so the list opens with `([(`.
case "$(call ListEnvironments)" in
    "([('Studio',"*) ok "the dock named first is first, and the rest kept their places" ;;
    *) fail "the order did not take: $(call ListEnvironments)" ;;
esac
case "$(call ReorderEnvironments "['Nowhere']")" in
    *InvalidArgs*"no dock called Nowhere"*) ok "a reorder cannot invent a dock" ;;
    *) fail "the refusal was not showable: $(call ReorderEnvironments "['Nowhere']")" ;;
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

# Water sends what it drank, what it is aiming at and what one go is worth,
# as numbers — the bar draws out of them, and only the bar decides what that
# says. So the assertions are on the payload the variant carries. All three
# are millilitres.
water_is() {
    case "$(call ListWidgets)" in
        *"uint32 $1, uint32 $2, uint32 $3"*) return 0 ;;
        *) return 1 ;;
    esac
}

call InvokeWidget "water" "drink" >/dev/null
call InvokeWidget "water" "drink" >/dev/null
water_is 1000 2000 500 \
    && ok "two bottles were counted, in millilitres" \
    || fail "the water widget did not count: $(call ListWidgets)"

# The whole of F4 in one assertion: a setting written now, to a widget that
# has been running for a while, keeping what it was counting.
call SetWidgetSetting "water" "goal" "<uint32 1500>" >/dev/null
water_is 1000 1500 500 \
    && ok "a new goal reached the running widget and kept today's count" \
    || fail "the running widget did not take the new goal: $(call ListWidgets)"

# The bottle is a setting too, and changing it must not undo the afternoon.
call SetWidgetSetting "water" "bottle" "<uint32 250>" >/dev/null
water_is 1000 1500 250 \
    && ok "a smaller bottle keeps what was already drunk" \
    || fail "the bottle setting did not land: $(call ListWidgets)"

# Each widget says what shape its state takes, so a reader can have a drawer
# per shape instead of one column of labels for all of them. Checked on the
# wire because the union is spelled by hand — a name beside a variant — and a
# payload written one way and read another is a bar with no tiles on it.
case "$(call ListWidgets)" in
    *"('water', 'water', <(uint32"*) ok "a widget state names the shape it carries" ;;
    *) fail "the typed state did not come back as a name and a payload: $(call ListWidgets)" ;;
esac
case "$(call ListWidgets)" in
    # Two lines, a progress figure and a flag: the tile the nine widgets
    # without a variant of their own still draw.
    *"'note', 'simple', <("*", -1.0, false)>"*)
        ok "a widget with no variant of its own still sends one" ;;
    *) fail "the note did not come back as a simple body: $(call ListWidgets)" ;;
esac

# An action nobody has is refused in words rather than shrugged off, which is
# what makes a panel's controls safe to build from the list the contract
# publishes.
call InvokeWidget "water" "drink" >/dev/null
if grep -aq "takes no action" "$WORK/docad.log"; then
    fail "a real action was reported as unknown"
else
    ok "an action the widget declares is an action it takes"
fi
call InvokeWidget "water" "teleport" >/dev/null
grep -aq "water takes no action teleport" "$WORK/docad.log" \
    && ok "an action nobody has is named in the log" \
    || fail "an unknown action was taken in silence"
call InvokeWidget "water" "undo" >/dev/null

call SetWidgetSetting "note" "text" "<'milk\nand bread'>" >/dev/null
case "$(call ListWidgets)" in
    *"'milk'"*"'and bread'"*) ok "a note written now is the note the bar shows" ;;
    *) fail "the note widget kept the old text: $(call ListWidgets)" ;;
esac

case "$(call WidgetSettings)" in
    *"'milk\nand bread'"*"uint32 1500, uint32 250"*)
        ok "the settings read back the way they were written" ;;
    *) fail "WidgetSettings said: $(call WidgetSettings)" ;;
esac

# Dropping one widget must not rebuild the others — the water count is the
# only visible proof that the rest were carried over rather than made again.
call SetEnvironmentWidgets "Work" "['water']" >/dev/null
DROPPED=$(call ListWidgets)
case "$DROPPED" in
    *"'note'"*) fail "a widget no dock asks for is still running: $DROPPED" ;;
    *"uint32 1000, uint32 1500, uint32 250"*)
        ok "dropping a widget left the others counting where they were" ;;
    *) fail "the surviving widget was rebuilt: $DROPPED" ;;
esac

# What the day accumulated, which no unit test can check the way this can:
# the daemon is stopped and started, over the same state file, and the
# millilitres are still counted. A setting living in the config file and a
# count living in the state file is the whole of why that works.
STATE="${XDG_STATE_HOME:-$HOME/.local/state}/doca/state.toml"
[ -f "$STATE" ] \
    && ok "the day's counters were written to $STATE" \
    || fail "nothing was written to $STATE"
grep -q "^ml = " "$STATE" 2>/dev/null \
    && ok "what was drunk is in the state file" \
    || fail "the state file holds no count: $(cat "$STATE" 2>&1)"
grep -q "\bml\b" "$XDG_CONFIG_HOME/doca/config.toml" \
    && fail "a counter was written into the config file the user edits" \
    || ok "the config file was left to the choices"

kill "$DAEMON" 2>/dev/null
wait "$DAEMON" 2>/dev/null
"$ROOT/target/release/docad" > "$WORK/docad-again.log" 2>&1 &
DAEMON=$!
for _ in $(seq 1 60); do
    gdbus introspect --session --dest "$BUS" --object-path "$OBJECT" >/dev/null 2>&1 && break
    sleep 0.25
done
RESTARTED=$(call ListWidgets)
case "$RESTARTED" in
    *"uint32 1000, uint32 1500, uint32 250"*)
        ok "what was drunk survived a restart of the daemon" ;;
    *) fail "the day started over when the daemon did: $RESTARTED" ;;
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

# The folder grid, which needs a window, a bus and a real click at once.
#
# A click is the only thing that finds what is wrong here, and twice now it
# has: a grid put up inside the button-press handler is taken down again by
# the release of that same click, and a grid taken down to be resized cannot
# be put back up in the same turn — it is swallowed by the teardown of the one
# just dismissed. Both are invisible to `cargo test`, which has no click to
# make and no grab to lose, and both leave exactly the same trace: a folder
# that opens nothing at all.
if ! command -v xdotool >/dev/null; then
    echo "  skip  xdotool is not installed, so nothing here can be clicked"
else
    # Nothing but the folder on the bar: the checks above left a widget
    # running, and a tile beside the icon moves the middle of the bar off it.
    call SetEnvironmentWidgets "Work" "[]" >/dev/null
    "$ROOT/target/release/doca-shell" > "$WORK/shell.log" 2>&1 &
    SHELL_PID=$!
    # By title, not by size: the bar is the window called `Doca`, and GTK
    # keeps several small ones of its own that a size test picks up instead.
    # A menu is titled after the program, so the two never collide.
    BAR=""
    for _ in $(seq 1 60); do
        BAR=$(xwininfo -root -children 2>/dev/null | grep '"Doca":' | head -1)
        [ -n "$BAR" ] && break
        sleep 0.25
    done
    # Read it again once it has settled: the window exists before it is the
    # size it will be, and a click aimed at the first geometry it reports
    # lands wherever the bar used to be.
    sleep 1
    BAR=$(xwininfo -root -children 2>/dev/null | grep '"Doca":' | head -1)

    # The bar's own geometry, so the click lands on the icon rather than on a
    # guess: `WIDTHxHEIGHT+X+Y`, and the one icon is centred in it.
    GEOMETRY=$(echo "$BAR" | sed -n 's/.* \([0-9]*x[0-9]*+-\?[0-9]*+-\?[0-9]*\) .*/\1/p' | head -1)
    if [ -z "$GEOMETRY" ]; then
        fail "the bar never came up"
        cat "$WORK/shell.log"
    else
        ok "the bar is on screen at $GEOMETRY"
        BAR_W=${GEOMETRY%%x*}
        REST=${GEOMETRY#*x}
        BAR_H=${REST%%+*}
        REST=${REST#*+}
        BAR_X=${REST%%+*}
        BAR_Y=${REST#*+}
        AT_X=$(( BAR_X + BAR_W / 2 ))
        AT_Y=$(( BAR_Y + BAR_H - 30 ))

        # Any doca-shell window that is neither the bar nor one of the small
        # ones GTK keeps off screen. The grid is the only thing that can be.
        grid_is_up() {
            xwininfo -root -children 2>/dev/null \
                | grep '"doca-shell":' \
                | grep -v '"Doca":' \
                | awk '{ for (i = 1; i <= NF; i++) if ($i ~ /^[0-9]+x[0-9]+\+/) {
                             split($i, size, "x"); if (size[1] >= 200) { print; break } }}' \
                | head -1
        }

        # The size of the grid, which is the whole of what these check: a
        # skeleton of one row and a folder of seven files are not the same
        # shape, and whether the second ever replaces the first is the
        # question.
        # The geometry token, not the window id: `0x1200084` is a perfectly
        # good `[0-9]+x[0-9]+` and this read the id as a size until it was
        # made to match the `+X+Y` that only a geometry has.
        size_of() {
            grid_is_up | grep -oE '[0-9]+x[0-9]+\+-?[0-9]+\+-?[0-9]+' | head -1 | cut -d+ -f1
        }
        wait_for_grid() {
            for _ in $(seq 1 20); do
                [ -n "$(size_of)" ] && return
                sleep 0.15
            done
        }

        xdotool mousemove "$AT_X" "$AT_Y"
        sleep 0.4
        xdotool click 1
        wait_for_grid
        FIRST=$(size_of)
        [ -n "$FIRST" ] && ok "a folder nobody had opened opens a grid at once ($FIRST)" \
            || fail "the first click on a folder put nothing on screen"
        # Seven files are two rows; the skeleton of a folder nobody has opened
        # is one. The grid has to grow into what arrived rather than scroll.
        GREW=""
        for _ in $(seq 1 20); do
            GREW=$(size_of)
            [ -n "$GREW" ] && [ "$GREW" != "$FIRST" ] && break
            sleep 0.15
        done
        [ -n "$GREW" ] && [ "$GREW" != "$FIRST" ] \
            && ok "the folder that arrived made the grid its own size ($FIRST then $GREW)" \
            || fail "the grid stayed $FIRST with seven files in it, which is a scroll arrow"

        xdotool key Escape
        sleep 0.5
        # And again, now that the dock knows the shape the folder takes. This
        # is the open that happens over and over, and the one that must not
        # move at all: same window, same size, the cells swapped underneath.
        xdotool click 1
        wait_for_grid
        AGAIN=$(size_of)
        [ -n "$AGAIN" ] && ok "a folder opened before opens again ($AGAIN)" \
            || fail "the second click on a folder put nothing on screen"
        sleep 1
        SETTLED_SIZE=$(size_of)
        [ "$SETTLED_SIZE" = "$AGAIN" ] \
            && ok "it opened at the shape it turned out to have, and did not move" \
            || fail "the grid went from $AGAIN to $SETTLED_SIZE on a folder it had opened before"
        xdotool key Escape
    fi
    kill "$SHELL_PID" 2>/dev/null
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

    # The panel, which needs a window, a bus and a real click at once — the
    # same three things the folder grid needs, and for the same reason: a
    # menu put up over a DOCK window either gets its grab or it does not, and
    # nothing short of a click can tell which.
    if ! command -v xdotool >/dev/null; then
        echo "  skip  xdotool is not installed, so no tile can be clicked"
    else
        call SetEnvironmentWidgets "Work" "['water', 'clock']" >/dev/null
        sleep 2
        GEOMETRY=$(xwininfo -root -children 2>/dev/null | grep '"Doca":' |
            grep -oE '[0-9]+x[0-9]+\+-?[0-9]+\+-?[0-9]+' | head -1)
        BAR_WIDTH=$(grep -a "strut applied" "$WORK/shell.log" | tail -1 |
            sed 's/\x1b\[[0-9;]*m//g' | sed 's/.*width=\([0-9]*\).*/\1/')
        if [ -z "$GEOMETRY" ] || [ -z "$BAR_WIDTH" ]; then
            fail "the bar is not on screen to click a tile on"
        else
            WIN_W=${GEOMETRY%%x*}
            REST=${GEOMETRY#*x}
            WIN_H=${REST%%+*}
            REST=${REST#*+}
            WIN_X=${REST%%+*}
            WIN_Y=${REST#*+}

            # The window is wider than the bar by the room a magnified icon
            # spreads into, at both ends — so the bar's own right edge is that
            # much inside the window's. The bar's width is the one the strut
            # was set to, which is the only place it is written down.
            MARGIN=$(( (WIN_W - BAR_WIDTH) / 2 ))
            # The clock is the last thing on the bar and a wide tile: two and
            # a half of the configured 48px icons, inside the bar's own ten
            # pixels of padding and the tile's two. Its middle is that far in
            # from the bar's right edge.
            AT_X=$(( WIN_X + MARGIN + BAR_WIDTH - 10 - 2 - 60 ))
            # And the bar sits at the bottom of the window, so the tiles are a
            # tile's half-height above the bottom edge plus that padding.
            AT_Y=$(( WIN_Y + WIN_H - 10 - 24 ))

            # Any doca-shell window that is neither the bar nor one of the
            # small ones GTK keeps off screen. The panel is the only thing
            # that can be.
            panel_size() {
                xwininfo -root -children 2>/dev/null \
                    | grep '"doca-shell":' \
                    | grep -v '"Doca":' \
                    | grep -oE '[0-9]+x[0-9]+\+-?[0-9]+\+-?[0-9]+' \
                    | awk -F'x' '$1 >= 100 { print; exit }' \
                    | cut -d+ -f1
            }

            xdotool mousemove "$AT_X" "$AT_Y"
            sleep 0.4
            xdotool click 1
            PANEL=""
            for _ in $(seq 1 20); do
                PANEL=$(panel_size)
                [ -n "$PANEL" ] && break
                sleep 0.15
            done
            [ -n "$PANEL" ] && ok "a click on a tile opened its panel ($PANEL)" \
                || fail "a click on a tile put nothing on screen"

            # And it closes on Esc, which is the half a GtkPopover over a
            # DOCK window does not reliably get.
            xdotool key Escape
            CLOSED=1
            for _ in $(seq 1 20); do
                [ -z "$(panel_size)" ] && break
                CLOSED=0
                sleep 0.15
            done
            [ -z "$(panel_size)" ] && ok "Esc closed the panel" \
                || fail "the panel ignored Esc, so it never had the keyboard"
            [ "$CLOSED" = "1" ] || true
        fi
    fi
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
