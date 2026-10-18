mod appearance;
mod dock;
mod fit;
mod ground;
mod magnify;
mod motion;
mod panel;
mod row;
mod stack;
mod strut;
mod theme;
mod tooltip;
mod widget_tile;

use std::cell::{Cell, RefCell};
use std::time::Instant;
use std::rc::Rc;

use anyhow::Result;
use gtk::prelude::*;
use doca_ipc::DocaProxy;
use tracing_subscriber::EnvFilter;

use crate::strut::BottomStrut;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("doca_shell=info")),
        )
        .init();

    gtk::init()?;

    let monitor = gdk::Display::default()
        .and_then(|display| display.primary_monitor())
        .ok_or_else(|| anyhow::anyhow!("no primary monitor"))?;
    let screen = monitor.geometry();

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Doca");
    window.set_type_hint(gdk::WindowTypeHint::Dock);
    window.set_keep_above(true);
    window.stick();
    window.set_decorated(false);
    window.set_resizable(false);
    window.set_skip_taskbar_hint(true);
    window.set_skip_pager_hint(true);
    window.set_app_paintable(true);

    if let Some(visual) = gtk::prelude::WidgetExt::screen(&window).and_then(|s| s.rgba_visual()) {
        window.set_visual(Some(&visual));
    }

    let css = gtk::CssProvider::new();
    css.load_from_data(theme::css(theme::DEFAULT).as_bytes())?;
    gtk::StyleContext::add_provider_for_screen(
        &gtk::prelude::WidgetExt::screen(&window).unwrap(),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // The icons are drawn above the bar rather than inside it, so a magnified
    // one rises out of the top the way it does on a dock that is one drawing.
    // Keeping them inside would mean a bar as tall as the largest icon, with
    // that much empty air over the row whenever the pointer is elsewhere.
    let items = gtk::Box::new(gtk::Orientation::Horizontal, dock::ITEM_SPACING);
    items.set_halign(gtk::Align::Center);
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bar.set_widget_name("bar");
    bar.set_valign(gtk::Align::End);
    // The window is wider than the bar by the room the lens needs at either
    // end, so the bar is placed in it rather than filling it.
    bar.set_halign(gtk::Align::Center);
    bar.pack_start(&items, true, true, 0);
    let stage = gtk::Overlay::new();
    stage.add(&bar);
    window.add(&stage);
    window.show_all();

    glib::spawn_future_local(async move {
        if let Err(e) = drive(window, stage, bar, items, screen, css).await {
            tracing::error!("daemon link failed: {e:#}");
        }
    });

    gtk::main();
    Ok(())
}

type Tiles = Rc<widget_tile::Shelf>;

/// The bar's own movement: where it sits, and the slide between the two places.
///
/// `shown` is where the bar is *going*, not where it is. Where it is comes from
/// `since` and the curve in `motion` — one source of truth, read afresh every
/// frame, so a slide can be turned round halfway without anyone having to know
/// how far along it was.
#[derive(Clone)]
struct Hide {
    enabled: Rc<Cell<bool>>,
    shown: Rc<Cell<bool>>,
    shown_y: Rc<Cell<i32>>,
    height: Rc<Cell<i32>>,
    x: Rc<Cell<i32>>,
    /// When the bar was last told where to go.
    since: Rc<Cell<Instant>>,
    ticking: Rc<Cell<bool>>,
    /// How many things that belong to the dock are open on top of it.
    ///
    /// A menu, a folder grid, a widget panel: while any of them is up the
    /// dock is still being used, whatever the pointer is doing. Counted
    /// rather than flagged because two can be open at once — a context menu
    /// over a grid — and the first to close must not speak for the second.
    holds: Rc<Cell<usize>>,
    /// A hide that was asked for while something was open, kept for when the
    /// last of them closes.
    held_back: Rc<Cell<bool>>,
    /// A move the pointer has asked for and not yet held long enough to get.
    ///
    /// One at a time, and cancelled by the opposite asking: a pointer that
    /// crosses the edge and leaves again has asked for both, and should get
    /// neither.
    intent: Rc<RefCell<Option<glib::SourceId>>>,
}

impl Default for Hide {
    fn default() -> Self {
        Self {
            enabled: Rc::new(Cell::new(false)),
            shown: Rc::new(Cell::new(false)),
            shown_y: Rc::new(Cell::new(0)),
            height: Rc::new(Cell::new(0)),
            x: Rc::new(Cell::new(0)),
            // Far enough back that the bar is already wherever it was sent.
            since: Rc::new(Cell::new(Instant::now() - motion::SLIDE)),
            ticking: Rc::new(Cell::new(false)),
            holds: Rc::new(Cell::new(0)),
            held_back: Rc::new(Cell::new(false)),
            intent: Rc::new(RefCell::new(None)),
        }
    }
}

impl Hide {
    fn place(&self, window: &gtk::Window, x: i32, shown_y: i32, height: i32, enabled: bool) {
        self.x.set(x);
        self.shown_y.set(shown_y);
        self.height.set(height);
        let was_enabled = self.enabled.replace(enabled);
        if !enabled {
            // A move the pointer asked for a moment ago must not land on a
            // bar that has since been told to stay put.
            self.forget_intent();
            self.held_back.set(false);
            self.shown.set(true);
            self.settle();
            window.move_(x, shown_y);
            return;
        }
        if !was_enabled {
            // Auto-hide has just been switched on, and the bar is on screen
            // because until now it had no reason not to be. Sliding it away
            // shows what the toggle did; dropping it there a frame later
            // reads as the dock having crashed.
            self.shown.set(true);
            self.settle();
            window.move_(x, shown_y);
            self.slide(window, false);
            return;
        }
        // A rebuild lands at any moment, the middle of a slide included. The
        // bar has already been told where to go; moving it now would undo the
        // frame the slide just drew.
        if self.ticking.get() {
            return;
        }
        window.move_(x, self.y());
    }

    /// Where the bar is this instant, part-way through a slide or at rest.
    fn y(&self) -> i32 {
        self.y_at(Instant::now())
    }

    /// Where the bar is at `now`. The clock is an argument so the slide can
    /// be walked a frame at a time without a window or a main loop — which
    /// is the only way to check a reversal frame by frame.
    fn y_at(&self, now: Instant) -> i32 {
        let shown_y = self.shown_y.get();
        let hidden = motion::hidden_y(shown_y, self.height.get());
        let elapsed = now.saturating_duration_since(self.since.get());
        let progress = motion::slide_progress(elapsed, self.shown.get());
        motion::slide_y(progress, shown_y, hidden)
    }

    /// Declare the slide over, wherever it was going.
    fn settle(&self) {
        self.since.set(Instant::now() - motion::SLIDE);
    }

    /// Ask for the bar, or ask for it to go, and wait to be sure.
    ///
    /// The pointer crossing the screen edge is not the same event as the user
    /// wanting the dock: the edge is also the way to the bottom of a window
    /// and to nothing at all. So an arrival is held for `motion::REVEAL_AFTER`
    /// before it counts, a departure for the longer `motion::HIDE_AFTER`, and
    /// either one cancels the other outright — a pointer that passes through
    /// asks for both and gets neither, which is the whole of what "intent"
    /// means here.
    fn intend(&self, window: &gtk::Window, to_shown: bool) {
        self.forget_intent();
        if !self.enabled.get() || self.shown.get() == to_shown {
            return;
        }
        if !to_shown && self.holds.get() > 0 {
            // Not refused, deferred: the pointer did leave, and when the last
            // thing in the way closes the bar should still go.
            self.held_back.set(true);
            return;
        }

        let hide = self.clone();
        let window = window.clone();
        let waiting = glib::timeout_add_local_once(motion::intent_delay(to_shown), move || {
            hide.intent.replace(None);
            hide.slide(&window, to_shown);
        });
        self.intent.replace(Some(waiting));
    }

    /// Keep the bar where it is for as long as this menu is up.
    ///
    /// Hiding under an open folder grid leaves the grid floating over
    /// nothing, and closing it then drops the pointer onto a desktop nobody
    /// aimed at. The menu is the dock still being used, so the dock stays.
    fn holds_for(&self, window: &gtk::Window, menu: &gtk::Menu) {
        let holding = self.clone();
        menu.connect_map(move |_| holding.hold());
        let releasing = self.clone();
        let window = window.clone();
        menu.connect_unmap(move |_| releasing.release(&window));
    }

