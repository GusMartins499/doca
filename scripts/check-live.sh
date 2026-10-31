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

# "WxH+X+Y" of every window of the bar's on screen, besides the bar itself.
# Mapped, not merely there: the window that carries a dragged pin or tile is
# kept, hidden, between drags, and is not a panel or a menu.
shown_shell_windows() {
    local line id
    xwininfo -root -children 2>/dev/null | grep '"doca-shell":' | grep -v '"Doca":' |
        while read -r line; do
            id=${line%% *}
            xwininfo -id "$id" 2>/dev/null | grep -q 'IsViewable' || continue
            echo "$line" | grep -oE '[0-9]+x[0-9]+\+-?[0-9]+\+-?[0-9]+' | head -1
        done
}

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

# A pin that knows where: places counted from 1 in the order the bar shows,
# which for pins is the order they were pinned in. Studio holds [$APP].
read -r APP2 APP3 < <(call ListApplications | tr '(' '\n' \
    | sed -n "s/^'\([^']*\)'.*/\1/p" | grep -vxF "$APP" | head -2 | tr '\n' ' ')
studio_pins() {
    # ('Studio', @ai [], false, ['a', 'b'], @as []) — the pins are the third
    # field, the second list.
    call ListEnvironments | grep -o "('Studio', [^)]*)" \
        | sed -E "s/^\('Studio', [^]]*\], (true|false), (\[[^]]*\]).*/\2/"
}
if [ -n "${APP3:-}" ]; then
    call PinInAt "Studio" "$APP2" 1 >/dev/null
    call PinInAt "Studio" "$APP3" 2 >/dev/null
    case "$(studio_pins)" in
        "['$APP2', '$APP3', '$APP']") ok "a pin given a place lands there, first and in the middle" ;;
        *) fail "the places did not take: $(studio_pins)" ;;
    esac
    call PinInAt "Studio" "$APP3" 2 >/dev/null
    case "$(studio_pins)" in
        "['$APP2', '$APP3', '$APP']") ok "the same place twice is the same dock" ;;
        *) fail "a second identical pin changed something: $(studio_pins)" ;;
    esac
    call PinInAt "Studio" "$APP2" 3 >/dev/null
    case "$(studio_pins)" in
        "['$APP3', '$APP', '$APP2']") ok "a pin moved to the end is moved, not doubled" ;;
        *) fail "moving a pin went wrong: $(studio_pins)" ;;
    esac
    case "$(call PinInAt "Studio" "$APP" 9)" in
        *InvalidArgs*"from 1 to 3"*) ok "a place past the pins is refused in words" ;;
        *) fail "the refusal was not showable: $(call PinInAt "Studio" "$APP" 9)" ;;
    esac
    case "$(cat "$XDG_CONFIG_HOME/doca/config.toml")" in
        *"# Kept by hand. The daemon must not eat this."*) ok "the comment outlived the pins" ;;
        *) fail "a pin took the comment with it" ;;
    esac
    case "$(call PinIn "Studio" "$APP")" in
        *error*) fail "PinIn lost its old signature: $(call PinIn "Studio" "$APP")" ;;
        *) ok "PinIn still answers to the call it always did" ;;
    esac
else
    fail "fewer than three applications installed; PinInAt went unchecked"
fi
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
call SetEnvironmentWidgets "Work" "['water', 'note', 'clock']" >/dev/null
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

# The week the panel draws comes over the wire with the count, seven places
# whatever the history has in it — a machine that was shut on Tuesday still
# has a Tuesday in its week, and it is a zero rather than a missing bar.
case "$(call ListWidgets)" in
    *"[uint32 0, 0, 0, 0, 0, 0, 1000]"*)
        ok "the week arrives with seven places and today in the last one" ;;
    *) fail "the week did not come back: $(call ListWidgets)" ;;
esac

