# PrimoDock Linux

A native dock for Linux with per-environment profiles and live widgets.
No Electron, no webview.

Port of [PrimoDock](https://dock.oprimo.dev) (macOS) to Linux.

## Status

**Phase 5 — parity.** Magnification on hover, a trash item, files dropped onto
an app, folders that open as a grid, and a window list on right click. Thumbnail
previews and the remaining two dozen widgets are not here.

**Phase 4 — themes.** Three of them, chosen in config, plus a configurable
icon size. Three, not eight: the shape of the theme system is what matters,
and more of them is a morning's work once it holds.

**Phase 3 — widgets.** Live tiles in the bar: clock, battery, CPU, music over
MPRIS, and a pomodoro. Each widget declares its own poll interval, and the
scheduler only announces a widget when its rendered state actually changed —
so a paused pomodoro and a steady battery cost nothing.

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

## Try it safely

```bash
cargo build --release
./scripts/sandbox.sh
```

Opens a window you can click around in. Inside it is a nested X server with
its own session bus and its own config, seeded from your Plank launchers. The
dock in there reserves space on the nested screen, not yours; your own dock
keeps running untouched. Close the window to stop.

`--headless` runs the same thing on a virtual screen with no window at all,
for screenshots and CI.

This is the only way to evaluate the dock at no risk, and it is how every
phase of this project is verified.

## Try it on your own session

```bash
cargo build --release
./scripts/try-it.sh
```

Stops Plank, runs PrimoDock in its place, and restarts Plank when you press
Ctrl-C. Two docks cannot share a screen edge — both reserve space through the
same strut protocol and each reacts to the other's reservation — so they take
turns rather than fight. On first run it seeds a config from your Plank
launchers so the dock is not empty.

If the script is killed outright rather than interrupted, Plank will not come
back on its own: `setsid plank >/dev/null 2>&1 &`.

If the dock ever covers the terminal you started it from, switch to a text
console with `Ctrl+Alt+F3`, log in, and run:

```bash
pkill -x primodock-shell; pkill -x primodockd; setsid plank &
```

`Ctrl+Alt+F2` returns to the desktop. The bar is capped at a fraction of the
screen height so it should not get there — that cap exists because an icon
declared as a 1024px PNG once made it.

## Run it in isolation

`scripts/sandbox.sh` above is the one to use. `scripts/dev-session.sh` is the
older, barer version of the same idea, kept for a debug build with an empty
config:

```bash
sudo apt-get install -y xserver-xephyr xvfb openbox dbus   # once
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
[appearance]
theme = "native"
icon_size = 48
magnification = 1.6
show_trash = true

[[environments]]
name = "Work"
workspaces = [0, 1]
pinned = ["code", "dev.warp.Warp"]
widgets = ["clock", "pomodoro", "cpu", "battery"]
folders = ["~/Downloads"]

[[environments]]
name = "Personal"
workspaces = [2, 3]
pinned = ["discord", "spotify"]
widgets = ["clock", "music"]
```

## Themes

| `theme` | |
|---|---|
| `native` | translucent dark glass, blue accents. The default. |
| `midnight` | solid near-black, navy tint |
| `paper` | light cream, terracotta accents |

An unknown name falls back to `native` with a warning rather than leaving the
bar unstyled. `icon_size` is clamped to 24–96, and shrinks further on its own
when there are more apps than fit.

## Stacks

A path in `folders` becomes a dock item that opens as a grid of its contents,
directories first, then names case-insensitively, capped at 60 entries so a
crowded Downloads folder does not become an endless menu. Hidden entries are
left out. Each entry opens with `xdg-open`; file icons come from the content
type, not the extension.

## Window lists

Right clicking an app with more than one window lists them by title, and
clicking a title raises that window. Titles are elided by character count, not
bytes, so accented titles are cut where you would expect.

Thumbnail previews are not implemented: on X11 they need composite redirect
and per-window pixmap capture, which is a great deal of machinery for a
hover effect.

## Magnification

`magnification` is how much the icon under the pointer grows, clamped to
1.0–2.5. `1.0` turns the lens off.

The lens measures distance against the *base* layout, cached when the bar is
built, not against the current one. Measuring against the live layout makes
the icon grow, push its neighbours, change the distance, and shake.

## Widgets

| id | shows | poll | click |
|---|---|---|---|
| `clock` | time and date | 1s | — |
| `battery` | charge and time to full or empty | 30s | — |
| `cpu` | busy share since the last sample | 2s | — |
| `network` | up and down rates | 2s | — |
| `music` | MPRIS title and artist | 2s | play/pause, right click resets |
| `pomodoro` | focus and break blocks | 1s | start/pause, right click resets |
| `stopwatch` | counting up, with laps | 1s | start/pause, right click resets |
| `timer` | counting down to a ring | 1s | start/pause, right click resets |
| `time-progress` | how much of a span has gone | 30s | next span, right click back to today |
| `countdown` | days to a date | 60s | — |
| `water` | glasses against a goal | 60s | one more, right click resets |
| `note` | a line you leave yourself | — | — |

Widgets that need settings take them from a `[widgets.*]` block:

```toml
[widgets.countdown]
date = "2026-12-25"
label = "until Christmas"

[widgets.note]
text = "Call the dentist\nThursday at four"

[widgets.timer]
minutes = 15

[widgets.water]
goal = 8
```

A poll is not an update. The scheduler compares the rendered state to the last
one and stays quiet when nothing changed, so a clock showing `17:21` is read
every second and announced once a minute. That is deliberate: this is a laptop
dock, and a tile that wakes the bar sixty times a minute is paid for in
battery.

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
