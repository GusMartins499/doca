# Leaving GTK3: an assessment

Issue #81, slice 5 of #72 (finding 4). An assessment, not a port. Facts
checked on 2026-11-08.

## Summary

Do not port to GTK4 now. The GTK3 bindings are maintained again: the
RUSTSEC advisory was withdrawn on 2026-08-14 and `gtk` 0.19.0 came out on
2026-09-08, and moving to it costs three compile errors. GTK4 itself has
deprecated its X11 backend, so a port would trade a toolkit whose home is
X11 for one leaving it, and would still do every dock-specific call through
x11rb. The deadline that matters is not the toolkit but GNOME's X11
session, which GNOME 50 removed. Bump to 0.19, and open a separate decision
on Wayland.

## The problem

Issue #72 found that `gtk3-rs` was archived on 2024-03-04, that 0.18 (the
version Doca uses) was the last GTK3 release, that RUSTSEC-2024-0412 marked
it unmaintained, and that Fedora proposed retiring the packages. GTK4 brings
`Snapshot`, GPU rendering and blur, but it removed the X11 window APIs the
dock is built on.

Two of those facts have since changed (see [How long we can stay](#how-long-we-can-stay)).
The third, that GTK4 has no dock APIs, still holds, and it got worse.

## Inventory

Every GTK3/GDK3 call in `doca-shell` and `doca-prefs` that GTK4 removed,
renamed, or only has on X11. `docad` does not link GTK: its
`crates/docad/src/x11.rs` reads the EWMH window list (`_NET_CLIENT_LIST`,
`_NET_ACTIVE_WINDOW`, `_NET_WM_STATE`…) over x11rb, and the toolkit does not
affect it. `crates/doca-shell/src/strut.rs` is also x11rb already; it needs
only the window's XID from GDK.

Cost: **P** small (hours, local), **M** medium (a day or more, many call
sites or a behaviour to re-check), **G** large (a rewrite of that part).

| API (GTK3) | Where | In GTK4 | Substitute | Cost |
|---|---|---|---|---|
| `set_type_hint(Dock)` on the bar | `doca-shell/src/main.rs:45`, `keys.rs:527` (test) | Removed, with no type-hint API at all | x11rb: set `_NET_WM_WINDOW_TYPE = _NET_WM_WINDOW_TYPE_DOCK` on the XID in `realize`, before the window maps | P |
| `Window::new(WindowType::Popup)` + `set_type_hint(Tooltip/Dnd/Notification)` | `tooltip.rs:50-51`, `preview.rs:80-81`, `poof.rs:36-37,62,193`, `main.rs:548` (edge strip) | `GtkWindowType` gone; no override-redirect toplevel | Tooltip and preview: `GtkPopover` anchored to the bar (it gets its own `GdkPopup` surface and can extend outside the bar). Poof (at an arbitrary screen point) and the edge strip: a toplevel made override-redirect through x11rb `ChangeWindowAttributes` before map. Not verified that GDK4 tolerates it | M |
| `move_` | `main.rs:255,265,275,606,630`, `tooltip.rs:132,134`, `preview.rs:238,240`, `poof.rs:114,195` | Removed ("Most likely, you should just stop using them") | x11rb `ConfigureWindow` on the XID. GDK4 no longer knows where the window is, so the auto-hide slide (one move per frame) keeps its own coordinates | M |
| `set_keep_above`, `stick` | `main.rs:46-47`, `tooltip.rs:52`, `preview.rs:82` | Removed | `_NET_WM_STATE_ABOVE` client message and `_NET_WM_DESKTOP = 0xFFFFFFFF` via x11rb; most window managers already keep a DOCK above | P |
| `set_skip_taskbar_hint`, `set_skip_pager_hint` | `main.rs:50-51` | Moved to `gdk4-x11` `X11Surface` (deprecated since 4.18) | `X11Surface::set_skip_*_hint`, or `_NET_WM_STATE_SKIP_*` via x11rb | P |
| `gdkx11::X11Window::xid()` for the strut | `main.rs:2024,2044`, `main.rs:2741` (test); `strut.rs:35-80` | `gdk4-x11` `X11Surface::xid()`, deprecated since 4.18 | Same call; `strut.rs` is unchanged | P |
| `input_shape_combine_region` | `main.rs:532`, `poof.rs:47` | Renamed `gdk::Surface::set_input_region` (not deprecated) | Direct | P |
| `rgba_visual` / `set_visual` / `set_app_paintable` / `is_composited` | `main.rs:52-56`, `tooltip.rs:57-59`, `preview.rs:87-89`, `poof.rs:41-42,189`, `widget_tile.rs:1614`, `theme.rs` | GTK4 always uses RGBA; `GdkVisual` removed; `Display::is_composited` stays | Delete the visual code; keep the composited check | P |
| `connect_draw` with a cairo `Context` | `row.rs:393`, `widget_tile.rs:119`, `panel.rs:702`, `ground.rs:101`, `poof.rs:65,201`, `main.rs:554` | `draw` signal removed | `DrawingArea::set_draw_func`, which still hands a `cairo::Context`; the drawing code survives. Drawing on a bare `Window` needs a `DrawingArea` child | P–M |
| `EventBox` and the event signals (`connect_button_press/release_event`, `motion_notify`, `enter/leave_notify`, `key_press`, `add_events`/`EventMask`) | `widget_tile.rs:129-165`, `dock.rs:68-72`, `panel.rs:758-862`, `main.rs:564-684,941-1039,1839`, `preview.rs:91,119-124`, `keys.rs:166-173`, `row.rs:349`; prefs `shortcuts.rs:372` | `EventBox` removed; "Event controllers and gestures replace event signals" | `GestureClick`, `GestureDrag`, `EventControllerMotion`, `EventControllerKey`. Mechanical, but the drag-to-reorder and auto-hide timings have to be re-checked | M |
| `Seat::grab` (keyboard mode) | `keys.rs:239-284` | `gdk_seat_grab` removed with no replacement except a popup's autohide | x11rb on its own connection would take the keys away from GTK. Options: `XGrabKeyboard` on GTK's own Xlib connection (`X11Display::xdisplay()`, deprecated 4.18), or re-shape the keyboard mode around an autohide popover. No clean substitute | M |
| `gtk::Menu` / `MenuItem`, `popup_at_pointer`, `popup_at_rect`, `popup_at_widget` | `main.rs:1297,1338,1386-1389,1843,2636,2674`, `panel.rs:110-118,782,862,1225`, `stack.rs:321`, `keys.rs:707`, `dock.rs` (25 `MenuItem`) | Removed; `PopoverMenu` "can only be constructed from menu models" | `gio::Menu` + actions + `PopoverMenu::set_pointing_to`. The hand-written key handling on menus (`panel.rs:782,862`) has to be redone | M |
| `drag_dest_set` + `TargetEntry` (uri-list on an icon); `connect_drag_motion` (reveal a hidden bar) | `dock.rs:419-431`; `main.rs:366,700-701` | Removed | `DropTarget` with `gdk::FileList`; `DropControllerMotion` | P |
| `set_tooltip_text` | `dock.rs`, `stack.rs`; prefs `shortcuts.rs` | Stays | Direct | P |
| `StyleContext::add_provider_for_screen` | `main.rs:60`, `fit.rs:228`, `ground.rs:206`; prefs `main.rs:66` | `GdkScreen` removed | `gtk::style_context_add_provider_for_display` | P |
| `StyleContext::lookup_color` for the `system` theme | `theme.rs:220-367`, `main.rs:1436-1475,1728,1856` | Deprecated since 4.10, "will be removed in GTK 5"; no replacement named | None for named `@define-color`s. GTK4 also reads a theme's `gtk-4.0/` stylesheet, not the `gtk-3.0/` one `theme = "system"` follows today. **No substitute**; the mode would have to be redefined | M |
| `gtk::Settings` (theme and icon theme names) | `appearance.rs:33-189`, `main.rs:741,782` | Stays | Direct | P |
| `IconTheme` + `Pixbuf` | `dock.rs`, `appearance.rs`, `row.rs`, `preview.rs`, `poof.rs` (25 `Pixbuf` uses) | `IconTheme::for_display`; lookups return an `IconPaintable`, not a pixbuf; `Texture::new_for_pixbuf` deprecated in 4.20 | Load to `gdk::Texture` (glycin is the suggested loader), or keep pixbufs for cairo and look up icon paths only | M |
| ATK (`accessible()`, `atk::Role`) | `row.rs:540`, `preview.rs:305`, `widget_tile.rs:229-231` | No ATK | `gtk::Accessible`: role at construction, `update_property(Label)` | P |
| Monitors: `n_monitors`, `monitor(i)`, `is_primary`, `monitor_at_point` | `screen.rs:77-85`, `tooltip.rs:125`, `preview.rs:213` | `Display::monitors()` list model; no `is_primary`; `X11Display::primary_monitor()` deprecated 4.18 | RandR `GetOutputPrimary` via x11rb, or the deprecated call | P |
| `Container`, `Layout`, `show_all`, `pack_start` | `widget_tile.rs:363,2233,2479`; prefs, 65 `pack_start` | Removed / renamed (`append`, `Fixed`) | Mechanical | P–M |
| Prefs widgets: `Dialog::with_buttons`, `ComboBoxText`, `set_shadow_type` | prefs `docks.rs:616`, `appearance.rs:108,131`, `widgets.rs:239,302,326,334`, `chrome.rs:38` | Dialog without `run`; `ComboBoxText` deprecated 4.10; `ShadowType` removed | Response signal; `DropDown`; CSS | M |
| `events_pending` / `main_iteration_do` (tests) | `keys.rs:490`, `stack.rs:283`, `panel.rs:1178`, `poof.rs:276`, `main.rs:2397`, `widget_tile.rs:2628` | Removed | `glib::MainContext::default().iteration(false)` | P |
| `Seat::pointer`, `device_position` | `main.rs:397,1686-1691` | Stay | Direct | P |

Five rows are X11 window management (type, popups, move, above/sticky,
skip hints). GTK4 offers nothing for them except deprecated `gdk4-x11`
getters; all of them become x11rb calls on the XID, as `strut.rs` already is.

## How long we can stay

**The bindings are maintained again.**

- RUSTSEC-2024-0412 (and its siblings for `gtk`, `gdkx11`, `atk`…) is
  marked withdrawn on 2026-08-14: "The gtk-rs repository was unarchived".
  The withdrawal was merged into advisory-db on 2026-09-08 (PR #3189).
- `gtk-rs/gtk3-rs` is no longer archived; `gtk` 0.19.0 was released on
  2026-09-08, on `glib`/`cairo-rs`/`pango` 0.22 (the line `gtk4` 0.11 uses),
  with MSRV 1.92 (Doca's is 1.87).
- The revival is one person: 98 of the 100 commits since 2026-01-01 are by
  the same contributor. A bus factor of one.
- Trial run here: with `gtk`/`gdk`/`gdkx11`/`atk` at 0.19 and
  `glib`/`pango`/`pangocairo` at 0.22, `cargo check --all-targets` on
  `doca-shell` and `doca-prefs` gives **3 errors**, all in `doca-shell`:
  `atk::Role::PushButton` is gone (`widget_tile.rs:231,3809`; `Role::Button`
  remains) and `gio::content_type_guess` takes an `Option<&[u8]>`
  (`stack.rs:99`). `doca-prefs` checks clean. Nothing was run or tested.

**Fedora's retirement does not reach Doca.** FESCo accepted it on
2025-04-10 and the `rust-gtk` package is dead in Fedora 43. That removes
Fedora's own `rust-*` source packages, used to build Rust apps inside
Fedora. Doca builds with cargo from crates.io and needs only the C library,
which every distribution still ships:

| Distribution | GTK 3 (C) |
|---|---|
| Upstream | 3.24.52, tagged 2026-03-22 |
| Fedora 44 | 3.24.52 |
| Debian 13 trixie / forky | 3.24.49 / 3.24.52 |
| Ubuntu 24.04 noble / 26.04 resolute | 3.24.41 / 3.24.52 |
| Arch | 3.24.52 (2026-10-07) |

It would matter only if Doca were packaged in Fedora proper.

**The real clock is GNOME's X11 session.** The README asks for an X11
session on GNOME Shell 42+. GNOME 49 turned X11 off at compile time and
GNOME 50 removed it from Mutter and GDM. Fedora 43 ships GNOME Wayland-only,
Ubuntu dropped GNOME on Xorg in 25.10, and Ubuntu 26.04 ships GNOME Shell
50.1. What is left:

| Release | GNOME Shell | X11 session | Standard support |
|---|---|---|---|
| Ubuntu 22.04 | 42.0 | yes | May 2027 |
| Ubuntu 24.04 | 46.0 | yes | May 2029 |
| Debian 12 | 43.9 | yes | — |
| Debian 13 | 48.7 | yes | — |
| Ubuntu 26.04, Fedora 43+ | 50 / 49 | no | — |

So Doca's platform ends around May 2029 whatever the toolkit, and a GTK4
port does not move that date: GTK4's X11 backend was deprecated with 4.18
(GTK blog, 2025-02-01: "we intend to remove them in the GTK 5"), and every
`GdkX11Surface`/`GdkX11Display` method is deprecated since 4.18.

## Alternatives

Size of what would change (`wc -l`, and lines naming `gtk::`/`gdk::`/`atk::`):

| Module | Lines | GTK lines | cairo lines |
|---|---|---|---|
| `doca-shell/src/widget_tile.rs` | 4,362 | 132 | 178 |
| `doca-shell/src/main.rs` | 2,890 | 121 | 5 |
| `doca-shell/src/panel.rs` | 2,341 | 107 | 12 |
| `doca-shell/src/row.rs` | 2,328 | 28 | 28 |
| `dock.rs`, `keys.rs`, `stack.rs`, `theme.rs`, `ground.rs`, `preview.rs`, `poof.rs`, `fit.rs`, `screen.rs`, `tooltip.rs`, `appearance.rs` | 5,245 | 273 | 23 |
| `motion.rs`, `magnify.rs`, `strut.rs` (no GTK) | 1,482 | 0 | 0 |
| `doca-prefs` (8 files) | 5,010 | 246 | 0 |

| Option | What it gives | What it costs | Cost |
|---|---|---|---|
| **A. Stay on GTK3, move to 0.19** | Maintained bindings, same behaviour | 3 errors, MSRV to 1.92 | P |
| **B. GTK4 + `gdk4-x11` + x11rb** | `Snapshot`, GPU, blur | Every row of the inventory; X11 window management by hand on a deprecated backend; keyboard grab and `system` theme without a clean substitute. Prefs alone is M; the shell is G | G |
| **C. No toolkit: x11rb + cairo (`xcb` surface) + pango** | No GTK at all; `cairo-rs`/`pango` 0.22.9 are maintained (2026-08-30) | Only the drawing code survives. Doca would own the event loop, menus, XDND drops, icon-theme lookup, HiDPI, theme colours, and accessibility (e.g. AccessKit). Still X11-only | G+ |
| **D. Wayland via `gtk4-layer-shell` 0.8.1** | A dock on KDE Plasma, wlroots, COSMIC, Mir | Not GNOME: its README lists GNOME-on-Wayland as unsupported, and it does not work on X11. Requires B first. `docad`'s window list would need a foreign-toplevel protocol per compositor (not verified which ones offer it). A different product target | G |
| **E. GNOME Shell extension** | The only way to a dock on GNOME 50 | A different language (JS) and architecture; not assessed here | — |

C has no reason to exist: it costs more than B and leaves Doca exactly as
tied to X11. B is worth it only together with D, that is, only if Doca
decides to target the layer-shell compositors.

## Recommendation

Stay on GTK3. Slices:

1. **chore(deps): gtk-rs 0.19** — `gtk`/`gdk`/`gdkx11`/`atk` 0.19,
   `glib`/`pango`/`pangocairo` 0.22, `rust-version` 1.92; fix the three
   errors; run the `--ignored` GTK tests and the sandbox. P.
2. **refactor(shell): one X11 seam** (optional) — gather the XID downcasts
   (`main.rs:2024,2741`), the strut, the type hint and the input region into
   one module, so a later change of backend touches one file. P.
3. **A decision, in its own issue: Doca after GNOME's X11 session** — stay
   X11 (GNOME until ~2029, plus Xfce/MATE/Cinnamon), a layer-shell dock for
   KDE/wlroots (D, which brings B), or a GNOME extension (E). This decides
   the toolkit, not the other way round.

Reopen the GTK4 question when one of these happens: slice 3 picks
layer-shell; gtk3-rs goes 12 months without a release or stops building on
a current rustc; or another dependency needs a `glib` that gtk3-rs does not
follow. A GTK4 port would then go: prefs first (no X11), drawing via
`set_draw_func` and event controllers, the dock window's X11 properties and
moves via x11rb, menus to `PopoverMenu`, tooltips and previews to popovers,
the keyboard mode, and last the `system` theme.

No issues were opened for these slices; the owner decides.

## Not verified

- Nothing was compiled against GTK4, and nothing was run against 0.19 (only
  `cargo check`).
- That GDK4 tolerates an override-redirect window set from x11rb, or a
  window moved behind its back.
- Whether Mutter honours an Xwayland client's struts and DOCK type in a
  GNOME Wayland session.
- Which Wayland compositors expose a foreign-toplevel protocol `docad` could
  use.

## Sources

- RUSTSEC-2024-0412 (withdrawn 2026-08-14): https://osv.dev/vulnerability/RUSTSEC-2024-0412
- advisory-db, "Withdraw gtk3-rs unmaintained advisories" (#3189, merged 2026-09-08): https://github.com/rustsec/advisory-db/pull/3189
- gtk3-rs repository and 0.19.0 release (2026-09-08): https://github.com/gtk-rs/gtk3-rs/releases/tag/0.19.0
- gtk3-rs, "Update gtk-rs-core deps to 0.22" (#857, MSRV 1.92): https://github.com/gtk-rs/gtk3-rs/pull/857
- crates.io: https://crates.io/crates/gtk, https://crates.io/crates/gtk4, https://crates.io/crates/gdk4-x11, https://crates.io/crates/gtk4-layer-shell, https://crates.io/crates/cairo-rs
- Fedora F43, retire gtk3-rs (accepted 2025-04-10): https://discussion.fedoraproject.org/t/f43-change-proposal-retire-gtk3-rs-gtk-rs-core-v0-18-and-gtk4-rs-v0-7-self-contained/146853
- Fedora `rust-gtk` dead.package: https://src.fedoraproject.org/rpms/rust-gtk
- Fedora F43, Wayland-only GNOME: https://fedoraproject.org/wiki/Changes/WaylandOnlyGNOME
- Ubuntu 25.10 drops GNOME on Xorg: https://discourse.ubuntu.com/t/ubuntu-25-10-drops-support-for-gnome-on-xorg/62538
- Ubuntu release cycle: https://ubuntu.com/about/release-cycle
- Package versions: https://archlinux.org/packages/extra/x86_64/gtk3/, https://sources.debian.org/src/gtk+3.0/, https://sources.debian.org/src/gnome-shell/, https://launchpad.net/ubuntu/+source/gtk+3.0, https://launchpad.net/ubuntu/+source/gnome-shell
- GTK 3.24.52 tag: https://gitlab.gnome.org/GNOME/gtk/-/tags/3.24.52
- GTK, "Migrating from GTK 3.x to GTK 4": https://docs.gtk.org/gtk4/migrating-3to4.html
- GTK blog, "What's new in GTK, winter 2025 edition" (2025-02-01): https://blog.gtk.org/2025/
- `GdkX11Surface`, `GdkX11Display` (deprecated 4.18): https://docs.gtk.org/gdk4-x11/class.X11Surface.html, https://docs.gtk.org/gdk4-x11/class.X11Display.html
- `gdk_surface_set_input_region`: https://docs.gtk.org/gdk4/method.Surface.set_input_region.html
- `gtk_style_context_lookup_color` (deprecated 4.10): https://docs.gtk.org/gtk4/method.StyleContext.lookup_color.html
- `gdk_texture_new_for_pixbuf` (deprecated 4.20): https://docs.gtk.org/gdk4/ctor.Texture.new_for_pixbuf.html
- gtk4-layer-shell: https://github.com/wmww/gtk4-layer-shell