    fn hold(&self) {
        self.holds.set(self.holds.get() + 1);
        // Whatever the pointer had asked for, it asked before this opened.
        self.forget_intent();
    }

    /// One thing fewer in the way. The hide that was waiting on it, if any,
    /// starts over from here rather than landing the moment the menu goes —
    /// the pointer is usually still on the dock, having just used it.
    fn release(&self, window: &gtk::Window) {
        self.holds.set(self.holds.get().saturating_sub(1));
        if self.holds.get() == 0 && self.held_back.replace(false) {
            self.intend(window, false);
        }
    }

    /// Drop a move that was asked for and not yet made.
    fn forget_intent(&self) {
        if let Some(waiting) = self.intent.replace(None) {
            waiting.remove();
        }
    }

    /// Send the bar up or down, from wherever it happens to be.
    ///
    /// Asking for the way it is already going is nothing; asking for the other
    /// way turns it round at the position it is at, and the time left shrinks
    /// with the distance left. Dropping the request instead — which is what
    /// this did — left the pointer inside a bar that stayed hidden until the
    /// mouse moved again.
    fn slide(&self, window: &gtk::Window, to_shown: bool) {
        if self.turn(to_shown, Instant::now()) {
            self.run(window);
        }
    }

    /// Point the slide the other way at `now`, from where it is. `false` when
    /// there was nothing to turn: auto-hide off, or already going that way.
    fn turn(&self, to_shown: bool, now: Instant) -> bool {
        if !self.enabled.get() || self.shown.replace(to_shown) == to_shown {
            return false;
        }
        let elapsed = now.saturating_duration_since(self.since.get());
        self.since
            .set(now - motion::reversed_start(elapsed, motion::SLIDE));
        true
    }

