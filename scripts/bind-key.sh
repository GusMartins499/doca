#!/usr/bin/env bash
#
# Binds a key on your desktop to a Doca action.
#
# The dock does not grab keys. On Linux the keyboard belongs to the desktop, so
# the dock exposes its actions on D-Bus and GNOME is told which key calls them.
# This script writes that GNOME custom keybinding for you.
#
#   ./scripts/bind-key.sh                    <Super>e cycles environment
#   ./scripts/bind-key.sh '<Super>x'         the same action, your key
#   ./scripts/bind-key.sh '<Super>1' Work    one key straight to one environment
#   ./scripts/bind-key.sh --list             what is bound now
#   ./scripts/bind-key.sh --remove '<Super>1' Work
#   ./scripts/bind-key.sh --item 3           <Super>3 chooses the third icon
#   ./scripts/bind-key.sh --item 3 '<Alt>3'  the same, your key
#   ./scripts/bind-key.sh --items            <Super>1 to <Super>9, all nine
#   ./scripts/bind-key.sh --remove --items   and gone again
#
# GNOME binds <Super>1..9 itself, to its own dash (switch-to-application-N).
# Two bindings on one key is a coin toss, so --item and --items refuse a key
# GNOME already holds and say which; add --take to clear GNOME's binding for
# exactly those keys. Nothing else of GNOME's is touched.
#
# Running it twice with the same action changes that one binding rather than
# adding a second: each action owns a named slot. Keybindings you set yourself
# are read, kept and written back — the list GNOME keeps is shared by every
# application, and replacing it outright would drop everyone else's.
#
set -uo pipefail

MEDIA_KEYS=org.gnome.settings-daemon.plugins.media-keys
SLOT_SCHEMA="$MEDIA_KEYS.custom-keybinding"
SLOT_ROOT=/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings
BUS=io.github.gusmartins499.Doca
OBJECT=/io/github/gusmartins499/Doca
INTERFACE=io.github.gusmartins499.Doca1

command -v gsettings >/dev/null || {
    echo "gsettings not found — this script configures GNOME, and needs it" >&2
    exit 1
}

# The list of slots GNOME currently knows about, as plain paths.
read_slots() {
    local raw entry
    raw=$(gsettings get "$MEDIA_KEYS" custom-keybindings)
    SLOTS=()
    [ "$raw" = "@as []" ] && return
    raw=${raw#\[}
    raw=${raw%\]}
    local IFS=,
    for entry in $raw; do
        entry=${entry#"${entry%%[![:space:]]*}"}
        entry=${entry%"${entry##*[![:space:]]}"}
        entry=${entry#\'}
        entry=${entry%\'}
        [ -n "$entry" ] && SLOTS+=("$entry")
    done
}

write_slots() {
    local joined="" slot
    for slot in "${SLOTS[@]:-}"; do
        [ -n "$slot" ] || continue
        joined+="${joined:+, }'$slot'"
    done
    gsettings set "$MEDIA_KEYS" custom-keybindings "[$joined]"
}

has_slot() {
    local wanted=$1 slot
    for slot in "${SLOTS[@]:-}"; do
        [ "$slot" = "$wanted" ] && return 0
    done
    return 1
}

drop_slot() {
    local unwanted=$1 slot kept=()
    for slot in "${SLOTS[@]:-}"; do
        [ -n "$slot" ] || continue
        [ "$slot" = "$unwanted" ] && continue
        kept+=("$slot")
    done
    SLOTS=("${kept[@]:-}")
}

slug() {
    echo "$1" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9]\+/-/g; s/^-//; s/-$//'
}

# The environments the running daemon knows, one per line. Empty when it is not
# running, which is not an error: a key can be bound before the dock starts.
known_environments() {
    command -v gdbus >/dev/null || return 0
    gdbus call --session --dest "$BUS" --object-path "$OBJECT" \
        --method "$INTERFACE.ListEnvironments" 2>/dev/null |
        grep -o "('[^']*'," | sed "s/('//; s/',//"
}

if [ "${1:-}" = "--list" ]; then
    read_slots
    mine=0
    for slot in "${SLOTS[@]:-}"; do
        case "$slot" in
        "$SLOT_ROOT"/doca-*/) ;;
        *) continue ;;
        esac
        mine=1
        printf '%s\t%s\n' \
            "$(gsettings get "$SLOT_SCHEMA:$slot" binding)" \
            "$(gsettings get "$SLOT_SCHEMA:$slot" name)"
    done
    [ "$mine" = "0" ] && echo "no Doca keys bound"
    exit 0
fi

REMOVE=0
TAKE=0
ITEM=""
ALL_ITEMS=0
ARGS=()
for arg in "$@"; do
    case "$arg" in
    --remove) REMOVE=1 ;;
    --take) TAKE=1 ;;
    --items) ALL_ITEMS=1 ;;
    --item) ITEM=next ;;
    *)
        if [ "$ITEM" = "next" ]; then
            ITEM=$arg
        else
            ARGS+=("$arg")
        fi
        ;;
    esac
done
[ "$ITEM" = "next" ] && {
    echo "--item needs a position, 1 to 9" >&2
    exit 1
}