# Each widget says what shape its state takes, so a reader can have a drawer
# per shape instead of one column of labels for all of them. Checked on the
# wire because the union is spelled by hand — a name beside a variant — and a
# payload written one way and read another is a bar with no tiles on it.
case "$(call ListWidgets)" in
    *"('water', 'water', <(uint32"*) ok "a widget state names the shape it carries" ;;
    *) fail "the typed state did not come back as a name and a payload: $(call ListWidgets)" ;;
esac
case "$(call ListWidgets)" in
    # Two lines, a progress figure and a flag: the tile the widgets without a
    # variant of their own still draw. The clock, now — the note grew one of
    # its own when it became a post-it you can write on.
    *"'clock', 'simple', <("*", -1.0, false)>"*)
        ok "a widget with no variant of its own still sends one" ;;
    *) fail "the clock did not come back as a simple body: $(call ListWidgets)" ;;
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
    # Whole, newlines and all: the daemon stopped splitting it into a first
    # line and a rest when the note became something you edit in the panel.
    *"'milk\nand bread'"*) ok "a note written now is the note the bar shows" ;;
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
                shown_shell_windows | awk -F'x' '$1 >= 100 { print; exit }' | cut -d+ -f1
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

            # A tile dragged to a new place (#18, P3): the clock, last on the
            # bar, carried left past the water and let go. The order is read
            # back from the daemon, the last list in the dock's entry.
            work_widgets() {
                call ListEnvironments | grep -o "('Work', [^)]*)" \
                    | grep -oE "\[[^]]*\]\)$" | tr -d ')'
            }
            xdotool mousemove "$AT_X" "$AT_Y"; sleep 0.3
            xdotool mousedown 1; sleep 0.1
            for step in 4 12 30 60 100 150 200 260 320; do
                xdotool mousemove $((AT_X - step)) "$AT_Y"; sleep 0.05
            done
            sleep 0.4
            xdotool mouseup 1; sleep 1
            case "$(work_widgets)" in
                "['clock', 'water']") ok "a tile dragged past its neighbour took its place" ;;
                *) fail "after dragging the clock the tiles are $(work_widgets)" ;;
            esac
            [ -z "$(panel_size)" ] \
                && ok "a tile that was dragged did not open its panel as well" \
                || { fail "letting go of a dragged tile opened its panel"; xdotool key Escape; }
            call SetEnvironmentWidgets "Work" "['water', 'clock']" >/dev/null
            sleep 0.5
        fi
    fi
else
    fail "the bar never drew its widgets"
    tail -5 "$WORK/shell.log"
fi
# A file dragged from another app to a hidden bar (#18, item 12). While a
# drag has the pointer the dock gets no enter or leave, so auto-hide never
# heard it: the bar stayed hidden under a file brought to its edge.
bar_y() {
    local id
    id=$(xdotool search --name '^Doca$' 2>/dev/null | head -1)
    [ -n "$id" ] && xwininfo -id "$id" | awk '/Absolute upper-left Y/ {print $4}'
}
if command -v xdotool >/dev/null && python3 -c 'import gi; gi.require_version("Gtk", "3.0")' 2>/dev/null; then
    gdbus call --session --dest "$BUS" --object-path "$OBJECT" \
        --method "$INTERFACE.SetAppearance" "{'auto_hide': <true>}" >/dev/null 2>&1
    for _ in $(seq 1 30); do [ "$(bar_y)" -ge 790 ] 2>/dev/null && break; sleep 0.1; done
    HIDDEN_Y=$(bar_y)
    echo dragged > "$WORK/dragged.txt"
    cat > "$WORK/drag_source.py" <<'PY'