    /// Draw the slide on the compositor's clock, and stop once it has arrived.
    fn run(&self, window: &gtk::Window) {
        if self.ticking.replace(true) {
            return;
        }
        let hide = self.clone();
        let moving = window.clone();
        window.add_tick_callback(move |_, _| {
            moving.move_(hide.x.get(), hide.y());
            if hide.since.get().elapsed() >= motion::SLIDE {
                hide.ticking.set(false);
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }
}

async fn drive(
    window: gtk::Window,
    stage: gtk::Overlay,
    bar: gtk::Box,
    items: gtk::Box,
    screen: gdk::Rectangle,
    css: gtk::CssProvider,
) -> Result<()> {
    let connection = zbus::Connection::session().await?;
    let proxy: Rc<DocaProxy<'static>> = Rc::new(DocaProxy::new(&connection).await?);
    let tiles: Tiles = Rc::new(widget_tile::Shelf::new());
    let applied: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let hide = Hide::default();
    let label = tooltip::Tooltip::new();
    let icons = row::Row::new();
    icons.stage_on(&stage);
    // The background goes on before the bar it belongs to, and follows the
    // icons rather than the layout — see `ground.rs`.
    let ground = Rc::new(ground::Ground::new());
    ground.under(&stage, &bar, &icons);
    window.show_all();
    tracing::info!("connected to docad");

    window.add_events(
        gdk::EventMask::POINTER_MOTION_MASK
            | gdk::EventMask::LEAVE_NOTIFY_MASK
            | gdk::EventMask::ENTER_NOTIFY_MASK,
    );
    let leaving = icons.clone();
    let hiding = hide.clone();
    let hiding_label = label.clone();
    window.connect_leave_notify_event(move |window, event| {
        if event.detail() == gdk::NotifyType::Inferior {
            return glib::Propagation::Proceed;
        }
        leaving.aim(None);
        hiding.intend(window, false);
        hiding_label.hide();
        glib::Propagation::Proceed
    });
    let showing = hide.clone();
    window.connect_enter_notify_event(move |window, _| {
        showing.intend(window, true);
        glib::Propagation::Proceed
    });

    icons.serve(proxy.clone());
    wire(
        &icons,
        &label,
        proxy.clone(),
        stack::Remembered::default(),
        window.clone(),
        hide.clone(),
    );

    let chrome = Chrome {
        style: Style { css, applied },
        row: icons.clone(),
        hide,
        label,
        themes: appearance::Themes::capture(),
        panels: panel::Remembered::default(),
        showing: panel::Showing::default(),
        settling: Rc::new(RefCell::new(None)),
    };

    // A theme switched in GNOME Tweaks arrives as a GtkSettings notification,
    // on the GTK thread, with no way to await a rebuild from there — so the
    // handlers only name the property, and the loop below does the work.
    let (theme_moved, theme_moves) = async_channel::unbounded::<&'static str>();
    let mut theme_moves = Box::pin(theme_moves);
    if let Some(settings) = gtk::Settings::default() {
        for property in [
            appearance::GTK_THEME,
            appearance::ICON_THEME,
            appearance::CURSOR_THEME,
        ] {
            let telling = theme_moved.clone();
            settings.connect_notify_local(Some(property), move |_, _| {
                let _ = telling.send_blocking(property);
            });
        }
    }
    // Whether the desktop wants things to move at all, read now and followed:
    // GNOME's "Reduce animation" flips it while the dock is running, and the
    // lens has to go or come back with it, which is a rebuild.
    let (stillness_moved, stillness_moves) = async_channel::unbounded::<()>();
    let mut stillness_moves = Box::pin(stillness_moves);
    if let Some(settings) = gtk::Settings::default() {
        motion::hold_still(!settings.is_gtk_enable_animations());
        settings.connect_gtk_enable_animations_notify(move |settings| {
            motion::hold_still(!settings.is_gtk_enable_animations());
            let _ = stillness_moved.send_blocking(());
        });
    }
    rebuild(&window, &bar, &items, &screen, proxy.clone(), tiles.clone(), &chrome).await;

    let mut items_changed = proxy.receive_items_changed().await?;
    let mut environment_changed = proxy.receive_environment_changed().await?;
    let mut widget_changed = proxy.receive_widget_changed().await?;
    let mut config_changed = proxy.receive_config_changed().await?;
    let mut item_activated = proxy.receive_item_activated().await?;

    loop {
        futures_util::select! {
            _ = futures_util::StreamExt::next(&mut items_changed) => {}
            _ = futures_util::StreamExt::next(&mut environment_changed) => {}
            // The config was written, so the look may have moved under us.
            // A rebuild re-reads `appearance()` and reapplies all of it —
            // theme, icon size, lens, auto-hide, strut — on the window that
            // is already on screen, which is what makes a preferences window
            // possible without asking anyone to restart the dock.
            _ = futures_util::StreamExt::next(&mut config_changed) => {}
            _ = futures_util::StreamExt::next(&mut stillness_moves) => {}
            property = futures_util::StreamExt::next(&mut theme_moves) => {
                let Some(property) = property else { continue };
                let Some(themes) = &chrome.themes else { continue };
                // Tell a change the desktop made from one we made ourselves:
                // only the desktop's changes what "follow the system" means.
                themes.absorb(property);
                let redo = appearance::redo_for(property, &chrome.style.applied.borrow().clone());
                if !redo.anything() {
                    continue;
                }
                if redo.stylesheet {
                    chrome.style.invalidate();
                }
                if redo.icons {
                    chrome.row.reload_icons();
                }
            }
            signal = futures_util::StreamExt::next(&mut item_activated) => {
                // A key chose an item. It gets the bounce a click gets, by
                // the same rule — only when it started something — and
                // nothing else about the bar moves.
                if let Some(args) = signal.as_ref().and_then(|signal| signal.args().ok()) {
                    if args.starting {
                        if let Some(index) = chrome.row.index_of(args.id) {
                            chrome.row.launch(index);
                        }
                    }
                }
                continue;
            }
            signal = futures_util::StreamExt::next(&mut widget_changed) => {
                // A widget's new value touches its own tile and nothing
                // else — never the row of icons, which is the tightest thing
                // this loop has to do. A widget the bar is not showing has no
                // tile and nothing to act on here: what puts it on the bar is
                // the *list* changing, which arrives as another signal.
                if let Some(signal) = signal {
                    if let Ok(args) = signal.args() {
                        tiles.update(&args.state);
                        // And the panel, when it is this widget's that is
                        // up: a panel outlives its controls, so it has to
                        // show what they did.
                        chrome.showing.update(&args.state);
                    }
                }
                continue;
            }
            complete => break,
        }
        rebuild(&window, &bar, &items, &screen, proxy.clone(), tiles.clone(), &chrome).await;
    }
    Ok(())
}

struct Style {
    css: gtk::CssProvider,
    applied: Rc<RefCell<String>>,
}

/// Clicks, labels and dropped files, by where they land on the row.
///
/// The row is one widget, so there is no per-icon handler to hang these on:
/// every event arrives with an x, and the row says which icon is drawn there.
/// Name whatever the pointer is on, or take the name away if it is on nothing.
fn name_what_is_hovered(icons: &row::Row, label: &tooltip::Tooltip) {
    let Some((index, item)) = icons.hovered() else {
        label.hide();
        return;
    };
    let origin = icons
        .area
        .toplevel()
        .and_then(|top| top.window())
        .map(|window| window.position())
        .unwrap_or((0, 0));
    let allocation = icons.area.allocation();
    let (left, width) = icons.rect_of(index);
    label.point_at(
        (origin.0 + allocation.x() + left, origin.1 + allocation.y()),
        width,
        &item.name,
    );
}

fn wire(
    icons: &row::Row,
    label: &tooltip::Tooltip,
    proxy: Rc<DocaProxy<'static>>,
    folders: stack::Remembered,
    window: gtk::Window,
    hide: Hide,
) {
    let naming = icons.clone();
    let naming_label = label.clone();
    icons.area.connect_motion_notify_event(move |_, event| {
        naming.aim(Some(event.position().0));
        name_what_is_hovered(&naming, &naming_label);
        glib::Propagation::Proceed
    });

    let leaving = icons.clone();
    let leaving_label = label.clone();
    icons.area.connect_leave_notify_event(move |_, _| {
        leaving.aim(None);
        leaving_label.hide();
        glib::Propagation::Proceed
    });

    let clicked = icons.clone();
    let clicked_label = label.clone();
    icons.area.connect_button_press_event(move |_, event| {
        clicked_label.hide();
        let Some((index, item)) = clicked.item_at(event.position().0) else {
            return glib::Propagation::Stop;
        };
        let proxy = proxy.clone();
        let trigger = event.clone();

        match event.button() {
            1 if stack::is_folder(&item.id) => {
                // The grid goes up on the click, not on the answer: reading a
                // folder is the one thing a click can start that has no
                // bounded cost — a sleeping disk, a mount that is not local —
                // and nothing on the drawing path may wait on it.
                let expected = folders.of(&item.id);
                let grid = stack::opening(expected);
                hide.holds_for(&window, &grid);
                // One closure opens the grid and, if what arrives does not fit
                // what it was opened for, opens it again. It anchors on the
                // icon rather than on the click: a grid that has to come back
                // comes back a moment later, and an event that old no longer
                // places a menu — it put the second one in the screen's top
                // corner, and sometimes failed to put it up at all. The icon
                // is still where it was, and a stack belongs over the thing it
                // belongs to anyway.
                let show = over(&clicked, index);
                show(&grid);
                let opened = Instant::now();
                let folders = folders.clone();
                glib::spawn_future_local(async move {
                    let entries = proxy.list_folder(&item.id).await.unwrap_or_default();
                    folders.note(&item.id, entries.len());
                    stack::fill(
                        &grid,
                        expected,
                        &entries,
                        &opens_with(proxy.clone()),
                        &show,
                        opened,
                    );
                });
            }
            1 => {
                if item.windows.is_empty() && !stack::is_folder(&item.id) {
                    clicked.launch(index);
                }
                glib::spawn_future_local(async move {
                    let _ = proxy.activate_item(&item.id).await;
                });
            }
            3 => {
                let (holding, under) = (hide.clone(), window.clone());
                glib::spawn_future_local(async move {
                    let windows = proxy.item_windows(&item.id).await.unwrap_or_default();
                    let menu = dock::context_menu(&item, &windows, proxy.clone());
                    holding.holds_for(&under, &menu);
                    menu.popup_at_pointer(Some(&trigger));
                });
            }
            _ => {}
        }
        glib::Propagation::Stop
    });

    dock::accept_file_drops(icons);
}

/// Put a menu above the icon it belongs to.
///
/// The row is one widget, so there is no per-icon window to anchor on: the
/// icon's place is a rectangle inside the row's, which is what `rect_of`
/// answers and what the tooltip already points at.
fn over(icons: &row::Row, index: usize) -> stack::Show {
    let (left, width) = icons.rect_of(index);
    let height = icons.area.allocated_height();
    let window = icons.area.window();
    Rc::new(move |menu: &gtk::Menu| {
        let Some(window) = &window else {
            menu.popup_at_pointer(None);
            return;
        };
        menu.popup_at_rect(
            window,
            &gdk::Rectangle::new(left, 0, width, height),
            gdk::Gravity::North,
            gdk::Gravity::South,
            None,
        );
    })
}

/// Opening a path is the daemon's job, so that is all the grid is handed.
fn opens_with(proxy: Rc<DocaProxy<'static>>) -> stack::Open {
    Rc::new(move |path: &str| {
        let path = path.to_string();
        let proxy = proxy.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = proxy.open_path(&path).await {
                tracing::warn!("cannot open {path}: {e}");
            }
        });
    })
}

/// The pieces of the bar that outlive any one rebuild.
struct Chrome {
    style: Style,
    row: row::Row,
    hide: Hide,
    label: tooltip::Tooltip,
    /// None when GTK has no settings to override, which no real session is.
    themes: Option<appearance::Themes>,
    /// How big each widget's panel was the last time it was opened, so the
    /// next open does not blink — see `panel::Remembered`.
    panels: panel::Remembered,
    /// The panel that is up, so a widget's news reaches it as well as the
    /// tile.
    showing: panel::Showing,
    /// The bar's second seating, once what a rebuild set moving has stopped.
    /// One at a time: a rebuild that lands mid-change starts the wait over.
    settling: Rc<RefCell<Option<glib::SourceId>>>,
}

impl Style {
    fn apply(&self, requested: &str, context: &gtk::StyleContext) {
        let resolved = self.usable(theme::resolve(requested), context);
        if *self.applied.borrow() == resolved {
            return;
        }
        match self.css.load_from_data(theme::css(resolved).as_bytes()) {
            Ok(()) => {
                if requested != resolved {
                    tracing::warn!("unknown theme {requested}, using {resolved}");
                }
                tracing::info!(theme = resolved, "theme applied");
                *self.applied.borrow_mut() = resolved.to_string();
            }
            // A sheet that fails to load fails whole, leaving the bar with no
            // style at all — which `system` can do on a GTK theme that names
            // none of the colours it borrows. A dock in the wrong colours is
            // worth having; an unstyled one is not.
            Err(e) => {
                tracing::error!("theme {resolved} failed to load: {e}");
                if resolved == theme::DEFAULT {
                    return;
                }
                match self.css.load_from_data(theme::css(theme::DEFAULT).as_bytes()) {
                    Ok(()) => {
                        tracing::warn!("{resolved} did not load, falling back to {}", theme::DEFAULT);
                        *self.applied.borrow_mut() = theme::DEFAULT.to_string();
                    }
                    Err(e) => tracing::error!("even {} failed to load: {e}", theme::DEFAULT),
                }
            }
        }
    }