# GNOME's own binding for the n-th app in its dash, which <Super>n is out of
# the box. Printed as GNOME stores it.
GNOME_KEYS=org.gnome.shell.keybindings
gnome_holds() {
    local position=$1 key=$2 held
    held=$(gsettings get "$GNOME_KEYS" "switch-to-application-$position" 2>/dev/null) || return 1
    grep -qF "'$key'" <<<"$held"
}

# One position: its slot, label, command and key.
item_binding() {
    local position=$1
    SLOT="$SLOT_ROOT/doca-item-$position/"
    LABEL="Doca: item $position"
    COMMAND="gdbus call --session --dest $BUS --object-path $OBJECT --method $INTERFACE.ActivatePosition $position"
}

if [ -n "$ITEM" ] || [ "$ALL_ITEMS" = "1" ]; then
    if [ "$ALL_ITEMS" = "1" ]; then
        POSITIONS=(1 2 3 4 5 6 7 8 9)
    else
        case "$ITEM" in
        [1-9]) POSITIONS=("$ITEM") ;;
        *)
            echo "--item takes a position from 1 to 9, not $ITEM" >&2
            exit 1
            ;;
        esac
    fi
    read_slots

    if [ "$REMOVE" = "1" ]; then
        for position in "${POSITIONS[@]}"; do
            item_binding "$position"
            has_slot "$SLOT" && drop_slot "$SLOT" && echo "unbound $LABEL"
        done
        write_slots
        echo "GNOME's own <Super>1..9, if --take cleared them: gsettings reset $GNOME_KEYS switch-to-application-N"
        exit 0
    fi

    # Every key checked before any is written, so a refusal leaves nothing
    # half bound.
    taken=()
    for position in "${POSITIONS[@]}"; do
        key="${ARGS[0]:-<Super>$position}"
        [ "${#POSITIONS[@]}" -gt 1 ] && key="<Super>$position"
        gnome_holds "$position" "$key" && taken+=("$position:$key")
    done
    if [ "${#taken[@]}" -gt 0 ] && [ "$TAKE" = "0" ]; then
        echo "GNOME already binds these to its own dash:" >&2
        for pair in "${taken[@]}"; do
            echo "  ${pair#*:}  ($GNOME_KEYS switch-to-application-${pair%%:*})" >&2
        done
        echo "Run again with --take to give them to Doca, or pick other keys." >&2
        exit 1
    fi
    for pair in "${taken[@]:-}"; do
        [ -n "$pair" ] || continue
        gsettings set "$GNOME_KEYS" "switch-to-application-${pair%%:*}" "[]"
        echo "took ${pair#*:} from GNOME's switch-to-application-${pair%%:*}"
    done

    for position in "${POSITIONS[@]}"; do
        item_binding "$position"
        key="${ARGS[0]:-<Super>$position}"
        [ "${#POSITIONS[@]}" -gt 1 ] && key="<Super>$position"
        gsettings set "$SLOT_SCHEMA:$SLOT" name "$LABEL"
        gsettings set "$SLOT_SCHEMA:$SLOT" binding "$key"
        gsettings set "$SLOT_SCHEMA:$SLOT" command "$COMMAND"
        has_slot "$SLOT" || SLOTS+=("$SLOT")
        echo "$key → $LABEL"
    done
    write_slots
    exit 0
fi

set -- "${ARGS[@]:-}"
[ "${1:-}" = "" ] && set --

KEY="${1:-<Super>e}"
ENVIRONMENT="${2:-}"

if [ -n "$ENVIRONMENT" ]; then
    SLOT="$SLOT_ROOT/doca-$(slug "$ENVIRONMENT")/"
    LABEL="Doca: $ENVIRONMENT"
    COMMAND="gdbus call --session --dest $BUS --object-path $OBJECT --method $INTERFACE.SetEnvironment \"$ENVIRONMENT\""
else
    SLOT="$SLOT_ROOT/doca-cycle/"
    LABEL="Doca: cycle environment"
    COMMAND="gdbus call --session --dest $BUS --object-path $OBJECT --method $INTERFACE.CycleEnvironment"
fi

read_slots

if [ "$REMOVE" = "1" ]; then
    has_slot "$SLOT" || {
        echo "nothing bound for $LABEL"
        exit 0
    }
    drop_slot "$SLOT"
    write_slots
    echo "unbound $LABEL"
    exit 0
fi

if [ -n "$ENVIRONMENT" ]; then
    environments=$(known_environments)
    if [ -n "$environments" ] && ! grep -qxF "$ENVIRONMENT" <<<"$environments"; then
        echo "no environment named $ENVIRONMENT. The dock knows:" >&2
        sed 's/^/  /' <<<"$environments" >&2
        exit 1
    fi
fi

gsettings set "$SLOT_SCHEMA:$SLOT" name "$LABEL"
gsettings set "$SLOT_SCHEMA:$SLOT" binding "$KEY"
gsettings set "$SLOT_SCHEMA:$SLOT" command "$COMMAND"

if ! has_slot "$SLOT"; then
    SLOTS+=("$SLOT")
    write_slots
fi

echo "$KEY → $LABEL"
[ "${#SLOTS[@]}" -gt 1 ] && echo "${#SLOTS[@]} custom keybindings on this desktop, all kept"
exit 0