import sys, gi
gi.require_version('Gtk', '3.0')
from gi.repository import Gtk, Gdk, GLib
uri = GLib.filename_to_uri(sys.argv[1], None)
w = Gtk.Window(title="drag source")
w.move(500, 150); w.set_default_size(240, 160)
b = Gtk.Button(label="drag me")
b.drag_source_set(Gdk.ModifierType.BUTTON1_MASK, [Gtk.TargetEntry.new("text/uri-list", 0, 0)], Gdk.DragAction.COPY)
b.connect("drag-data-get", lambda w, c, data, i, t: data.set_uris([uri]))
w.add(b); w.show_all()
GLib.timeout_add(20000, Gtk.main_quit)
Gtk.main()
PY
    python3 -I "$WORK/drag_source.py" "$WORK/dragged.txt" &
    SOURCE=$!
    sleep 2
    xdotool mousemove 620 230; sleep 0.3; xdotool mousedown 1; sleep 0.2
    for y in 260 320 420 520 620 720 780 799; do xdotool mousemove 630 "$y"; sleep 0.08; done
    for _ in 1 2 3 4 5; do xdotool mousemove 632 799; sleep 0.1; xdotool mousemove 630 799; sleep 0.1; done
    sleep 0.5
    DRAG_Y=$(bar_y)
    for y in 600 400 230; do xdotool mousemove 630 "$y"; sleep 0.1; done
    xdotool mouseup 1
    sleep 1.5
    AFTER_Y=$(bar_y)
    kill "$SOURCE" 2>/dev/null
    [ -n "$DRAG_Y" ] && [ -n "$HIDDEN_Y" ] && [ "$DRAG_Y" -lt "$HIDDEN_Y" ] \
        && ok "a file dragged to the hidden bar's edge brought it up (y $HIDDEN_Y → $DRAG_Y)" \
        || fail "a file dragged to the hidden bar's edge left it hidden (y $HIDDEN_Y → $DRAG_Y)"
    [ "$AFTER_Y" = "$HIDDEN_Y" ] \
        && ok "the drag went elsewhere and the bar went away again" \
        || fail "after the drag left, the bar stayed at y $AFTER_Y"
    gdbus call --session --dest "$BUS" --object-path "$OBJECT" \
        --method "$INTERFACE.SetAppearance" "{'auto_hide': <false>}" >/dev/null 2>&1
else
    echo "  skip  xdotool or python3-gi missing: a drag to the bar is not checked"
fi