    /// The sheet to actually load, which is not always the one asked for.
    ///
    /// `system` borrows its colours from the GTK theme, and GTK will happily
    /// load a sheet naming colours the theme never defined — leaving a
    /// translucent dock with an invisible bar. Checking first is the only way
    /// to catch that, because nothing fails.
    fn usable(&self, resolved: &'static str, context: &gtk::StyleContext) -> &'static str {
        if resolved != theme::SYSTEM {
            return resolved;
        }
        let missing = theme::missing_colours(context);
        if missing.is_empty() {
            return resolved;
        }
        tracing::warn!(
            missing = missing.join(", "),
            "the GTK theme does not define the colours the system theme borrows, using {}",
            theme::DEFAULT
        );
        theme::DEFAULT
    }

    /// Forget which sheet is loaded, so the next apply loads it again.
    ///
    /// The cache exists because a rebuild happens several times a minute and
    /// the theme almost never moves. `system` is the exception: the sheet is
    /// the same string, and the colours it reads are not.
    fn invalidate(&self) {
        self.applied.borrow_mut().clear();
    }
}

/// What a click on a tile does: open that widget's panel over it.
///
/// The panel goes up in the frame the click landed in, and the daemon is
/// asked for what goes in it afterwards — the same order the folder grid
/// opens in, and for the same reason: a widget's contents are a question
/// whose cost has no ceiling, and the shell can put none on it.
fn expanding(
    window: gtk::Window,
    proxy: Rc<DocaProxy<'static>>,
    panels: panel::Remembered,
    showing: panel::Showing,
    hide: Hide,
) -> widget_tile::Expand {
    let invoke: panel::Invoke = {
        let asking = proxy.clone();
        Rc::new(move |id: &str, action: &str| {
            let (id, action) = (id.to_string(), action.to_string());
            let proxy = asking.clone();
            glib::spawn_future_local(async move {
                if let Err(e) = proxy.invoke_widget(&id, &action).await {
                    tracing::warn!("widget {id} rejected {action}: {e}");
                }
            });
        })
    };

    // Typing into a row is a setting written, not an action invoked — a
    // different shape, and the daemon validates the pair.
    let write: panel::Write = {
        let asking = proxy.clone();
        Rc::new(move |id: &str, text: &str| {
            let (id, text) = (id.to_string(), text.to_string());
            let proxy = asking.clone();
            glib::spawn_future_local(async move {
                let value = zbus::zvariant::Value::from(text).try_to_owned();
                let Ok(value) = value else {
                    tracing::warn!("cannot send the {id} text");
                    return;
                };
                if let Err(e) = proxy
                    .set_widget_setting(&id, doca_ipc::widget_key::TEXT, value)
                    .await
                {
                    tracing::warn!("widget {id} refused its text: {e}");
                }
            });
        })
    };

    // A setting picked in a panel goes the way the preferences window sends
    // one, so the file, the window and the panel cannot disagree.
    let choose: panel::Choose = {
        let asking = proxy.clone();
        Rc::new(move |id: &str, key: &str, value: &str| {
            let (id, key, value) = (id.to_string(), key.to_string(), value.to_string());
            let proxy = asking.clone();
            glib::spawn_future_local(async move {
                let Ok(owned) = zbus::zvariant::Value::from(value).try_to_owned() else {
                    tracing::warn!("cannot send {id}.{key}");
                    return;
                };
                if let Err(e) = proxy.set_widget_setting(&id, &key, owned).await {
                    tracing::warn!("widget {id} refused {key}: {e}");
                }
            });
        })
    };

    let asks = panel::Asks { invoke, write, choose };

    Rc::new(move |id: &str, over: &gtk::Widget| {
        let menu = panel::opening(panels.of(id));
        hide.holds_for(&window, &menu);

        // Over the tile that asked, and the same way every time it is
        // reopened — including the reopen `panel::fill` does when the panel
        // turns out to be a different shape than it was opened for.
        let show = panel::above(over);
        show(&menu);
        let opened = std::time::Instant::now();

        // The focus and the auto-hide, both of which the grab takes charge of
        // while the panel is up.
        //
        // A menu's grab is what carries keys over a window of type DOCK, and
        // handing it back is what returns the keyboard to the bar. What it
        // does not hand back is the bar's own idea of where the pointer is:
        // the window gets no leave event for a pointer that went to the menu
        // and none when the menu closes either, so a bar that auto-hides
        // would sit there shown for ever. Asking again on close is what makes
        // auto-hide work after a panel rather than only before one.
        //
        // On the menu being closed, not on it being hidden: a panel that
        // turns out to be a different shape is taken down and put up again
        // by `panel::fill`, and that is a hide too — one after which the bar
        // slid away from under the panel still on it, and the panel stopped
        // hearing the widget's news. Escape, a click elsewhere and a lost
        // grab all close a menu by deactivating it; taking it down from code
        // does not.
        let closing = hide.clone();
        let dock = window.clone();
        let back_to = over.clone();
        let going = showing.clone();
        let gone = id.to_string();
        menu.connect_deactivate(move |_| {
            going.gone(&gone);
            back_to.grab_focus();
            closing.slide(&dock, pointer_is_over(&dock));
        });

        // What the rows are once they are in. None until the daemon has
        // answered: news that arrives before then is already in the answer
        // the panel is waiting for.
        let live: Rc<RefCell<Option<panel::Live>>> = Rc::default();
        {
            let (menu, live, asks, show, panels) =
                (menu.clone(), live.clone(), asks.clone(), show.clone(), panels.clone());
            let widget = id.to_string();
            showing.now(
                id,
                Rc::new(move |state: &doca_ipc::WidgetState| {
                    let Some(current) = live.borrow().clone() else { return };
                    let rows = panel::rows_for(state);
                    if current.refresh(&rows) {
                        return;
                    }
                    // A different shape — a cover that arrived — is drawn
                    // again, which may move the panel; that is the price of a
                    // row that was not there.
                    let refilled = panel::fill(&menu, &widget, &rows, &asks, &show, opened);
                    panels.note(&widget, &menu);
                    live.replace(Some(refilled));
                }),
            );
        }

        let filling = menu.clone();
        let proxy = proxy.clone();
        let asks = asks.clone();
        let panels = panels.clone();
        let id = id.to_string();
        glib::spawn_future_local(async move {
            // Asked again rather than taken from the tile: a panel shows what
            // the widget is now, and the tile's state is as old as the last
            // signal. A widget the daemon no longer has leaves the rows to
            // the id alone, which is still a panel with working controls.
            let states = proxy.list_widgets().await.unwrap_or_default();
            let rows = match states.iter().find(|state| state.id == id) {
                Some(state) => panel::rows_for(state),
                None => {
                    // A widget that was dropped between the click and the
                    // answer. `fill` says so rather than leaving a skeleton
                    // on screen for ever.
                    tracing::debug!("the daemon no longer has {id}");
                    Vec::new()
                }
            };
            let filled = panel::fill(&filling, &id, &rows, &asks, &show, opened);
            panels.note(&id, &filling);
            live.replace(Some(filled));
        });
    })
}

/// Whether the pointer is inside this window this instant.
///
/// Asked rather than remembered, because the thing that would have remembered
/// it — the window's own enter and leave events — is exactly what a menu's
/// grab takes away.
fn pointer_is_over(window: &gtk::Window) -> bool {
    let Some(surface) = window.window() else {
        return false;
    };
    let Some(pointer) = gdk::Display::default()
        .and_then(|display| display.default_seat())
        .and_then(|seat| seat.pointer())
    else {
        return false;
    };
    surface.device_position(&pointer).0.is_some()
}

async fn rebuild(
    window: &gtk::Window,
    bar: &gtk::Box,
    items: &gtk::Box,
    screen: &gdk::Rectangle,
    proxy: Rc<DocaProxy<'static>>,
    tiles: Tiles,
    chrome: &Chrome,
) {
    let Chrome {
        style,
        row,
        hide,
        label,
        themes,
        panels,
        showing,
        settling,
    } = chrome;
    let appearance = proxy.appearance().await.ok();
    if let Some(appearance) = &appearance {
        // The overrides go first: the `system` sheet reads named colours out
        // of the GTK theme in force, so loading it before switching themes
        // would read them from the one on its way out.
        if let Some(themes) = themes {
            themes.apply(appearance);
        }
        style.apply(&appearance.theme, &row.area.style_context());
    }
    let preferred_icon = appearance
        .as_ref()
        .map(|appearance| appearance.icon_size)
        .unwrap_or(dock::ICON_SIZE);
    let auto_hide = appearance
        .as_ref()
        .map(|appearance| appearance.auto_hide)
        .unwrap_or(false);
    // No lens at all when the desktop asked for no animation — not a lens
    // that snaps open, which is still an icon jumping under the pointer.
    let magnification = if motion::still() {
        1.0
    } else {
        appearance
            .as_ref()
            .map(|appearance| appearance.magnification)
            .unwrap_or(magnify::DEFAULT_SCALE)
    };

    let entries = proxy.list_items().await.unwrap_or_default();
    let widgets = proxy.list_widgets().await.unwrap_or_default();

    // The size the tiles are drawn at: what the config asked for, clamped,
    // and not the size the row of icons settles on. `widget_tile::room_for`
    // says why — the row's size is worked out from the room the tiles leave,
    // so sizing the tiles from the row's would be circular.
    let tile_icon = preferred_icon.clamp(dock::MIN_ICON_SIZE, dock::MAX_ICON_SIZE);

    // Everything but the row and the shelf is torn down and built again. The
    // row stays put: unparenting a widget destroys its window, and the pointer
    // leaving a window that was taken out from under it is a leave event like
    // any other — which shut the lens every time a window somewhere took
    // focus. The shelf stays for the same reason and one more: its tiles are
    // reconciled rather than remade, so sweeping them away here would undo
    // that before it happened.
    for child in items.children() {
        if child != row.perch.clone().upcast::<gtk::Widget>() && !tiles.owns(&child) {
            items.remove(&child);
        }
    }
    if row.perch.parent().is_none() {
        items.add(&row.perch);
    }

    // What the bar gives up to fit the screen, and in what order — the gap
    // first, then the icons, then how much of itself a tile draws. `fit` has
    // the ladder and the reason for its order; here it is only read.
    let shapes: Vec<doca_ipc::Tile> = widgets
        .iter()
        .map(|state| state.tile().unwrap_or(doca_ipc::Tile::Wide))
        .collect();
    let fitted = fit::fits(
        entries.len() as i32,
        &shapes,
        screen.width(),
        preferred_icon,
        magnification,
    );
    if !fitted.gave_nothing_up(preferred_icon) {
        tracing::debug!(
            icon = fitted.icon,
            spacing = fitted.spacing,
            tiles = ?fitted.tiles,
            overflow = fitted.overflow,
            "the bar gave way to fit the screen"
        );
    }
    items.set_spacing(fitted.spacing);

    if entries.is_empty() {
        row.fill(&[], row::Rest::default());
        let empty = gtk::Label::new(Some("nothing running, nothing pinned"));
        empty.set_widget_name("empty");
        items.add(&empty);
    } else {
        // The last rung of the ladder: what fits goes on the row, and what
        // does not goes behind the control beside it. `fit::fits` already
        // counted that control as a slot, which is why `shown` leaves room
        // for it.
        let (on_the_bar, over) = entries.split_at(fitted.shown.max(0) as usize);
        row.fill(
            on_the_bar,
            row::Rest::new(
                fitted.icon,
                fitted.spacing,
                dock::ITEM_PADDING,
                magnification,
            ),
        );
        if !over.is_empty() {
            tracing::info!(
                overflow = over.len(),
                shown = on_the_bar.len(),
                "more apps than there is bar; the rest are behind the control"
            );
            let control = dock::overflow_control(over.len(), fitted.icon);
            let (over, asking) = (over.to_vec(), proxy.clone());
            let (holding, under) = (hide.clone(), window.clone());
            let icon_size = fitted.icon;
            let activate: stack::Open = Rc::new(move |id: &str| {
                let (id, proxy) = (id.to_string(), asking.clone());
                glib::spawn_future_local(async move {
                    if let Err(e) = proxy.activate_item(&id).await {
                        tracing::warn!("cannot activate {id}: {e}");
                    }
                });
            });
            control.connect_button_press_event(move |control, _| {
                let grid = dock::overflow_grid(&over, icon_size, &activate);
                holding.holds_for(&under, &grid);
                grid.popup_at_widget(
                    control,
                    gdk::Gravity::NorthWest,
                    gdk::Gravity::SouthWest,
                    None,
                );
                glib::Propagation::Stop
            });
            items.add(&control);
        }
        // The row draws its own dots, so for `system` they are looked up as
        // values rather than read from the sheet.
        let (running, active) =
            theme::dots_for(&style.applied.borrow(), &row.area.style_context());
        row.dot_colours(running, active);
    }

    // The dock may have changed under a pointer that never moved.
    name_what_is_hovered(row, label);

    let expand = expanding(
        window.clone(),
        proxy.clone(),
        panels.clone(),
        showing.clone(),
        hide.clone(),
    );
    tiles.show(
        items,
        &widgets,
        widget_tile::Look {
            icon_size: tile_icon,
            palette: theme::palette_for(&style.applied.borrow(), &row.area.style_context()),
            level: fitted.tiles,
        },
        &expand,
    );

    // The row is drawn on a surface wider than the place it holds, and the
    // surplus hangs off both ends. Off the left end and, with no tiles, off
    // the right, it hangs into the window, where there is no widget to get in
    // its way — the bar's background follows it out there rather than being
    // covered by anything. The tiles are packed after the row, so when there
    // are any the room on that side has to be held inside the bar instead:
    // otherwise the drawing lies over the first tile and swallows the clicks
    // meant for it. Which end the row's background measures from changes with
    // it, and `ground::around` takes both from the bar.
    row.perch.set_margin_end(if widgets.is_empty() {
        0
    } else {
        row.margin()
    });

    items.show_all();
    // After `show_all`, which shows every child — including a progress bar a
    // tile had hidden for having no progress to report.
    tiles.refresh(&widgets);

    let width = seat(window, bar, screen, row, &tiles, hide, auto_hide);

    // A dock switched for another, or an app opened or closed, is still
    // moving when the rebuild is over: the bar was seated wide enough for
    // the whole of the change, and is seated again at its own size once the
    // change is done. Shrinking the window then trims air: it is centred,
    // and so is the bar in it, so nothing the eye is following moves. The
    // one exception is an X server with no compositor, which shows a window
    // at its new place a frame before GTK has drawn it there — the same
    // frame every rebuild that changed the bar's width has always cost, now
    // paid once after the movement instead of in the middle of it.
    if let Some(waiting) = settling.replace(None) {
        waiting.remove();
    }
    if row.changing() || tiles.changing() {
        let (window, bar, screen) = (window.clone(), bar.clone(), *screen);
        let (row, tiles, hide) = (row.clone(), tiles.clone(), hide.clone());
        let clearing = settling.clone();
        let waiting = glib::timeout_add_local_once(motion::ENTRY, move || {
            clearing.replace(None);
            row.settle();
            tiles.settle();
            seat(&window, &bar, &screen, &row, &tiles, &hide, auto_hide);
        });
        settling.replace(Some(waiting));
    }

    tracing::debug!(items = entries.len(), width, "rebuilt");
}

/// Size the window to what is on the bar, put it on the screen, and reserve
/// the edge it stands on. Answers with the width the bar was given.
///
/// Measured rather than worked out, and measured at the widest the bar will
/// be before what is changing on it has finished: an icon arriving eases out
/// while one leaving eases in, so halfway through a dock switch the two
/// together take more room than either dock does. A window narrower than
/// that is one the bar outgrows mid-switch, and a window that grows under a
/// bar it was centred for pushes the bar off centre — a jump made by the
/// thing meant to stop them.
fn seat(
    window: &gtk::Window,
    bar: &gtk::Box,
    screen: &gdk::Rectangle,
    row: &row::Row,
    tiles: &Tiles,
    hide: &Hide,
    auto_hide: bool,
) -> i32 {
    // The bar is as wide as its contents ask; the window is that plus the room
    // a magnified icon rises into and spreads into, which the bar is never
    // *laid out* over. Keeping that room out of the layout is what makes the
    // background hug the icons instead of reaching to both edges of the
    // screen; painting into it when the lens reaches out there is a separate
    // matter and `ground.rs`'s.
    let margin = row.margin();
    let natural = dock::natural_size(bar);
    let widest = (natural.0 + row.unfolding() + tiles.unfolding(), natural.1);
    let (width, bar_height) = dock::clamp_to_screen(
        widest,
        (screen.width() - margin * 2, screen.height()),
    );
    let height = bar_height + row.overhead();
    if (width, bar_height) != widest {
        tracing::warn!(
            natural_width = widest.0,
            natural_height = widest.1,
            width,
            bar_height,
            "bar clamped to the screen"
        );
    }
    // Pinned only once nothing on it is moving. A bar held at the width it
    // was seated at would hold that width for the whole of a change — the
    // room an app gave up kept until the end and then let go of in a frame,
    // which is the jump the row now goes out of its way not to make.
    if !row.changing() && !tiles.changing() {
        bar.set_size_request(width, -1);
    }
    let window_width = width + margin * 2;
    window.set_size_request(window_width, height);
    window.resize(window_width, height);

    let x = screen.x() + (screen.width() - window_width) / 2;
    let y = screen.y() + screen.height() - height;
    hide.place(window, x, y, height, auto_hide);

    let Some(gdk_window) = window.window() else {
        tracing::warn!("window is not realised yet, strut skipped");
        return width;
    };
    let Ok(x11_window) = gdk_window.downcast::<gdkx11::X11Window>() else {
        tracing::warn!("not an X11 window, strut skipped");
        return width;
    };
    // Only the bar takes the screen edge. The room above it is air a magnified
    // icon passes through, and the room either side of it is air one spreads
    // into; reserving either would hold screen back for a magnification
    // nobody is looking at. So the strut is the bar, not the window it is in.
    let bar_x = x + margin;
    let reserved = BottomStrut {
        height: strut::reserved_height(bar_height, auto_hide),
        start_x: bar_x.max(0) as u32,
        end_x: (bar_x + width).max(0) as u32,
    };
    let xid = x11_window.xid() as u32;
    match strut::apply(xid, &reserved) {
        Ok(()) => tracing::info!(xid, height, x, width, "strut applied"),
        Err(e) => tracing::error!("strut failed: {e:#}"),
    }
    width
}

/// The bar's own slide, walked a frame at a time on a clock the test holds.
///
/// `motion` already checks that a reversal lands where the slide was, at the
/// instant it turns. What it cannot check is the bar: that `Hide` asks the
/// curve the right question, at the right time, from the right start — a
/// `since` set a frame late, or from `Instant::now()` where it meant the
/// moment of turning, lands nowhere near where the curve says.
#[cfg(test)]
mod slide_tests {
    use super::*;
    use std::time::Duration;

