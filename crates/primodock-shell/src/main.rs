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
use std::collections::HashMap;
use std::rc::Rc;

use anyhow::Result;
use gtk::prelude::*;
use primodock_ipc::PrimoDockProxy;
use tracing_subscriber::EnvFilter;

use crate::strut::BottomStrut;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("primodock_shell=info")),
        )
        .init();

    gtk::init()?;

    let monitor = gdk::Display::default()
        .and_then(|display| display.primary_monitor())
        .ok_or_else(|| anyhow::anyhow!("no primary monitor"))?;
    let screen = monitor.geometry();

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("PrimoDock");
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

type Tiles = Rc<RefCell<HashMap<String, widget_tile::WidgetTile>>>;

#[derive(Clone, Default)]
struct Hide {
    enabled: Rc<Cell<bool>>,
    shown: Rc<Cell<bool>>,
    shown_y: Rc<Cell<i32>>,
    height: Rc<Cell<i32>>,
    x: Rc<Cell<i32>>,
    animating: Rc<Cell<bool>>,
}

impl Hide {
    fn place(&self, window: &gtk::Window, x: i32, shown_y: i32, height: i32, enabled: bool) {
        self.x.set(x);
        self.shown_y.set(shown_y);
        self.height.set(height);
        let was_enabled = self.enabled.replace(enabled);
        if !enabled {
            self.shown.set(true);
            window.move_(x, shown_y);
            return;
        }
        if !was_enabled {
            self.shown.set(false);
        }
        let y = if self.shown.get() {
            shown_y
        } else {
            motion::hidden_y(shown_y, height)
        };
        window.move_(x, y);
    }

    fn slide(&self, window: &gtk::Window, to_shown: bool) {
        if !self.enabled.get() || self.shown.get() == to_shown || self.animating.get() {
            return;
        }
        self.shown.set(to_shown);
        self.animating.set(true);

        let hide = self.clone();
        let window = window.clone();
        let start = Instant::now();
        glib::timeout_add_local(motion::FRAME, move || {
            let elapsed = start.elapsed();
            let raw = elapsed.as_secs_f64() / motion::SLIDE.as_secs_f64();
            let progress = if to_shown { raw } else { 1.0 - raw };

            let shown_y = hide.shown_y.get();
            let hidden = motion::hidden_y(shown_y, hide.height.get());
            window.move_(hide.x.get(), motion::slide_y(progress, shown_y, hidden));

            if raw >= 1.0 {
                hide.animating.set(false);
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
    let proxy: Rc<PrimoDockProxy<'static>> = Rc::new(PrimoDockProxy::new(&connection).await?);
    let tiles: Tiles = Rc::new(RefCell::new(HashMap::new()));
    let applied: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let hide = Hide::default();
    let label = tooltip::Tooltip::new();
    let icons = row::Row::new();
    icons.stage_on(&stage);
    window.show_all();
    tracing::info!("connected to primodockd");

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
    wire(&icons, &label, proxy.clone());

    let chrome = Chrome {
        style: Style { css, applied },
        row: icons.clone(),
        hide,
        label,
    };
    rebuild(&window, &bar, &items, &screen, proxy.clone(), tiles.clone(), &chrome).await;

    let mut items_changed = proxy.receive_items_changed().await?;
    let mut environment_changed = proxy.receive_environment_changed().await?;
    let mut widget_changed = proxy.receive_widget_changed().await?;

    loop {
        futures_util::select! {
            _ = futures_util::StreamExt::next(&mut items_changed) => {}
            _ = futures_util::StreamExt::next(&mut environment_changed) => {}
            signal = futures_util::StreamExt::next(&mut widget_changed) => {
                if let Some(signal) = signal {
                    if let Ok(args) = signal.args() {
                        if let Some(tile) = tiles.borrow().get(&args.state.id) {
                            tile.update(&args.state);
                            continue;
                        }
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

fn wire(icons: &row::Row, label: &tooltip::Tooltip, proxy: Rc<PrimoDockProxy<'static>>) {
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
                glib::spawn_future_local(async move {
                    let entries = proxy.list_folder(&item.id).await.unwrap_or_default();
                    stack::menu(&entries, proxy.clone()).popup_at_pointer(Some(&trigger));
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

/// The pieces of the bar that outlive any one rebuild.
struct Chrome {
    style: Style,
    row: row::Row,
    hide: Hide,
    label: tooltip::Tooltip,
}

impl Style {
    fn apply(&self, requested: &str) {
        let resolved = theme::resolve(requested);
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
            Err(e) => tracing::error!("theme {resolved} failed to load: {e}"),
        }
    }
}

async fn rebuild(
    window: &gtk::Window,
    bar: &gtk::Box,
    items: &gtk::Box,
    screen: &gdk::Rectangle,
    proxy: Rc<PrimoDockProxy<'static>>,
    tiles: Tiles,
    chrome: &Chrome,
) {
    let Chrome {
        style,
        row,
        hide,
        label,
    } = chrome;
    let appearance = proxy.appearance().await.ok();
    if let Some(appearance) = &appearance {
        style.apply(&appearance.theme);
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

    let appearance_theme = appearance
        .as_ref()
        .map(|appearance| appearance.theme.clone())
        .unwrap_or_else(|| theme::DEFAULT.to_string());

    let entries = proxy.list_items().await.unwrap_or_default();
    let widgets = proxy.list_widgets().await.unwrap_or_default();

    // Everything but the row is torn down and built again. The row stays put:
    // unparenting a widget destroys its window, and the pointer leaving a
    // window that was taken out from under it is a leave event like any other
    // — which shut the lens every time a window somewhere took focus.
    for child in items.children() {
        if child != row.perch.clone().upcast::<gtk::Widget>() {
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
        let (running, active) = theme::dots(&appearance_theme);
        row.dot_colours(running, active);
    }

    // The dock may have changed under a pointer that never moved.
    name_what_is_hovered(row, label);

    tiles.borrow_mut().clear();
    if !widgets.is_empty() {
        items.add(&dock::divider());

        for state in &widgets {
            let tile = widget_tile::WidgetTile::new(state, proxy.clone());
            items.add(&tile.root);
            tiles.borrow_mut().insert(state.id.clone(), tile);
        }
    }

    items.show_all();
    for state in &widgets {
        if let Some(tile) = tiles.borrow().get(&state.id) {
            tile.update(state);
        }
    }

    // The bar is as wide as its contents ask; the window is that plus the room
    // a magnified icon rises into, which the bar itself never occupies.
    let natural = dock::natural_size(bar);
    let (width, bar_height) = dock::clamp_to_screen(natural, (screen.width(), screen.height()));
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
    window.set_size_request(width, height);
    window.resize(width, height);

    let x = screen.x() + (screen.width() - width) / 2;
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
    // icon passes through, and reserving that would push every window down by
    // the height of a magnification nobody is looking at.
    let reserved = BottomStrut {
        height: strut::reserved_height(bar_height, auto_hide),
        start_x: x.max(0) as u32,
        end_x: (x + width).max(0) as u32,
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
}