# The dock with the keyboard on it (#18, item 9). The bar is a DOCK window, so
# whether real keys reach it at all is the whole question, and only real key
# presses on a real X server answer it. On the bar: the folder, then the
# water and clock tiles. Where the keys went is read out of the bar's log.
keyboard_on() {
    grep -a "the keyboard is on" "$WORK/shell.log" | tail -1 | sed 's/\x1b\[[0-9;]*m//g'
}
held_since() {
    tail -c +"$1" "$WORK/shell.log" | grep -ac "the dock has the keyboard"
}
some_menu_up() {
    shown_shell_windows | awk -F'x' '$1 >= 100 { print; exit }'
}
if command -v xdotool >/dev/null && python3 -c 'import gi; gi.require_version("Gtk", "3.0")' 2>/dev/null; then
    call SetAppearance "{'auto_hide': <true>}" >/dev/null
    xdotool mousemove 100 100
    # The drag check above has just switched auto-hide off, and that rebuild
    # may still be on its way: settled first, so it cannot pass for this one.
    sleep 1.5
    for _ in $(seq 1 30); do [ "$(bar_y)" -ge 790 ] 2>/dev/null && break; sleep 0.1; done
    HIDDEN_Y=$(bar_y)

    MARK=$(( $(stat -c %s "$WORK/shell.log") + 1 ))
    call FocusDock >/dev/null
    for _ in $(seq 1 30); do [ "$(held_since "$MARK")" -ge 1 ] && break; sleep 0.1; done
    sleep 0.4
    UP_Y=$(bar_y)
    [ -n "$UP_Y" ] && [ "$UP_Y" -lt "$HIDDEN_Y" ] \
        && ok "FocusDock brought a hidden bar up (y $HIDDEN_Y → $UP_Y)" \
        || fail "FocusDock left the hidden bar where it was (y $HIDDEN_Y → $UP_Y)"
    case "$(keyboard_on)" in
        *'Icon("folder:'*) ok "the first item has the keyboard" ;;
        *) fail "the keyboard did not land on the first item: $(keyboard_on)" ;;
    esac
    # Longer than auto-hide waits for a pointer that left, with the pointer
    # nowhere near: held is held.
    sleep 1
    [ "$(bar_y)" = "$UP_Y" ] && ok "the bar stays up while the keyboard is on it" \
        || fail "the bar hid with the keyboard on it (y $(bar_y))"

    xdotool key Right
    sleep 0.3
    case "$(keyboard_on)" in
        *'Tile("water")'*) ok "a real Right reached the dock and walked onto the tile" ;;
        *) fail "Right did not move the keyboard: $(keyboard_on)" ;;
    esac
    xdotool key Return
    PANEL=""
    for _ in $(seq 1 20); do PANEL=$(some_menu_up); [ -n "$PANEL" ] && break; sleep 0.15; done
    [ -n "$PANEL" ] && ok "Enter on a tile opened its panel ($PANEL)" \
        || fail "Enter on a tile opened nothing"
    xdotool key Escape
    for _ in $(seq 1 20); do [ -z "$(some_menu_up)" ] && break; sleep 0.15; done
    sleep 0.6
    xdotool key Right
    sleep 0.3
    case "$(keyboard_on)" in
        *'Tile("clock")'*) ok "Esc out of the panel came back to the bar, keys and all" ;;
        *) fail "after Esc out of the panel the keys went nowhere: $(keyboard_on)" ;;
    esac
    [ "$(bar_y)" = "$UP_Y" ] && ok "the bar stayed up under the panel and after it" \
        || fail "the panel closing took the bar away from the keyboard (y $(bar_y))"

    xdotool key Escape
    for _ in $(seq 1 30); do [ "$(bar_y)" -ge "$HIDDEN_Y" ] 2>/dev/null && break; sleep 0.1; done
    [ "$(bar_y)" = "$HIDDEN_Y" ] && ok "Esc gave the keyboard back and the bar hid again" \
        || fail "after Esc the bar stayed at y $(bar_y)"

    # A key bound on the desktop holds the keyboard from its press to its
    # release, so FocusDock usually lands while somebody else still has it.
    # Another client holds a grab across the call and lets go a moment later.
    cat > "$WORK/hold_keys.py" <<'PY'
import gi
gi.require_version('Gtk', '3.0'); gi.require_version('Gdk', '3.0')
from gi.repository import Gtk, Gdk, GLib
w = Gtk.Window(title="holding the keys"); w.move(10, 10); w.set_default_size(40, 40); w.show_all()
def grab():
    s = Gdk.Display.get_default().get_default_seat()
    print(s.grab(w.get_window(), Gdk.SeatCapabilities.KEYBOARD, False, None, None, None, None), flush=True)
    GLib.timeout_add(400, lambda: (s.ungrab(), Gtk.main_quit()))
GLib.timeout_add(300, grab)
Gtk.main()
PY
    python3 -I "$WORK/hold_keys.py" > "$WORK/hold_keys.log" 2>/dev/null &
    HOLDER=$!
    for _ in $(seq 1 30); do grep -q SUCCESS "$WORK/hold_keys.log" && break; sleep 0.05; done
    MARK=$(( $(stat -c %s "$WORK/shell.log") + 1 ))
    call FocusDock >/dev/null
    wait "$HOLDER"
    sleep 0.4
    xdotool key Right
    sleep 0.3
    if ! grep -q SUCCESS "$WORK/hold_keys.log"; then
        echo "  skip  the other client never got its grab, so there was nothing to wait out"
    elif ! tail -c +"$MARK" "$WORK/shell.log" | grep -aq "the keyboard is someone else's"; then
        echo "  skip  the keyboard was free again before FocusDock asked, so nothing was waited out"
    elif [ "$(held_since "$MARK")" -ge 1 ]; then
        case "$(keyboard_on)" in
            *'Tile("water")'*) ok "the dock waited out a keyboard someone else held, then took it" ;;
            *) fail "with the keyboard held elsewhere at the call, the dock never got it: $(keyboard_on)" ;;
        esac
    else
        fail "FocusDock went unanswered while the keyboard was held elsewhere"
    fi
    xdotool key Escape
    sleep 0.3
    call SetAppearance "{'auto_hide': <false>}" >/dev/null