    const SHOWN_Y: i32 = 900;
    const HEIGHT: i32 = 84;

    /// A bar on auto-hide, at rest where `shown` says, and told at `at` to
    /// go the other way.
    fn sent(shown: bool, at: Instant) -> Hide {
        let hide = Hide::default();
        hide.enabled.set(true);
        hide.shown_y.set(SHOWN_Y);
        hide.height.set(HEIGHT);
        hide.shown.set(shown);
        hide.since.set(at - motion::SLIDE);
        assert!(hide.turn(!shown, at), "the bar would not move");
        hide
    }

    /// The furthest an ordinary slide travels between two frames, either way.
    ///
    /// The yardstick and not a constant: the curve is fastest at the end
    /// nearest the screen edge, and a turn is allowed whatever a frame of
    /// that already costs — but not one pixel more.
    fn an_ordinary_frame() -> i32 {
        let start = Instant::now();
        [true, false]
            .into_iter()
            .map(|shown| {
                let hide = sent(shown, start);
                (1..=motion::SLIDE.as_millis() as u32 / motion::FRAME.as_millis() as u32 + 1)
                    .map(|frame| {
                        let before = hide.y_at(start + motion::FRAME * (frame - 1));
                        let after = hide.y_at(start + motion::FRAME * frame);
                        (after - before).abs()
                    })
                    .max()
                    .unwrap_or(0)
            })
            .max()
            .unwrap_or(0)
    }

