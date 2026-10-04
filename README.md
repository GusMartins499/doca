# Doca

A native dock for Linux with per-environment profiles and live widgets.
No Electron, no webview.

Doca is a Linux port of [PrimoDock](https://dock.oprimo.dev) (macOS).

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

Stops Plank, runs Doca in its place, and restarts Plank when you press
Ctrl-C. Two docks cannot share a screen edge — both reserve space through the
same strut protocol and each reacts to the other's reservation — so they take
turns rather than fight. On first run it seeds a config from your Plank
launchers so the dock is not empty. Right-clicking the dock opens
**Preferences…**, which edits that same config while it runs — the script puts
the freshly built binaries on `PATH` so the window that opens is the one you
just compiled.

If the script is killed outright rather than interrupted, Plank will not come
back on its own: `setsid plank >/dev/null 2>&1 &`.

If the dock ever covers the terminal you started it from, switch to a text
console with `Ctrl+Alt+F3`, log in, and run:

```bash
pkill -x doca-prefs; pkill -x doca-shell; pkill -x docad; setsid plank &
```

`Ctrl+Alt+F2` returns to the desktop.

## Configuration

`$XDG_CONFIG_HOME/doca/config.toml`:

```toml
[appearance]
theme = "native"
icon_size = 48
magnification = 1.6
auto_hide = false
show_trash = true
icon_theme = ""       # empty follows the system
gtk_theme = ""
cursor_theme = ""

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
| `theme` | `system` (borrows the colours of the GTK theme in force), `native` (translucent dark glass, blue accents — the default), `midnight` (solid near-black, navy tint), or `paper` (light cream, terracotta accents). An unknown name falls back to `native` with a warning. |
| `icon_size` | clamped to 24–96, and shrinks further on its own when there are more apps than fit, or than fit with the lens open. |
| `magnification` | how much the icon under the pointer grows, clamped to 1.0–2.5. `1.0` turns the lens off. The lens needs room to open, which comes out of the icon size on a crowded dock. |
| `auto_hide` | slides the bar off the bottom edge, leaving a two-pixel sliver to point at, and reserves only that sliver through the strut so maximised windows get the screen back. Pointing at the sliver slides it up; moving away slides it down. |
| `show_trash` | a trash item at the end of the bar. |
| `icon_theme` | an icon theme for the dock alone. Empty follows `gtk-icon-theme-name`, which is what GNOME Tweaks → Appearance → Icons sets. |
| `gtk_theme` | a GTK theme for the dock alone, which is what `theme = "system"` reads its colours from. Empty follows the system. |
| `cursor_theme` | a cursor theme for the dock's own windows. Empty follows the system. |

A name no theme answers to is not an error: GTK falls back on its own and the
log says what was asked for, so the dock stays usable. Clearing an override
back to `""` restores whatever the desktop asked for, without a restart.

### Following the system

With `theme = "system"` the bar takes `@theme_bg_color`, `@theme_fg_color`,
`@theme_selected_bg_color` and `@borders` from the GTK theme instead of carrying
a palette of its own, and changing the theme in GNOME Tweaks is reflected
without restarting anything — the dock reloads the stylesheet, and a change of
icon theme reloads the icons on the bar.

Two things worth knowing before choosing it:

- **The GNOME Shell theme is out of reach.** A *Shell* theme is a
  `gnome-shell.css` loaded by the GNOME Shell process; the dock is an ordinary
  GTK3 window, not a Shell extension, and cannot read it. What `system` follows
  is the *Applications* theme — the one GNOME Tweaks sets under Appearance →
  Applications. Picking `Sweet-Dark-v40` for both is what makes them match; the
  dock only ever sees the Applications half.
- **A name no theme answers to is a warning, not a refusal.** GTK falls back to
  something readable on its own, so `icon_theme = "Papirus-Drak"` leaves the
  dock looking exactly as it did — which is indistinguishable from the setting
  not working. The dock checks the three override names against `~/.themes`,
  `~/.icons` and the shared data directories, and says in the log which one is
  not installed.
- **A theme that defines none of those four colours falls back to `native`**,
  with the missing names in the log. GTK accepts a stylesheet naming colours
  the theme never defined — the load succeeds and the declaration simply draws
  as nothing, which on a translucent window means an *invisible* bar. So the
  colours are looked up before the sheet is trusted rather than waiting for a
  failure that never arrives.

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

An environment is a dock: its own apps, its own widgets. Two of them can share
one workspace and be swapped by a key, or each can own a workspace and follow
it — `workspaces` is what decides which. Leave it off every environment and
the workspace stops deciding altogether; the first in the file is what you
start on.

A key puts an environment on screen and moves nothing. The choice stands until
you step onto a workspace some environment asked for by name: that is a choice
of its own and replaces the one the key made. A workspace only a catch-all
covers asks for nothing, so a dock chosen by hand survives moving across it.

The dock does not grab keys itself — on Linux the desktop owns the keyboard —
so bind one. The [Shortcuts tab](#preferences) does this without a terminal;
the script is the same thing for automation, and writes the same slots:

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

Keys bound before this project was renamed from PrimoDock to Doca sit in slots
named `primodock-*`, still calling a bus name that no longer answers: dead keys,
and `--list` does not see them to say so. Rebind each one with the commands
above, then delete the old entries — they show up under Settings → Keyboard →
Custom Shortcuts, named `PrimoDock: …`.

Cycling walks the environments in config order and wraps round, catch-alls
included: switching by hand needs no workspace to switch to. It refuses when
there is only one environment, which is the only case with nowhere to go.

## Preferences

```bash
doca-prefs
```

Also from **right-clicking the dock → Preferences…**, and from the applications
menu once `packaging/doca-prefs.desktop` is installed. It is single-instance:
opening it again brings the window you have to the front rather than starting a
second writer.

Every control applies as it moves. There is no Apply and no OK — a dialog that
asked you to confirm what you had just chosen would be the TOML again, with
buttons. The window writes only through the bus, so the daemon stays the one
thing that validates and saves, and a change made anywhere else — a bound key,
a second window — moves these controls too.

The **Appearance** tab is live: theme, icon size, magnification, auto-hide and
the trash. The theme list comes from the shared contract rather than being
written out in the window, so a theme added to the dock appears here without
anyone remembering to.

The **Docks** tab is the part a terminal was the only way to reach. Docks are
added, removed and renamed on the left; on the right are the workspaces that
dock claims, the apps it pins and the widgets it shows. The workspace field
counts from 0, the same as the config file, so a hand-edited TOML and this
window never disagree about which workspace is which; left empty, the dock
covers whatever no other dock asked for. **Up** and **Down** under the list
reorder the docks themselves — that order is the order a key cycles through
them, and until this it was the one piece of the config only a text editor
could reach. **Add app…** opens a box you can type
in, over every installed application — searching the id as well as the name,
because the id is what the file holds. Pins reorder with Up and Down, which
move one pin a place and nothing else.

A name is renamed when you press Enter, not as you type: a rename sent letter
by letter would ask the daemon for "W", then "Wo", then "Wor". Remove greys out
on the last dock, because the daemon refuses to remove it and a button that is
always refused is better greyed than argued with. Anything else refused — a
name already taken — is shown as the sentence the daemon sent back.

The **Widgets** tab is what each widget is *set to* — which is a different
question from which docks show it, and the reason the two live in different
tabs: the countdown counts to one date no matter how many docks show it, while
showing it is a property of the dock. Four of the twelve take settings — a
countdown's date and caption, a note's text, a timer's length, a water goal —
and the other eight say so rather than leaving a blank pane. A widget id in the
config that nothing answers to — a typo in a hand-edited TOML — appears at the
end of the list marked *unknown*, spelled the way the file spells it, rather
than quietly not being there: the daemon drops it with a log line nobody reads,
and the bar showing nothing looks exactly like a widget that was never turned
on. Picking it says which docks ask for it and that it has to come out of
`config.toml`, because the tick boxes in the Docks tab are built from the
widgets that exist and an id that does not exist has no box to untick. Under each name is
the line that answers the question this tab exists for: **Shown in Work and
Home**, or, when nothing shows it, where to turn it on. A date is checked
before it is sent, because the daemon accepts any text and the widget would
just show "bad date" in the bar with no room to say why; an empty date is
allowed, since that is how a countdown is turned off. The date and caption go
out when you press Enter, the note when you click away or press its button —
Enter belongs in a note.

A setting reaches the widget that is already running, without restarting
anything, and without resetting what it was counting: a new water goal keeps
today's glasses, and a timer already counting down keeps its count and takes
the new length at its next reset. Changing which widgets a dock shows works
the same way — the ones that stay are carried over, not rebuilt, so adding a
widget to one dock does not reset the stopwatch in another.

The **Shortcuts** tab is the only one that does not write the dock's config.
The dock does not grab keys — on Linux the desktop owns the keyboard — so what
this writes is GNOME's own list of custom keybindings, the same list
`scripts/bind-key.sh` writes and the same one Settings → Keyboard → Custom
Shortcuts shows. The tab says so on screen, because a key that turns up in the
desktop's settings is surprising if nothing said it would.

One row for the cycle, one per dock. Click a key to set it, Backspace to clear
it, Esc to leave it as it was. A shortcut needs Ctrl, Alt or Super held with
it: a letter on its own would answer every time it was typed, and `<Shift>e` is
just E — Shift counts towards the name of a shortcut, never towards making one.
A lock that happens to be on, Num Lock above all, is kept out of the name, or
the key would stop working the moment it was switched off.

A key another application already holds is bound anyway and said out loud,
naming what else has it: refusing would be the dock deciding who owns a key it
does not own either. A key one of the *other rows* holds is refused, because
this window owns both sides of that collision and clearing one is a click away.

Two docks whose names come out the same once punctuation is stripped — `My
Work` and `My-Work` — would share one keybinding slot, so both rows say so and
offer no key until one is renamed. The same goes for a dock named only in
punctuation, which leaves nothing to make a slot name out of.

The script is not replaced by any of this: it is what automation and a terminal
reach for, and the two write the *same* slot, label and command, so binding a
key in one place moves what the other bound rather than sitting beside it.
`scripts/check-live.sh` holds them to that, against a real dconf.

On a desktop that keeps no such list — KDE, sway, a machine without
gnome-settings-daemon — the tab says that in a line instead of offering capture
buttons that would take a key press and drop it. The actions are still on the
bus; [the table below](#changing-the-configuration-while-it-runs) is what to
bind them to by hand.

With no daemon running the window says so in a line, instead of drawing
controls that would all look like they had worked and changed nothing.

A separate binary rather than a window inside the dock, for three reasons: the
shell draws the bar and reserves the screen edge, and a fault in a settings
window must not take that down; the window leaves memory when it closes; and
the daemon stays the single source of truth, which makes this window just
another caller on the bus, no different from a keybinding.

## Changing the configuration while it runs

Everything in `config.toml` that the dock can change, it can change on the bus,
and the bar applies it without being restarted. The daemon is the only writer:
it validates what arrives, writes the file beside itself and renames it into
place — so a session that dies mid-write leaves the old config intact rather
than half a TOML — and then announces `ConfigChanged`, which the bar answers by
re-reading and reapplying the theme, the icon size, the lens, the auto-hide and
the trash on the window already on screen.

| method | |
|---|---|
| `SetAppearance(a{sv})` | changes only the keys named: `theme`, `icon_size`, `magnification`, `auto_hide`, `show_trash`, `icon_theme`, `gtk_theme`, `cursor_theme`. A key nobody knows is refused rather than ignored; a value out of range is brought back in (an `icon_size` of 4000 becomes 96). |
| `SetWidgetSetting(s, s, v)` | one widget's one setting: `countdown`/`date`, `countdown`/`label`, `note`/`text`, `timer`/`minutes`, `water`/`goal`. Written, announced, and handed to the widget that is already running — which keeps whatever it was counting. A key nobody knows is refused rather than ignored, and a value out of range is brought back in. |
| `WidgetSettings()` → `(sssuu)` | what every widget setting is, already clamped. The counterpart of `SetWidgetSetting`, flat and named the same way, so a window can put them straight on its controls. |
| `AddEnvironment(s)` → `s` | a new dock, with nothing pinned and no workspace claimed, so it starts as a catch-all. Returns the name as stored. |
| `RemoveEnvironment(s)` | removes a dock. The last one cannot go: something always has to be on screen. |
| `RenameEnvironment(s, s)` → `s` | renames a dock, and follows the rename if that dock is the one a key put on screen. |
| `SetEnvironmentWorkspaces(s, ai)` | which workspaces a dock claims, tidied: sorted, deduplicated, negatives dropped. An empty list makes it the catch-all. |
| `SetEnvironmentWidgets(s, as)` | the widgets a dock shows, in the order it shows them. |
| `ReorderEnvironments(as)` | reorders the docks themselves, which is the order a key cycles through them. Same contract as `ReorderPinned`: a name nobody knows is refused, and a dock left out keeps its place at the end. |
| `ReorderPinned(s, as)` | reorders one dock's pins and nothing more. An id that is not pinned there is refused, and an id left out keeps its place at the end — so a window working from a stale list cannot quietly unpin what it had not heard about. |
| `PinIn(s, s)` / `UnpinIn(s, s)` | pin and unpin in the dock named, which need not be the one on screen — that is the whole difference from `PinItem`/`UnpinItem`, which act where the user is looking because that is what a click on the bar means. `PinIn` refuses an id no `.desktop` file answers to; `UnpinIn` does not, since that is how an uninstalled app gets out of the config. |
| `ListEnvironments()` → `a(saibasas)` | every dock: its name, the workspaces it claims, whether it is the one on screen, what it pins and which widgets it shows. Everything a writer can change about a dock is something a reader can see. |
| `ListApplications()` → `a(sss)` | every installed application — id, name, icon — sorted by name, for a window that has to offer a choice of them. Entries marked `NoDisplay` are left out, the same ones a desktop menu leaves out. |

Anything refused comes back as `InvalidArgs` with the reason in it, and nothing
is written. Numbers may be sent as any integer width, or as text, so a
keybinding or a `gdbus call` typed by hand works as well as a GUI does:

```bash
gdbus call --session --dest io.github.gusmartins499.Doca \
    --object-path /io/github/gusmartins499/Doca \
    --method io.github.gusmartins499.Doca1.SetAppearance \
    '{"theme": <"midnight">, "icon_size": <int32 64>}'
```

## Checking it against a real session

`cargo test` covers everything that can be decided without a bus. Four things
cannot be, and have scripts of their own:

```bash
cargo build --release && ./scripts/check-live.sh
```

starts a daemon, the bar and the preferences window on a nested display with a
session bus, a config and a dconf database of their own, and asserts four
things no unit test can reach.

That a change made from anywhere else reaches the window. Writing is a method
call and hearing is a signal, and the two break apart — every control can work
while the window quietly stops following. It waits for the subscription rather
than for the window, because the window is drawn well before it subscribes.

And that a widget setting reaches the widget that is already running: it drinks
two glasses, moves the water goal to four, and expects `2/4` back. A widget
rebuilt instead of told would answer `0/4`, and the value landing in the config
file would prove nothing either way.

And that a rebuild of the bar keeps the widget tiles it already had. The bar is
rebuilt whenever a window opens, closes or takes focus, and on every change to
the config as well — which means on every frame of a slider being dragged in
the preferences window. Tiles torn down and built again at that rate are a
visible flicker, and a bar that quietly went back to doing so looks exactly
like one that did not: the counts are the only thing that tells them apart, so
they are read out of the bar's own log. Changing the icon size should carry
every tile over and make none; dropping one widget should take one tile and
leave the other where it was.

And that `scripts/bind-key.sh` and the Shortcuts tab write the same slot, label
and command. Two implementations of one algorithm is the price of the script
staying useful for automation, and the price is only safe while they agree — a
slot named differently would not replace the other's binding, it would sit
beside it, and one key would fire the action twice. It binds with the script and
then reads the count back out of the window's own log. It also plants another
application's keybinding first, and expects to find it afterwards: the list is
shared by every app on the desktop, and the whole reason the script exists is
that writing the array outright drops everyone else's keys.

The keybinding part writes dconf, so `XDG_CONFIG_HOME` is exported **before**
`dbus-run-session`, not after. dconf writes do not go to the file the writing
process points at — they go over the bus to `dconf-service`, which inherits its
environment from the bus daemon. Exported too late, the session *reads* the
temporary database and *writes* the real one, and the script quietly replaces
the keybindings of whoever ran it. A sentinel write proves the isolation before
anything shared is touched, and the keybinding checks are skipped rather than
guessed at if it fails.

```bash
DISPLAY=:9 cargo test -p doca-shell -- --ignored
DISPLAY=:9 cargo test -p doca-prefs -- --ignored
```

run the checks that need real GTK widgets — the ones that would otherwise be
assertions about what GTK probably does. They need an X server on `:9`, which
either `./scripts/dev-session.sh` or `./scripts/sandbox.sh` leaves running in
another terminal.

## Development

```
crates/doca-ipc     shared D-Bus contract
crates/docad        the daemon: windows, workspaces, state
crates/doca-shell   the bar: draws, anchors, never talks X11 policy
crates/doca-prefs   the preferences window: writes only through the bus
```

The D-Bus boundary between the two is load-bearing: when the X11 session goes
away, the shell is replaced and the daemon survives intact.

The row of icons is one widget that draws them all, not a widget each. A
widget per icon cannot animate: changing an icon's size changes what it asks
of its parent, so every frame of the lens renegotiates the layout of the whole
bar — 33 frames a second with twenty-nine icons, against 60 for a drawing.
Where an icon goes is a function of its own resting place and the pointer, and
of nothing else, so growing one cannot shift the rest by accumulation. Both
the shape of that and its constants come from
[Plank](https://github.com/ricotz/plank)'s `PositionManager`.

What the lens is aimed at is not quite the pointer. A pointer that *moves* is
followed exactly — nothing is interpolated and nothing lags. A pointer that
*appears* somewhere it did not travel to is not: leaving the bar at one end
and coming straight back at the other aims the lens somewhere new between two
frames, which moved an icon 61px at once, where even a brisk sweep costs 27px.
So the lens has a ceiling on how fast it travels, set above any speed a hand
produces, and a jump is spread over the frames it takes — at worst, across the
whole row, about as long as the lens takes to open. The figures are measured
off the lens's own shape rather than guessed at, and the checks in
`crates/doca-shell/src/row.rs` hold a jump to what a sweep at that ceiling
would have cost.

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

GTK belongs to the thread that starts it and the harness gives each test a
thread of its own, so those checks live beside the code they check and are
called from a single `on_a_display` test rather than being `#[test]`s.

Hover has no one to trigger it on a headless screen, so the pointer is moved
by hand — and, for clicks, pressed for real through XTEST:

```bash
DISPLAY=:9 cargo run -p docad --example warp-pointer -- 700 1035
DISPLAY=:9 cargo run -p docad --example warp-pointer -- 700 1035 click:3
```

## Not implemented

Thumbnail previews on window-list hover: on X11 they need composite redirect
and per-window pixmap capture, which is a great deal of machinery for a hover
effect. Right clicking an app with more than one window still lists the
windows by title, and clicking a title raises that window.
