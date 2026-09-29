# PrimoDock Linux

A native dock for Linux with per-environment profiles and live widgets.
No Electron, no webview.

Port of [PrimoDock](https://dock.oprimo.dev) (macOS) to Linux.

## Status

**Phase 2 — environments.** Each environment owns a set of workspaces and its
own pinned apps. Switching workspace switches the dock: its apps, and only the
windows living on its workspaces. Pinning applies to the environment you are
in, not globally.

See [PLANO.md](PLANO.md) for the full technical plan.

## Target

| | |
|---|---|
| Session | X11 (Wayland requires a different shell layer — see plan §3) |
| Desktop | GNOME Shell 42+ |
| Toolkit | GTK3 (deliberately not GTK4 — see plan §5) |

## Layout

```
crates/primodock-ipc     shared D-Bus contract
crates/primodockd        the daemon: windows, workspaces, state
crates/primodock-shell   the bar: draws, anchors, never talks X11 policy
```

The D-Bus boundary between the two is load-bearing: when the X11 session goes
away, the shell is replaced and the daemon survives intact.

## Build

```bash
sudo apt-get install -y pkg-config libgtk-3-dev
cargo build --release
```

## Run

Always in a nested X server, never in the session you are working in:

```bash
sudo apt-get install -y xserver-xephyr openbox dbus   # once
cargo build
./scripts/dev-session.sh
```

A dock reserves screen edge space through `_NET_WM_STRUT_PARTIAL`, which
changes the desktop work area for **every** dock and panel on that display.
Run this bar on your own session and whatever dock you already use will
repaint over it, or fail to repaint at all.

`dev-session.sh` gives it a display of its own, a session bus of its own, and
a config directory of its own. All three matter. The nested display is not
enough by itself: launching an app goes through the session bus, and
single-instance apps like gedit are D-Bus activated, so the copy already
running on your real display answers the request and opens its window there,
outside the nesting.

Some tests need a real X display and are marked `#[ignore]`. Run them from
inside the nested session:

```bash
DISPLAY=:9 cargo test -- --ignored
```

## Configuration

`$XDG_CONFIG_HOME/primodock/config.toml`:

```toml
[[environments]]
name = "Work"
workspaces = [0, 1]
pinned = ["code", "dev.warp.Warp"]

[[environments]]
name = "Personal"
workspaces = [2, 3]
pinned = ["discord", "spotify"]
```

The ids are `.desktop` file names without the extension. An environment with
no `workspaces` is a catch-all, used for any workspace no other environment
claims. A config from phase 1, with a top-level `pinned` list, is migrated on
load into a single catch-all environment.

## Switching environments

Click the chip on the left of the bar, or bind a key to the D-Bus method. The
dock does not grab keys itself: on Linux the desktop owns the keyboard, and
the app exposes the action.

```bash
KEY=/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/primodock/
gsettings set org.gnome.settings-daemon.plugins.media-keys custom-keybindings "['$KEY']"
gsettings set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:$KEY name 'PrimoDock: cycle environment'
gsettings set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:$KEY binding '<Super>e'
gsettings set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:$KEY command 'gdbus call --session --dest dev.oprimo.PrimoDock --object-path /dev/oprimo/PrimoDock --method dev.oprimo.PrimoDock1.CycleEnvironment'
```

`SetEnvironment` takes a name, if you would rather bind one key per
environment than cycle.

The `GlobalShortcuts` portal would be the other route, but it landed in
`xdg-desktop-portal` 1.17 and Ubuntu 22.04 ships 1.14.