    /// Walk a slide that turns at each of `turns`, one frame at a time, and
    /// answer with the largest step between two frames and where it ended.
    fn walk(shown: bool, turns: &[Duration]) -> (i32, i32) {
        let start = Instant::now();
        let hide = sent(shown, start);
        let mut going = !shown;
        let mut turns = turns.iter().map(|after| start + *after).peekable();
        let last = turns.clone().last().unwrap_or(start);
        let mut now = start;
        let mut at = hide.y_at(now);
        let mut widest = 0;
        while now < last + motion::SLIDE * 2 {
            let next = now + motion::FRAME;
            // A turn lands between two frames, on the clock the pointer
            // keeps rather than the one the compositor does.
            while let Some(turn) = turns.next_if(|turn| *turn <= next) {
                going = !going;
                assert!(hide.turn(going, turn), "the turn was refused");
            }
            let then = hide.y_at(next);
            widest = widest.max((then - at).abs());
            at = then;
            now = next;
        }
        (widest, at)
    }

    /// The case P0.1 was opened about: the pointer comes back while the bar
    /// is on its way down, at any moment of the way.
    #[test]
    fn turning_the_bar_round_mid_slide_never_jumps_further_than_a_frame_does() {
        let ordinary = an_ordinary_frame();
        let hidden = motion::hidden_y(SHOWN_Y, HEIGHT);
        for shown in [true, false] {
            for ms in 0..=motion::SLIDE.as_millis() as u64 {
                let (widest, ended) = walk(shown, &[Duration::from_millis(ms)]);
                assert!(
                    widest <= ordinary,
                    "turning round {ms}ms into a slide jumped {widest}px, where a frame moves {ordinary}px"
                );
                assert_eq!(
                    ended,
                    if shown { SHOWN_Y } else { hidden },
                    "turned round at {ms}ms, the bar never got back"
                );
            }
        }
    }

    /// A hand that cannot make up its mind: down, up, down again inside one
    /// slide's worth of time. Every turn starts from where the last one had
    /// got to, so no turn may cost more than a frame either.
    #[test]
    fn turning_round_again_and_again_never_jumps_either() {
        let ordinary = an_ordinary_frame();
        for first in (0..=motion::SLIDE.as_millis() as u64).step_by(7) {
            for gap in (5..=motion::SLIDE.as_millis() as u64).step_by(11) {
                let turns = [
                    Duration::from_millis(first),
                    Duration::from_millis(first + gap),
                    Duration::from_millis(first + gap * 2),
                ];
                let (widest, ended) = walk(true, &turns);
                assert!(
                    widest <= ordinary,
                    "turns at {turns:?} jumped {widest}px, where a frame moves {ordinary}px"
                );
                // Down, then up, down and up again: it ends where the
                // pointer last asked for it, on screen.
                assert_eq!(ended, SHOWN_Y, "after three turns the bar was not where it was sent");
            }
        }
    }

