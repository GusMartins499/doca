#!/usr/bin/env bash
#
# Installs Doca for your user, in place of Plank, and starts it at every login.
#
#   ./scripts/install.sh               build, install, start Doca now
#   ./scripts/install.sh --no-build    install what is already built
#   ./scripts/install.sh --uninstall   remove Doca and bring Plank back
#
# Nothing goes outside your home and nothing needs sudo:
#
#   ~/.local/bin/                      docad, doca-shell, doca-prefs, doca-session
#   ~/.local/share/applications/       doca-prefs.desktop, for the app grid
#   ~/.config/autostart/doca.desktop   runs doca-session at login
#   ~/.config/autostart/plank.desktop  gets Hidden=true while Doca is installed
#
# Run it again after pulling to update: it rebuilds, reinstalls and restarts.
# Your config (~/.config/doca) and counters (~/.local/state/doca) are never
# touched, not even by --uninstall.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${DOCA_BIN_DIR:-$HOME/.local/bin}"
CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
AUTOSTART="$CONFIG_HOME/autostart"
APPS="$DATA_HOME/applications"
CONFIG="$CONFIG_HOME/doca/config.toml"
PLANK_ENTRY="$AUTOSTART/plank.desktop"
# The line written into Plank's entry is marked, so that --uninstall takes out
# exactly what this script put there and nothing you wrote yourself.
MARK="# doca-install: hidden while Doca is installed"
BINARIES=(docad doca-shell doca-prefs)

# On a desktop session, start things now; in a script or over ssh, only install.
live() { [ -n "${DISPLAY:-}" ] && [ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ]; }

remove_doca() {
    echo "stopping Doca"
    if [ -x "$BIN/doca-session" ]; then
        "$BIN/doca-session" --stop || true
    else
        pkill -x doca-prefs 2>/dev/null || true
        pkill -x doca-shell 2>/dev/null || true
        pkill -x docad 2>/dev/null || true
    fi

    echo "removing the installed files"
    rm -f "$AUTOSTART/doca.desktop" "$APPS/doca-prefs.desktop" "$BIN/doca-session"
    for binary in "${BINARIES[@]}"; do rm -f "$BIN/$binary"; done

    if [ -f "$PLANK_ENTRY" ] && grep -qxF "$MARK" "$PLANK_ENTRY"; then
        echo "letting Plank start at login again"
        # The mark and the line right after it, Hidden=true.
        sed -i "/^$MARK\$/{N;d}" "$PLANK_ENTRY"
    fi

    if live && command -v plank >/dev/null && ! pgrep -x plank >/dev/null; then
        echo "starting Plank"
        setsid plank >/dev/null 2>&1 </dev/null &
    fi
    echo
    echo "Doca is gone. Your config is still in ${CONFIG%/*}; delete it by"
    echo "hand if you do not mean to come back."
}

install_doca() {
    local build=1
    [ "${1:-}" = "--no-build" ] && build=0

    if [ "$build" = 1 ]; then
        echo "building (cargo build --release)"
        (cd "$ROOT" && cargo build --release)
    fi
    for binary in "${BINARIES[@]}"; do
        [ -x "$ROOT/target/release/$binary" ] || {
            echo "missing target/release/$binary — run without --no-build" >&2
            exit 1
        }
    done

    echo "installing into $BIN"
    mkdir -p "$BIN" "$APPS" "$AUTOSTART"
    # `install` writes a new file rather than over the old one, so a dock that
    # is running while this replaces it carries on until it is restarted.
    for binary in "${BINARIES[@]}"; do
        install -m755 "$ROOT/target/release/$binary" "$BIN/$binary"
    done
    install -m755 "$ROOT/scripts/doca-session" "$BIN/doca-session"
    install -m644 "$ROOT/packaging/doca-prefs.desktop" "$APPS/doca-prefs.desktop"

    cat >"$AUTOSTART/doca.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Doca
Comment=The dock
Exec=$BIN/doca-session
Icon=preferences-desktop
Terminal=false
NoDisplay=true
X-GNOME-Autostart-enabled=true
EOF

    if [ -f "$PLANK_ENTRY" ] && ! grep -qxF "$MARK" "$PLANK_ENTRY"; then
        echo "keeping Plank from starting at login"
        printf '%s\nHidden=true\n' "$MARK" >>"$PLANK_ENTRY"
    fi

    if [ ! -f "$CONFIG" ]; then
        echo "no config yet, seeding one from your Plank launchers: $CONFIG"
        mkdir -p "${CONFIG%/*}"
        cp "$ROOT/examples/config-from-plank.toml" "$CONFIG"
    fi

    if live; then
        echo "starting Doca"
        setsid "$BIN/doca-session" >/dev/null 2>&1 </dev/null &
    fi
    echo
    echo "Doca is installed and starts at every login."
    echo "Log: ${XDG_STATE_HOME:-$HOME/.local/state}/doca/session.log"
    echo "Back to Plank: $ROOT/scripts/install.sh --uninstall"
}

case "${1:-}" in
    --uninstall) remove_doca ;;
    ""|--no-build) install_doca "${1:-}" ;;
    -h|--help) sed -n '3,19p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' ;;
    *) echo "unknown option: $1 (try --help)" >&2; exit 2 ;;
esac