else
    echo "  skip  xdotool or python3-gi missing: the keyboard on the dock is not checked"
fi

# A hidden bar is reached from more than what shows of it (#18, item 12). The
# strip is a second window, so this is the one place that sees it take a real
# pointer: five pixels up, over the two that show, and in the air beside the
# bar the window spreads into, where a bar that came up and could not hold the
# pointer would go and come back for as long as it rested there.
if command -v xdotool >/dev/null; then
    call SetAppearance "{'auto_hide': <true>}" >/dev/null
    xdotool mousemove 100 100
    sleep 1.5
    for _ in $(seq 1 30); do [ "$(bar_y)" -ge 790 ] 2>/dev/null && break; sleep 0.1; done
    HIDDEN_Y=$(bar_y)

    xdotool mousemove 640 790
    sleep 0.6
    [ "$(bar_y)" = "$HIDDEN_Y" ] && ok "above the edge the hidden bar stays hidden" \
        || fail "the bar came up for a pointer above the edge (y $HIDDEN_Y → $(bar_y))"

    xdotool mousemove 640 795
    sleep 0.6
    UP_Y=$(bar_y)
    [ -n "$UP_Y" ] && [ "$UP_Y" -lt "$HIDDEN_Y" ] \
        && ok "a pointer five pixels up the edge brought the bar up (y $HIDDEN_Y → $UP_Y)" \
        || fail "the edge did not bring the bar up (y $HIDDEN_Y → $UP_Y)"
    sleep 1
    [ "$(bar_y)" = "$UP_Y" ] && ok "the bar stays up under a pointer that came from the edge" \
        || fail "the bar left a pointer that came from the edge (y $(bar_y))"

    xdotool mousemove 100 100
    sleep 1
    for _ in $(seq 1 30); do [ "$(bar_y)" -ge 790 ] 2>/dev/null && break; sleep 0.1; done
    [ "$(bar_y)" = "$HIDDEN_Y" ] && ok "and it goes when the pointer does" \
        || fail "the bar stayed up after the pointer left (y $(bar_y))"
    call SetAppearance "{'auto_hide': <false>}" >/dev/null
    sleep 0.5
else
    echo "  skip  xdotool is not installed, so the edge is not checked"
fi