    #[test]
    fn a_frame_of_slide_is_a_real_distance_and_not_the_whole_way() {
        let ordinary = an_ordinary_frame();
        let travel = motion::hidden_y(SHOWN_Y, HEIGHT) - SHOWN_Y;

        assert!(ordinary > 0, "the yardstick measured a bar that never moved");
        assert!(
            ordinary < travel / 2,
            "a frame covers {ordinary}px of {travel}: the yardstick would excuse a jump"
        );
    }
}

/// The checks that need a real X display, run in the order GTK demands.
///
/// GTK belongs to the thread that starts it, and the test harness hands each
/// test a thread of its own — so these cannot each be a `#[test]`. They live
/// beside the code they check and are called from here, where GTK is started
/// once.
///
/// ```text
/// DISPLAY=:9 cargo test -- --ignored
/// ```
#[cfg(test)]
#[test]
#[ignore = "needs an X display; run inside scripts/dev-session.sh"]
fn on_a_display() {
    gtk::init().expect("no X display");

    hide_tests::a_pointer_only_passing_through_does_not_summon_the_bar();
    hide_tests::a_bar_held_still_is_where_it_was_sent_at_once();
    hide_tests::a_bar_turned_round_mid_slide_never_jumps();
    hide_tests::a_pointer_that_stays_is_an_arrival();
    hide_tests::a_hand_that_overshoots_and_comes_back_keeps_the_bar();
    hide_tests::an_open_menu_keeps_the_bar_where_it_is();
    hide_tests::the_first_menu_to_close_does_not_release_the_second();
    hide_tests::switching_auto_hide_off_drops_a_move_nobody_wants_any_more();
    dock::tests::every_app_that_did_not_fit_is_in_the_grid();
    dock::tests::the_control_says_how_many_it_is_hiding();
    dock::tests::measuring_a_bar_twice_gives_the_same_answer_both_times();
    fit::tests::the_model_never_promises_room_the_bar_does_not_have();
    ground::tests::the_ground_is_painted_in_the_colours_the_stylesheet_names();
    tooltip::tests::a_label_is_the_size_of_its_own_words();
    row::tests::a_pointer_that_left_is_not_pointing_at_anything();
    row::tests::a_pointer_that_comes_back_elsewhere_travels_rather_than_teleports();
    row::tests::a_lens_that_is_shut_aims_at_once_rather_than_travelling();
    row::tests::an_app_that_closed_keeps_its_place_until_it_has_finished_going();
    row::tests::an_app_that_opened_grows_into_the_room_rather_than_appearing_in_it();
    row::tests::a_row_that_was_resized_or_reloaded_is_not_a_row_six_apps_just_opened_on();
    row::tests::a_switch_that_took_the_icons_down_a_size_still_carries_the_shared_apps_over();
    row::tests::an_app_that_closed_and_opened_again_comes_back_from_where_it_had_got_to();
    stack::tests::a_grid_opens_before_the_folder_has_been_read();
    stack::tests::a_folder_opened_at_the_shape_it_turns_out_to_have_does_not_move();
    stack::tests::a_folder_larger_than_the_grid_it_opened_is_still_shown_whole();
    stack::tests::a_folder_with_nothing_in_it_says_so_rather_than_waiting_for_ever();
    stack::tests::a_folder_opened_before_is_drawn_at_the_size_it_was();
    stack::tests::a_grid_always_has_a_cell_to_show_however_little_is_expected();
    widget_tile::tests::a_note_is_readable_on_every_paper_it_offers();
    widget_tile::tests::a_note_looks_the_same_whatever_the_desktop_is_wearing();
    widget_tile::tests::a_cover_of_any_colour_is_made_dark_enough_to_write_on();
    widget_tile::tests::words_on_a_cover_are_white_whatever_the_theme_is();
    widget_tile::tests::the_bottle_holds_as_much_as_the_day_has_in_it();
    widget_tile::tests::a_calendar_tile_keeps_its_date_and_gives_up_its_list();
    widget_tile::tests::showing_the_same_widgets_again_keeps_the_very_same_tiles();
    widget_tile::tests::a_widget_that_went_takes_its_tile_off_the_bar();
    widget_tile::tests::a_widget_that_joined_grows_onto_the_bar_rather_than_appearing();
    widget_tile::tests::a_tile_that_is_leaving_keeps_its_place_until_it_has_gone();
    widget_tile::tests::a_tile_asked_for_again_while_leaving_turns_round();
    widget_tile::tests::a_widget_that_joined_leaves_the_others_alone();
    widget_tile::tests::a_reorder_moves_the_tiles_rather_than_remaking_them();
    widget_tile::tests::the_divider_only_stands_where_there_is_something_to_divide();
    widget_tile::tests::the_divider_that_comes_back_is_the_one_that_left();
    widget_tile::tests::the_shelf_knows_what_is_its_own();
    widget_tile::tests::a_state_for_a_widget_with_no_tile_is_not_claimed();
    widget_tile::tests::a_tile_is_the_size_the_widget_declared();
    widget_tile::tests::a_tile_follows_the_icon_size_without_being_rebuilt();
    widget_tile::tests::a_tile_coming_or_going_does_not_resize_its_neighbours();
    widget_tile::tests::drawing_a_tile_asks_the_daemon_for_nothing();
    widget_tile::tests::a_widget_this_bar_cannot_read_still_gets_a_tile();
    widget_tile::tests::a_tile_can_be_given_the_keyboard_back();
    panel::tests::reading_a_note_and_closing_writes_nothing();
    panel::tests::the_weeks_bars_stand_for_the_days_they_are_drawn_from();
    panel::tests::a_panel_closes_on_escape_and_holds_the_grab_that_dismisses_it();
    panel::tests::a_panel_opens_before_the_daemon_has_answered();
    panel::tests::a_panel_opened_at_the_shape_it_turns_out_to_have_does_not_move();
    panel::tests::a_panel_larger_than_it_opened_for_is_still_shown_whole();
    panel::tests::a_panel_hangs_above_the_tile_that_asked();
    panel::tests::a_panel_opened_again_is_shown_whole();
    panel::tests::a_panel_opened_before_is_drawn_at_the_size_it_was();
    panel::tests::choosing_a_control_asks_the_daemon_for_it();
    panel::tests::the_keyboard_walks_the_controls_and_skips_the_heading();
    panel::tests::choosing_a_control_leaves_the_panel_up();
    panel::tests::the_release_of_the_click_that_opened_a_panel_chooses_nothing();
    panel::tests::a_panel_that_is_up_takes_the_widgets_news_in_place();
    panel::tests::choosing_a_view_in_the_panel_writes_the_mode();
    an_undefined_colour_is_not_something_gtk_reports();
    a_sheet_that_failed_to_load_leaves_the_provider_able_to_load_another();
    the_system_sheet_loads_against_a_real_gtk_theme();
}

/// Why `system` is guarded by a lookup and not by a load error.
///
/// GTK does *not* refuse a sheet that names a colour the theme never defined:
/// the load returns `Ok` and the declaration resolves to nothing when drawn.
/// On a translucent dock that is an invisible bar — a worse outcome than an
/// ugly one, and a silent one. So `Style::usable` looks the colours up
/// instead of waiting for a failure that never comes.
///
/// If a future GTK starts refusing these, this check fails and says so, and
/// the guard can be simplified to the load error after all.
#[cfg(test)]
fn an_undefined_colour_is_not_something_gtk_reports() {
    let provider = gtk::CssProvider::new();

    assert!(
        provider
            .load_from_data(b"#bar { background: @no_such_colour_anywhere; }")
            .is_ok(),
        "GTK now refuses an undefined named colour; the lookup guard in \
         Style::usable can be replaced by the load error"
    );
}

/// The fallback in `Style::apply` loads `native` through the same provider
/// that just failed, which is only a rescue if the failure leaves the
/// provider usable rather than poisoned.
#[cfg(test)]
fn a_sheet_that_failed_to_load_leaves_the_provider_able_to_load_another() {
    let provider = gtk::CssProvider::new();

    assert!(
        provider.load_from_data(b"#bar { no-such-prop: 3px; }").is_err(),
        "a malformed sheet should fail, or there is nothing for the \
         fallback to catch"
    );
    assert!(
        provider
            .load_from_data(theme::css(theme::DEFAULT).as_bytes())
            .is_ok(),
        "the provider was left unusable by the failure, so falling back \
         through it would leave the bar unstyled"
    );
}

/// The system sheet against whatever theme this session is actually wearing.
#[cfg(test)]
fn the_system_sheet_loads_against_a_real_gtk_theme() {
    let provider = gtk::CssProvider::new();

    if let Err(e) = provider.load_from_data(theme::css(theme::SYSTEM).as_bytes()) {
        panic!("the system sheet does not load under this GTK theme: {e}");
    }

    // And the colours it borrows are really there to borrow, so an ordinary
    // theme does not trip the fallback.
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    let missing = theme::missing_colours(&gtk::prelude::WidgetExt::style_context(&window));
    assert!(
        missing.is_empty(),
        "this GTK theme defines none of {missing:?}, so system would fall back here"
    );
}

/// The auto-hide's own checks, which need a window and the main loop.
#[cfg(test)]
pub mod hide_tests {
    use super::*;

