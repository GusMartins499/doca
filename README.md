# PrimoDock Linux

A native dock for Linux with per-environment profiles and live widgets.
No Electron, no webview.

Port of [PrimoDock](https://dock.oprimo.dev) (macOS) to Linux.

## Status

**Phase 1 — a usable dock.** Pinned apps read from `.desktop` entries, running
apps matched to them by window class, one item per app however many windows it
has, an indicator for running and focused, left click to activate or minimise,
right click to pin, launch or close.

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

`$XDG_CONFIG_HOME/primodock/config.toml`, written by the daemon when you pin
or unpin:

```toml
pinned = ["code", "com.brave.Browser", "discord"]
```

The ids are `.desktop` file names without the extension.