# The air over a resting bar is the screen's. The window is the bar plus the
# room a magnified icon rises and spreads into, drawn transparent — and until
# the window was given an input shape, a click there went to the dock and did
# nothing, however plainly it was aimed at the window underneath. `xev` is
# that window: if it hears the press, the press went through.
if command -v xdotool >/dev/null && command -v xev >/dev/null; then
    xev -geometry 1280x800+0+0 -event button > "$WORK/behind.log" 2>&1 &
    BEHIND=$!
    sleep 0.8
    DOCK=$(xdotool search --name '^Doca$' 2>/dev/null | head -1)
    xdotool windowraise "$DOCK"
    xdotool mousemove 20 20
    sleep 0.6
    eval "$(xwininfo -id "$DOCK" | awk '/Absolute upper-left X/ {print "DX="$4} /Absolute upper-left Y/ {print "DY="$4} /Width/ {print "DW="$2} /Height/ {print "DH="$2}')"
    heard_behind() {
        : > "$WORK/behind.log"
        xdotool mousemove "$1" "$2"; sleep 0.3
        xdotool mousedown 1; xdotool mouseup 1; sleep 0.3
        grep -q ButtonPress "$WORK/behind.log"
    }
    heard_behind $((DX + 4)) $((DY + 4)) \
        && ok "a click in the air over the resting bar reached the window behind it" \
        || fail "a click in the air over the resting bar was taken by the dock"
    # With the pointer on the bar, the lens opens into that air, and the
    # pointer has to be able to stay on what it draws there.
    xdotool mousemove $((DX + DW / 2)) $((DY + DH - 3)); sleep 0.4
    heard_behind $((DX + 4)) $((DY + 4)) \
        && fail "with the pointer on the bar, the air its lens opens into let the pointer fall through" \
        || ok "with the pointer on the bar, the air over it is the dock's"
    xdotool mousemove 20 20; sleep 0.4
    heard_behind $((DX + 4)) $((DY + 4)) \
        && ok "the pointer gone, the air is the screen's again" \
        || fail "the air stayed the dock's after the pointer left"
    kill "$BEHIND" 2>/dev/null
else
    echo "  skip  xdotool or xev missing: what the air over the bar takes is not checked"
fi

# The menu on the bar (#18, P3). Where the bar starts is read from its strut,
# because the padding at its end is the one place that is neither an icon nor
# a tile. Real keys choose in the menu, and the daemon says if it landed.
if command -v xdotool >/dev/null && command -v xprop >/dev/null; then
    DOCK=$(xdotool search --name '^Doca$' 2>/dev/null | head -1)
    STRUT=$(xprop -id "$DOCK" _NET_WM_STRUT_PARTIAL | sed 's/.*= //; s/,//g')
    BAR_LEFT=$(echo "$STRUT" | awk '{print $11}')
    BAR_Y=$(( $(bar_y) + $(xwininfo -id "$DOCK" | awk '/Height/ {print $2}') - 20 ))
    xdotool mousemove $((BAR_LEFT + 3)) "$BAR_Y"
    sleep 0.4
    xdotool click 3
    sleep 0.5
    if [ -n "$(some_menu_up)" ]; then
        ok "a right click on the empty bar offered its menu"
    else
        fail "a right click on the empty bar offered nothing"
    fi
    xdotool key Down Down Down Return
    sleep 0.6
    # Toggled back: it answers with what it is now, so false means it was on.
    [ "$(call ToggleAutoHide)" = "(false,)" ] \
        && ok "choosing in the menu reached the daemon" \
        || fail "the auto-hide chosen in the menu never reached the daemon"
    xdotool mousemove 20 20
    sleep 0.6
else
    echo "  skip  xdotool or xprop missing: the menu on the bar is not checked"
fi