    /// Let glib get on with whatever is due, for about this long.
    fn waiting(span: std::time::Duration) {
        let until = Instant::now() + span;
        while Instant::now() < until {
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn hidden_bar() -> (gtk::Window, Hide) {
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_default_size(400, 60);
        let hide = Hide::default();
        // Switched on, which leaves the bar sliding away, and then let it
        // arrive so the test starts from a bar that is really hidden.
        hide.place(&window, 0, 500, 60, true);
        hide.settle();
        assert!(!hide.shown.get(), "the bar did not go away when asked");
        (window, hide)
    }

    /// With animations off, the bar does not slide: it is where it was sent
    /// on the first frame drawn.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_bar_held_still_is_where_it_was_sent_at_once() {
        let (shown_y, height) = (500, 60);
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_default_size(400, height);
        window.show_all();
        let hide = Hide::default();
        hide.place(&window, 0, shown_y, height, false);

        motion::hold_still(true);
        hide.place(&window, 0, shown_y, height, true);
        let first: Rc<Cell<Option<i32>>> = Rc::default();
        let seen = first.clone();
        let watching = hide.clone();
        window.add_tick_callback(move |_, _| {
            if seen.get().is_none() {
                seen.set(Some(watching.y()));
            }
            glib::ControlFlow::Continue
        });
        waiting(motion::FRAME * 3);
        motion::hold_still(false);

        assert_eq!(
            first.get(),
            Some(motion::hidden_y(shown_y, height)),
            "with animations off the bar still slid"
        );
        window.close();
        waiting(std::time::Duration::from_millis(20));
    }

    /// The widest step one frame of a slide can take, in pixels, for a frame
    /// of this length — measured off the curve itself, so the bound is the
    /// motion's own and not a number chosen to pass.
    fn widest_step(frame: std::time::Duration, shown_y: i32, hidden_y: i32) -> i32 {
        let mut widest = 0;
        for showing in [true, false] {
            let mut at = std::time::Duration::ZERO;
            while at < motion::SLIDE {
                let here = motion::slide_y(motion::slide_progress(at, showing), shown_y, hidden_y);
                let next = motion::slide_y(motion::slide_progress(at + frame, showing), shown_y, hidden_y);
                widest = widest.max((next - here).abs());
                at += std::time::Duration::from_millis(1);
            }
        }
        widest
    }

    /// #18, P0 item 1, the box left open: a bar turned round part-way down
    /// never jumps further than a frame of its own travel.
    ///
    /// The motion functions have a test of their own for this; this one
    /// watches the *bar* — the `Hide` the dock really runs, on the window's
    /// frame clock — and measures every frame it draws, through the turn.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_bar_turned_round_mid_slide_never_jumps() {
        let (shown_y, height) = (500, 60);
        let hidden_y = motion::hidden_y(shown_y, height);

        for share in [10u32, 50, 90] {
            let window = gtk::Window::new(gtk::WindowType::Toplevel);
            window.set_default_size(400, height);
            window.show_all();
            let hide = Hide::default();
            // On, and on screen: switching it on starts the slide away.
            hide.place(&window, 0, shown_y, height, false);
            hide.place(&window, 0, shown_y, height, true);
            assert!(hide.ticking.get(), "switching auto-hide on did not start a slide");

            // Every frame the clock draws: when, and where the bar was put.
            let frames: Rc<RefCell<Vec<(i64, i32)>>> = Rc::default();
            let recording = frames.clone();
            let watching = hide.clone();
            window.add_tick_callback(move |_, clock| {
                recording.borrow_mut().push((clock.frame_time(), watching.y()));
                glib::ControlFlow::Continue
            });

            waiting(motion::SLIDE * share / 100);
            hide.slide(&window, true);
            waiting(motion::SLIDE * 2);

            let frames = frames.borrow();
            assert!(frames.len() > 4, "the frame clock barely ran: {} frames", frames.len());
            assert_eq!(frames.last().map(|(_, y)| *y), Some(shown_y), "turned round at {share}% and never came back");
            for pair in frames.windows(2) {
                let ((t0, y0), (t1, y1)) = (pair[0], pair[1]);
                let frame = std::time::Duration::from_micros((t1 - t0).max(0) as u64);
                let allowed = widest_step(frame.max(motion::FRAME), shown_y, hidden_y) + 1;
                assert!(
                    (y1 - y0).abs() <= allowed,
                    "turned round at {share}%: the bar jumped {} px in one frame of {frame:?}, where the slide moves at most {allowed}",
                    (y1 - y0).abs()
                );
            }
            window.close();
            waiting(std::time::Duration::from_millis(20));
        }
    }

    /// The whole of the item: a pointer that crosses the edge and keeps going
    /// has asked for the bar and asked for it to go, and must get neither.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_pointer_only_passing_through_does_not_summon_the_bar() {
        let (window, hide) = hidden_bar();

        hide.intend(&window, true);
        // The loop runs here, which is the point: asked for and cancelled in
        // the same breath would pass with no wait at all. Half of the wait is
        // long enough for a reveal that was never going to wait to land.
        waiting(motion::REVEAL_AFTER / 2);
        hide.intend(&window, false);
        waiting(motion::REVEAL_AFTER * 2);

        assert!(
            !hide.shown.get(),
            "the bar came out for a pointer that had already left"
        );
        window.close();
    }

    /// And the other way, which is the one that would make a dock useless:
    /// held, the arrival does count.
    pub fn a_pointer_that_stays_is_an_arrival() {
        let (window, hide) = hidden_bar();

        hide.intend(&window, true);
        waiting(motion::REVEAL_AFTER * 3);

        assert!(hide.shown.get(), "the bar never came out for a pointer that waited");
        window.close();
    }

    /// Leaving is held for longer than arriving, so a hand that overshoots on
    /// the way to an icon and comes straight back keeps the bar.
    pub fn a_hand_that_overshoots_and_comes_back_keeps_the_bar() {
        let (window, hide) = hidden_bar();
        hide.intend(&window, true);
        waiting(motion::REVEAL_AFTER * 3);
        assert!(hide.shown.get());

        hide.intend(&window, false);
        waiting(motion::REVEAL_AFTER);
        assert!(
            hide.shown.get(),
            "the bar left on the first frame the pointer was outside it"
        );

        hide.intend(&window, true);
        waiting(motion::HIDE_AFTER);

        assert!(
            hide.shown.get(),
            "a moment outside the bar was enough to lose it"
        );
        window.close();
    }

    /// A menu that belongs to the dock keeps the dock, however long the
    /// pointer has been off it.
    ///
    /// Right-clicking an icon moves the pointer onto the menu, which is off
    /// the bar: without this the bar slides away under the menu the click
    /// just opened, and closing the menu drops the pointer onto a desktop
    /// nobody aimed at.
    pub fn an_open_menu_keeps_the_bar_where_it_is() {
        let (window, hide) = hidden_bar();
        hide.intend(&window, true);
        waiting(motion::REVEAL_AFTER * 3);
        assert!(hide.shown.get());

        let menu = gtk::Menu::new();
        menu.add(&gtk::MenuItem::with_label("something"));
        menu.show_all();
        hide.holds_for(&window, &menu);
        // Against the window, not the pointer: `popup_at_pointer` reads the
        // pointer out of an event and a test has none to give it, so the menu
        // never maps and nothing is ever held.
        menu.popup_at_widget(
            &window,
            gdk::Gravity::NorthWest,
            gdk::Gravity::SouthWest,
            None,
        );
        waiting(std::time::Duration::from_millis(80));
        assert!(menu.is_visible(), "the menu under test never opened");

        hide.intend(&window, false);
        waiting(motion::HIDE_AFTER * 2);

        assert!(
            hide.shown.get(),
            "the bar hid itself under a menu that was still open"
        );

        menu.popdown();
        waiting(motion::HIDE_AFTER * 2);
        assert!(
            !hide.shown.get(),
            "the bar never went once the menu that was holding it closed"
        );
        window.close();
    }

    /// Two at once — a context menu over a folder grid — and the first to
    /// close must not speak for the second.
    pub fn the_first_menu_to_close_does_not_release_the_second() {
        let (window, hide) = hidden_bar();
        hide.intend(&window, true);
        waiting(motion::REVEAL_AFTER * 3);

        let (first, second) = (gtk::Menu::new(), gtk::Menu::new());
        for menu in [&first, &second] {
            menu.add(&gtk::MenuItem::with_label("something"));
            menu.show_all();
            hide.holds_for(&window, menu);
            menu.popup_at_widget(
                &window,
                gdk::Gravity::NorthWest,
                gdk::Gravity::SouthWest,
                None,
            );
        }
        waiting(std::time::Duration::from_millis(80));
        hide.intend(&window, false);

        first.popdown();
        waiting(motion::HIDE_AFTER * 2);

        assert!(
            hide.shown.get(),
            "one menu closing let the bar go while another was still open"
        );
        second.popdown();
        waiting(motion::HIDE_AFTER * 2);
        assert!(!hide.shown.get(), "the bar stayed after everything closed");
        window.close();
    }

    /// Turning auto-hide off while the pointer's departure is still being
    /// waited on must not leave that departure to land later.
    pub fn switching_auto_hide_off_drops_a_move_nobody_wants_any_more() {
        let (window, hide) = hidden_bar();
        hide.intend(&window, true);
        waiting(motion::REVEAL_AFTER * 3);
        hide.intend(&window, false);

        hide.place(&window, 0, 500, 60, false);
        waiting(motion::HIDE_AFTER * 2);

        assert!(
            hide.shown.get(),
            "the bar hid itself after auto-hide was switched off"
        );
        window.close();
    }
}
