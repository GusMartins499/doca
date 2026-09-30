# PrimoDock Linux

A native dock for Linux with per-environment profiles and live widgets.
No Electron, no webview.

Port of [PrimoDock](https://dock.oprimo.dev) (macOS) to Linux.

![The dock, with pinned apps on the left and clock, CPU and battery widgets on the right](docs/screenshot.png)

## Requirements

| | |
|---|---|
| Session | X11 |
| Desktop | GNOME Shell 42+ |
| Toolkit | GTK3 |

## Build

```bash
sudo apt-get install -y pkg-config libgtk-3-dev
cargo build --release
```

## Run

### In a sandbox

```bash
./scripts/sandbox.sh
```

Opens a window you can click around in. Inside it is a nested X server with
its own session bus and its own config, seeded from your Plank launchers. The
dock in there reserves space on the nested screen, not yours; your own dock
keeps running untouched. Close the window to stop.

`--headless` runs the same thing on a virtual screen with no window at all,
for screenshots and CI.

This is the way to evaluate the dock at no risk.

### On your own session

```bash
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

`Ctrl+Alt+F2` returns to the desktop.

## Configuration

`$XDG_CONFIG_HOME/primodock/config.toml`:

```toml
[appearance]
theme = "native"
icon_size = 48
magnification = 1.6
auto_hide = false
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

The ids in `pinned` are `.desktop` file names without the extension. An
environment with no `workspaces` is a catch-all, used for any workspace no
other environment claims.

### Appearance

| key | |
|---|---|
| `theme` | `native` (translucent dark glass, blue accents — the default), `midnight` (solid near-black, navy tint), or `paper` (light cream, terracotta accents). An unknown name falls back to `native` with a warning. |
| `icon_size` | clamped to 24–96, and shrinks further on its own when there are more apps than fit. |
| `magnification` | how much the icon under the pointer grows, clamped to 1.0–2.5. `1.0` turns the lens off. |
| `auto_hide` | slides the bar off the bottom edge, leaving a two-pixel sliver to point at, and reserves only that sliver through the strut so maximised windows get the screen back. Pointing at the sliver slides it up; moving away slides it down. |
| `show_trash` | a trash item at the end of the bar. |

### Folders

A path in `folders` becomes a dock item that opens as a grid of its contents,
directories first, then names case-insensitively, capped at 60 entries. Hidden
entries are left out. Each entry opens with `xdg-open`; file icons come from
the content type, not the extension.

### Widgets

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

A poll is not an update: the scheduler compares the rendered state to the last
one and stays quiet when nothing changed, so a clock showing `17:21` is read
every second and announced once a minute.

## Switching environments

The environment follows the workspace, so your own workspace shortcuts already
switch it and the bar carries no control for it. To switch environment without
moving workspace, bind a key:

```bash
./scripts/bind-key.sh                    # <Super>e cycles environment
./scripts/bind-key.sh '<Super>x'         # the same action, your key
./scripts/bind-key.sh '<Super>1' Work    # one key straight to one environment
./scripts/bind-key.sh --list
./scripts/bind-key.sh --remove '<Super>1' Work
```

The script writes a GNOME custom keybinding whose command is a `gdbus call` to
`CycleEnvironment` or `SetEnvironment`. It exists because that list of
keybindings is shared by every application on the desktop: it has to be read
and written back, and a copy-pasted `gsettings set` that assigns the whole
array drops every shortcut you had. Each action owns a named slot, so running
it twice for one action moves that binding instead of adding another.

Cycling walks the environments in config order, skips the catch-all — it has
no workspace of its own to switch to — and refuses when there is only one real
environment.

## Development

```
crates/primodock-ipc     shared D-Bus contract
crates/primodockd        the daemon: windows, workspaces, state
crates/primodock-shell   the bar: draws, anchors, never talks X11 policy
```

The D-Bus boundary between the two is load-bearing: when the X11 session goes
away, the shell is replaced and the daemon survives intact.

`scripts/dev-session.sh` is a barer version of the sandbox, for a debug build
with an empty config:

```bash
sudo apt-get install -y xserver-xephyr xvfb openbox dbus   # once
cargo build
./scripts/dev-session.sh
```

Some tests need a real X display and are marked `#[ignore]`. Run them from
inside a nested session:

```bash
DISPLAY=:9 cargo test -- --ignored
```

## Not implemented

Thumbnail previews on window-list hover: on X11 they need composite redirect
and per-window pixmap capture, which is a great deal of machinery for a hover
effect. Right clicking an app with more than one window still lists the
windows by title, and clicking a title raises that window.