# A pin dragged to a new place on the bar (#18, P3). Two pins and the folder,
# no tiles and no lens, so where each icon is can be worked out from the
# window: the bar is centred in it, three slots wide, and the second pin is
# its middle. The order is read back from the daemon, which is where the drop
# has to have landed — the bar showing it is not enough.
# How many icon-sized windows of the bar's are on screen. Mapped, not merely
# there: the window that carries a pin is kept and hidden between drags.
carried_in_the_air() {
    local id seen=0
    for id in $(xwininfo -root -children 2>/dev/null | grep '"doca-shell":' | grep ' 64x64+' | awk '{print $1}'); do
        xwininfo -id "$id" 2>/dev/null | grep -q 'IsViewable' && seen=$((seen + 1))
    done
    echo "$seen"
}
work_pins() {
    call ListEnvironments | grep -o "('Work', [^)]*)" \
        | sed -E "s/^\('Work', [^]]*\], (true|false), (\[[^]]*\]).*/\2/"
}
if command -v xdotool >/dev/null && [ -n "${APP3:-}" ]; then
    call SetEnvironmentWidgets "Work" "[]" >/dev/null
    call SetAppearance "{'magnification': <1.0>, 'icon_size': <int32 64>}" >/dev/null
    call PinInAt "Work" "$APP2" 1 >/dev/null
    call PinInAt "Work" "$APP3" 2 >/dev/null
    sleep 1.5
    DOCK=$(xdotool search --name '^Doca$' 2>/dev/null | head -1)
    eval "$(xwininfo -id "$DOCK" | awk '/Absolute upper-left X/ {print "DX="$4} /Absolute upper-left Y/ {print "DY="$4} /Width/ {print "DW="$2} /Height/ {print "DH="$2}')"
    PITCH=$((64 + 2 * 2 + 2))
    MIDDLE=$((DX + DW / 2))
    ICON_Y=$((DY + DH - 9 - 32))
    FIRST=$((MIDDLE - PITCH))
    case "$(work_pins)" in
        "['$APP2', '$APP3']") ;;
        *) fail "the two pins to drag did not land: $(work_pins)" ;;
    esac
    xdotool mousemove "$FIRST" "$ICON_Y"; sleep 0.3
    xdotool mousedown 1; sleep 0.1
    for step in 4 12 24 40 56 72 88; do
        xdotool mousemove $((FIRST + step)) "$ICON_Y"; sleep 0.05
    done
    sleep 0.2
    xdotool mouseup 1; sleep 1
    case "$(work_pins)" in
        "['$APP3', '$APP2']") ok "the first pin dragged past the second took its place" ;;
        *) fail "after the drag the pins are $(work_pins)" ;;
    esac
    grep -aq "an icon was dragged to a new place" "$WORK/shell.log" \
        && ok "the bar said where the icon was dropped" \
        || fail "the bar never logged the drop"
    # And a press that does not travel is still a click: the folder opens.
    xdotool mousemove $((MIDDLE + PITCH)) "$ICON_Y"; sleep 0.3
    xdotool mousedown 1; sleep 0.05; xdotool mouseup 1; sleep 0.8
    if [ -n "$(some_menu_up)" ]; then
        ok "a press that did not travel is still a click: the folder opened ($(some_menu_up))"
    else
        fail "a click on the folder, after the drag code, opened nothing"
    fi
    xdotool key Escape; sleep 0.3
    # Pulled up off the bar and let go of, a pin is unpinned. While it is in
    # the air it is carried by a window of its own, the icon's size — the
    # bar's window ends a little above the bar and would cut it off.
    xdotool mousemove "$FIRST" "$ICON_Y"; sleep 0.3
    xdotool mousedown 1; sleep 0.1
    for step in 6 16 40 80 120 160; do
        xdotool mousemove "$FIRST" $((ICON_Y - step)); sleep 0.05
    done
    sleep 0.3
    CARRIED=$(carried_in_the_air)
    xdotool mouseup 1; sleep 1
    [ "$CARRIED" -ge 1 ] \
        && ok "a pin pulled off the bar was carried at the pointer" \
        || fail "nothing carried the pin once it was off the bar"
    case "$(work_pins)" in
        "['$APP2']") ok "a pin let go of off the bar was unpinned, and only that one" ;;
        *) fail "after letting go off the bar the pins are $(work_pins)" ;;
    esac
    grep -aq "a pin was pulled off the bar and let go of" "$WORK/shell.log" \
        && ok "the bar said it let the pin go" \
        || fail "the bar never logged the pin it let go of"
    [ "$(carried_in_the_air)" = 0 ] \
        && ok "the carried icon was put down with the pin" \
        || fail "the carried icon was left in the air"
    call UnpinIn "Work" "$APP3" >/dev/null 2>&1
    call UnpinIn "Work" "$APP2" >/dev/null 2>&1
    call SetEnvironmentWidgets "Work" "['water', 'clock']" >/dev/null
    call SetAppearance "{'magnification': <1.6>}" >/dev/null
    sleep 0.5
