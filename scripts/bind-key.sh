#!/usr/bin/env bash
#
# Binds a key on your desktop to a PrimoDock action.
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
BUS=dev.oprimo.PrimoDock
OBJECT=/dev/oprimo/PrimoDock
INTERFACE=dev.oprimo.PrimoDock1

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
        "$SLOT_ROOT"/primodock-*/) ;;
        *) continue ;;
        esac
        mine=1
        printf '%s\t%s\n' \
            "$(gsettings get "$SLOT_SCHEMA:$slot" binding)" \
            "$(gsettings get "$SLOT_SCHEMA:$slot" name)"
    done
    [ "$mine" = "0" ] && echo "no PrimoDock keys bound"
    exit 0
fi

REMOVE=0
[ "${1:-}" = "--remove" ] && {
    REMOVE=1
    shift
}

KEY="${1:-<Super>e}"
ENVIRONMENT="${2:-}"

if [ -n "$ENVIRONMENT" ]; then
    SLOT="$SLOT_ROOT/primodock-$(slug "$ENVIRONMENT")/"
    LABEL="PrimoDock: $ENVIRONMENT"
    COMMAND="gdbus call --session --dest $BUS --object-path $OBJECT --method $INTERFACE.SetEnvironment \"$ENVIRONMENT\""
else
    SLOT="$SLOT_ROOT/primodock-cycle/"
    LABEL="PrimoDock: cycle environment"
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
