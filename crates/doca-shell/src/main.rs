mod appearance;
mod dock;
mod magnify;
mod motion;
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
        let shown_y = self.shown_y.get();
        let hidden = motion::hidden_y(shown_y, self.height.get());
        let progress = motion::slide_progress(self.since.get().elapsed(), self.shown.get());
        motion::slide_y(progress, shown_y, hidden)
    }

    /// Declare the slide over, wherever it was going.
    fn settle(&self) {
        self.since.set(Instant::now() - motion::SLIDE);
    }

    /// Send the bar up or down, from wherever it happens to be.
    ///
    /// Asking for the way it is already going is nothing; asking for the other
    /// way turns it round at the position it is at, and the time left shrinks
    /// with the distance left. Dropping the request instead — which is what
    /// this did — left the pointer inside a bar that stayed hidden until the
    /// mouse moved again.
    fn slide(&self, window: &gtk::Window, to_shown: bool) {
        if !self.enabled.get() || self.shown.replace(to_shown) == to_shown {
            return;
        }
        let elapsed = self.since.get().elapsed();
        self.since
            .set(Instant::now() - motion::reversed_start(elapsed, motion::SLIDE));
        self.run(window);
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
        hiding.slide(window, false);
        hiding_label.hide();
        glib::Propagation::Proceed
    });
    let showing = hide.clone();
    window.connect_enter_notify_event(move |window, _| {
        showing.slide(window, true);
        glib::Propagation::Proceed
    });

    icons.serve(proxy.clone());
    wire(&icons, &label, proxy.clone(), stack::Remembered::default());

    let chrome = Chrome {
        style: Style { css, applied },
        row: icons.clone(),
        hide,
        label,
        themes: appearance::Themes::capture(),
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
    rebuild(&window, &bar, &items, &screen, proxy.clone(), tiles.clone(), &chrome).await;

    let mut items_changed = proxy.receive_items_changed().await?;
    let mut environment_changed = proxy.receive_environment_changed().await?;
    let mut widget_changed = proxy.receive_widget_changed().await?;
    let mut config_changed = proxy.receive_config_changed().await?;

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
            signal = futures_util::StreamExt::next(&mut widget_changed) => {
                // A widget's new value touches its own tile and nothing
                // else — never the row of icons, which is the tightest thing
                // this loop has to do. A widget the bar is not showing has no
                // tile and nothing to act on here: what puts it on the bar is
                // the *list* changing, which arrives as another signal.
                if let Some(signal) = signal {
                    if let Ok(args) = signal.args() {
                        tiles.update(&args.state);
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
                glib::spawn_future_local(async move {
                    let windows = proxy.item_windows(&item.id).await.unwrap_or_default();
                    dock::context_menu(&item, &windows, proxy.clone())
                        .popup_at_pointer(Some(&trigger));
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
    let magnification = appearance
        .as_ref()
        .map(|appearance| appearance.magnification)
        .unwrap_or(magnify::DEFAULT_SCALE);

    let entries = proxy.list_items().await.unwrap_or_default();
    let widgets = proxy.list_widgets().await.unwrap_or_default();

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

    if entries.is_empty() {
        row.fill(&[], row::Rest::default());
        let empty = gtk::Label::new(Some("nothing running, nothing pinned"));
        empty.set_widget_name("empty");
        items.add(&empty);
    } else {
        let icon_size = dock::icon_size_for(
            entries.len() as i32,
            widgets.len() as i32,
            screen.width(),
            preferred_icon,
            magnification,
        );
        row.fill(
            &entries,
            row::Rest::new(
                icon_size,
                dock::ITEM_SPACING,
                dock::ITEM_PADDING,
                magnification,
            ),
        );
        // The row draws its own dots, so for `system` they are looked up as
        // values rather than read from the sheet.
        let (running, active) =
            theme::dots_for(&style.applied.borrow(), &row.area.style_context());
        row.dot_colours(running, active);
    }

    // The dock may have changed under a pointer that never moved.
    name_what_is_hovered(row, label);

    let asking = proxy.clone();
    let invoke: widget_tile::Invoke = Rc::new(move |id: &str, action: &str| {
        let (id, action) = (id.to_string(), action.to_string());
        let proxy = asking.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = proxy.invoke_widget(&id, &action).await {
                tracing::warn!("widget {id} rejected {action}: {e}");
            }
        });
    });
    tiles.show(items, &widgets, &invoke);

    // The row is drawn on a surface wider than the place it holds, and the
    // surplus hangs off both ends. Off the left end and, with no tiles, off
    // the right, it hangs into the window, where there is nothing to cover.
    // The tiles are packed after the row, so when there are any the room on
    // that side has to be held inside the bar instead — otherwise the
    // drawing lies over the first tile and swallows the clicks meant for it.
    row.perch.set_margin_end(if widgets.is_empty() {
        0
    } else {
        row.margin()
    });

    items.show_all();
    // After `show_all`, which shows every child — including a progress bar a
    // tile had hidden for having no progress to report.
    tiles.refresh(&widgets);

    // The bar is as wide as its contents ask; the window is that plus the room
    // a magnified icon rises into and spreads into, which the bar never
    // occupies. Keeping that room out of the bar is what makes the background
    // hug the icons instead of reaching to both edges of the screen.
    let margin = row.margin();
    let natural = dock::natural_size(bar);
    let (width, bar_height) = dock::clamp_to_screen(
        natural,
        (screen.width() - margin * 2, screen.height()),
    );
    let height = bar_height + row.overhead();
    if (width, bar_height) != natural {
        tracing::warn!(
            natural_width = natural.0,
            natural_height = natural.1,
            width,
            bar_height,
            "bar clamped to the screen"
        );
    }
    bar.set_size_request(width, -1);
    let window_width = width + margin * 2;
    window.set_size_request(window_width, height);
    window.resize(window_width, height);

    let x = screen.x() + (screen.width() - window_width) / 2;
    let y = screen.y() + screen.height() - height;
    hide.place(window, x, y, height, auto_hide);

    let Some(gdk_window) = window.window() else {
        tracing::warn!("window is not realised yet, strut skipped");
        return;
    };
    let Ok(x11_window) = gdk_window.downcast::<gdkx11::X11Window>() else {
        tracing::warn!("not an X11 window, strut skipped");
        return;
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

    tracing::debug!(items = entries.len(), width, "rebuilt");
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

    dock::tests::measuring_a_bar_twice_gives_the_same_answer_both_times();
    tooltip::tests::a_label_is_the_size_of_its_own_words();
    row::tests::a_pointer_that_left_is_not_pointing_at_anything();
    row::tests::a_pointer_that_comes_back_elsewhere_travels_rather_than_teleports();
    row::tests::a_lens_that_is_shut_aims_at_once_rather_than_travelling();
    row::tests::an_app_that_closed_keeps_its_place_until_it_has_finished_going();
    row::tests::an_app_that_opened_grows_into_the_room_rather_than_appearing_in_it();
    row::tests::a_row_that_was_resized_or_reloaded_is_not_a_row_six_apps_just_opened_on();
    row::tests::an_app_that_closed_and_opened_again_comes_back_from_where_it_had_got_to();
    stack::tests::a_grid_opens_before_the_folder_has_been_read();
    stack::tests::a_folder_opened_at_the_shape_it_turns_out_to_have_does_not_move();
    stack::tests::a_folder_larger_than_the_grid_it_opened_is_still_shown_whole();
    stack::tests::a_folder_with_nothing_in_it_says_so_rather_than_waiting_for_ever();
    stack::tests::a_folder_opened_before_is_drawn_at_the_size_it_was();
    stack::tests::a_grid_always_has_a_cell_to_show_however_little_is_expected();
    widget_tile::tests::showing_the_same_widgets_again_keeps_the_very_same_tiles();
    widget_tile::tests::a_widget_that_went_takes_its_tile_off_the_bar();
    widget_tile::tests::a_widget_that_joined_leaves_the_others_alone();
    widget_tile::tests::a_reorder_moves_the_tiles_rather_than_remaking_them();
    widget_tile::tests::the_divider_only_stands_where_there_is_something_to_divide();
    widget_tile::tests::the_divider_that_comes_back_is_the_one_that_left();
    widget_tile::tests::the_shelf_knows_what_is_its_own();
    widget_tile::tests::a_state_for_a_widget_with_no_tile_is_not_claimed();
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