else
    echo "  skip  xdotool missing or too few apps installed: a drag on the bar is not checked"
fi

# The monitor the dock was given, by name (#18, item 11). Xvfb has one output,
# `screen`, and lists no monitor `xrandr --setmonitor` adds, so moving between
# two is the unit tests' (`screen::choose`); what is checked here is the way
# there — the name going from the bus through GDK to where the bar stands —
# and a name nothing answers to. Before the screen is resized below: an Xvfb
# whose framebuffer `xrandr --fb` changed has no output left that GDK can
# name, and the one it makes up in its place is called nothing at all.
bar_geometry() {
    xwininfo -root -children 2>/dev/null | grep '"Doca":' | head -1 |
        grep -o '[0-9]*x[0-9]*+-\?[0-9]*+-\?[0-9]*' | head -1
}
missing_said() {
    grep -ac "is not connected" "$WORK/shell.log"
}
STOOD=$(bar_geometry)
call SetAppearance "{'monitor': <'screen'>}" >/dev/null
sleep 1
[ "$(bar_geometry)" = "$STOOD" ] && [ "$(missing_said)" = "0" ] \
    && ok "the monitor named by its output is the one the bar stands on ($STOOD)" \
    || fail "naming the monitor the bar was on moved it to $(bar_geometry), or called it missing"
call SetAppearance "{'monitor': <'HDMI-9'>}" >/dev/null
sleep 1
call SetAppearance "{'icon_size': <int32 60>}" >/dev/null
sleep 1
WHERE=$(bar_geometry)
WHERE_Y=${WHERE##*+}; WHERE_H=${WHERE#*x}; WHERE_H=${WHERE_H%%+*}
[ -n "$WHERE" ] && [ $((WHERE_Y + WHERE_H)) -eq 800 ] \
    && ok "a monitor that is not connected leaves the bar on the primary ($WHERE)" \
    || fail "with its monitor unplugged the bar went to $WHERE"
[ "$(missing_said)" = "1" ] \
    && ok "the missing monitor was said once, not on every rebuild" \
    || fail "the missing monitor was logged $(missing_said) times over two rebuilds"
call SetAppearance "{'monitor': <''>, 'icon_size': <int32 64>}" >/dev/null
sleep 1
grep -q '^monitor = ""' "$XDG_CONFIG_HOME/doca/config.toml" \
    && ok "the primary again is the empty name in the file" \
    || fail "the file says $(grep '^monitor' "$XDG_CONFIG_HOME/doca/config.toml")"
# The screen changing under a running bar (#18, item 11): a monitor swapped,
# a resolution switched. The bar has to land on the new bottom edge, inside
# the new width — before this it stayed where the old screen had it, which
# on a smaller one is entirely off it.
if command -v xrandr >/dev/null; then
    xrandr --fb 1024x640 >/dev/null 2>&1
    FOLLOWED=0
    for _ in $(seq 1 30); do
        # "WxH+X+Y" of the bar's window, as the X server has it.
        SHAPE=$(xwininfo -root -children 2>/dev/null | grep '"Doca":' | head -1 |
            grep -o '[0-9]*x[0-9]*+-\?[0-9]*+-\?[0-9]*' | head -1)
        W=${SHAPE%%x*}; REST=${SHAPE#*x}; H=${REST%%+*}; Y=${SHAPE##*+}
        if [ -n "$SHAPE" ] && [ $((Y + H)) -eq 640 ] && [ "$W" -le 1024 ]; then
            FOLLOWED=1
            break
        fi
        sleep 0.1
    done
    [ "$FOLLOWED" = "1" ] && ok "the bar moved to a screen that changed size under it ($SHAPE)" \
        || fail "the screen shrank to 1024x640 and the bar stayed at $SHAPE"
    xrandr --fb 1280x800 >/dev/null 2>&1
else
    echo "  skip  xrandr not installed: the screen changing is not checked"
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
