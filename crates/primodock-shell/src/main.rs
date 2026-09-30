mod dock;
mod magnify;
mod stack;
mod strut;
mod theme;
mod widget_tile;

use std::cell::{Cell, RefCell};
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

    let items = gtk::Box::new(gtk::Orientation::Horizontal, dock::ITEM_SPACING);
    items.set_widget_name("bar");
    items.set_halign(gtk::Align::Center);
    window.add(&items);
    window.show_all();

    glib::spawn_future_local(async move {
        if let Err(e) = drive(window, items, screen, css).await {
            tracing::error!("daemon link failed: {e:#}");
        }
    });

    gtk::main();
    Ok(())
}

type Tiles = Rc<RefCell<HashMap<String, widget_tile::WidgetTile>>>;

#[derive(Clone, Default)]
struct Lens {
    images: Rc<RefCell<Vec<gtk::Image>>>,
    roots: Rc<RefCell<Vec<gtk::Widget>>>,
    base_icon: Rc<Cell<f64>>,
    base_slot: Rc<Cell<f64>>,
    offset: Rc<Cell<f64>>,
    scale: Rc<Cell<f64>>,
}

impl Lens {
    fn remember(&self, base_icon: i32, scale: f64) {
        self.base_icon.set(base_icon as f64);
        self.base_slot
            .set((base_icon + dock::ITEM_PADDING * 2) as f64);
        self.scale.set(scale);
    }

    fn capture_offset(&self) {
        let offset = self
            .roots
            .borrow()
            .first()
            .map(|root| root.allocation().x() as f64)
            .unwrap_or(0.0);
        if offset > 0.0 {
            self.offset.set(offset);
        }
    }

    fn watch(&self) {
        for root in self.roots.borrow().iter() {
            let lens = self.clone();
            root.connect_motion_notify_event(move |widget, event| {
                lens.capture_offset();
                lens.focus(Some(widget.allocation().x() as f64 + event.position().0));
                glib::Propagation::Proceed
            });
        }
    }

    fn focus(&self, pointer_x: Option<f64>) {
        if self.scale.get() <= 1.0 {
            return;
        }
        let images = self.images.borrow();
        let sizes = magnify::sizes_under_pointer(
            images.len(),
            pointer_x,
            self.base_icon.get(),
            self.base_slot.get(),
            dock::ITEM_SPACING as f64,
            self.offset.get(),
            self.scale.get(),
        );
        for (image, size) in images.iter().zip(sizes) {
            if image.pixel_size() != size {
                image.set_pixel_size(size);
            }
        }
    }
}

async fn drive(
    window: gtk::Window,
    items: gtk::Box,
    screen: gdk::Rectangle,
    css: gtk::CssProvider,
) -> Result<()> {
    let connection = zbus::Connection::session().await?;
    let proxy: Rc<PrimoDockProxy<'static>> = Rc::new(PrimoDockProxy::new(&connection).await?);
    let tiles: Tiles = Rc::new(RefCell::new(HashMap::new()));
    let applied: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let lens = Lens::default();
    tracing::info!("connected to primodockd");

    window.add_events(gdk::EventMask::POINTER_MOTION_MASK | gdk::EventMask::LEAVE_NOTIFY_MASK);
    let leaving = lens.clone();
    window.connect_leave_notify_event(move |_, _| {
        leaving.focus(None);
        glib::Propagation::Proceed
    });

    let style = Style { css, applied };
    rebuild(&window, &items, &screen, proxy.clone(), tiles.clone(), &style, &lens).await;

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
        rebuild(&window, &items, &screen, proxy.clone(), tiles.clone(), &style, &lens).await;
    }
    Ok(())
}

struct Style {
    css: gtk::CssProvider,
    applied: Rc<RefCell<String>>,
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
    items: &gtk::Box,
    screen: &gdk::Rectangle,
    proxy: Rc<PrimoDockProxy<'static>>,
    tiles: Tiles,
    style: &Style,
    lens: &Lens,
) {
    let appearance = proxy.appearance().await.ok();
    if let Some(appearance) = &appearance {
        style.apply(&appearance.theme);
    }
    let preferred_icon = appearance
        .as_ref()
        .map(|appearance| appearance.icon_size)
        .unwrap_or(dock::ICON_SIZE);
    let magnification = appearance
        .as_ref()
        .map(|appearance| appearance.magnification)
        .unwrap_or(magnify::DEFAULT_SCALE);

    let entries = proxy.list_items().await.unwrap_or_default();
    let widgets = proxy.list_widgets().await.unwrap_or_default();
    for child in items.children() {
        items.remove(&child);
    }

    if entries.is_empty() {
        let empty = gtk::Label::new(Some("nothing running, nothing pinned"));
        empty.set_widget_name("empty");
        items.add(&empty);
        lens.images.borrow_mut().clear();
        lens.roots.borrow_mut().clear();
    } else {
        let icon_size = dock::icon_size_for(
            entries.len() as i32,
            widgets.len() as i32,
            screen.width(),
            preferred_icon,
        );
        lens.images.borrow_mut().clear();
        lens.roots.borrow_mut().clear();
        for entry in &entries {
            let built = dock::item_button(entry, icon_size, proxy.clone());
            items.add(&built.root);
            lens.images.borrow_mut().push(built.image);
            lens.roots.borrow_mut().push(built.root);
        }
        lens.remember(icon_size, magnification);
        lens.watch();
    }

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
    let settled = lens.clone();
    glib::idle_add_local_once(move || settled.capture_offset());
    for state in &widgets {
        if let Some(tile) = tiles.borrow().get(&state.id) {
            tile.update(state);
        }
    }

    let natural = dock::natural_size(items);
    let (width, height) = dock::clamp_to_screen(natural, (screen.width(), screen.height()));
    if (width, height) != natural {
        tracing::warn!(
            natural_width = natural.0,
            natural_height = natural.1,
            width,
            height,
            "bar clamped to the screen"
        );
    }
    window.set_size_request(width, height);
    window.resize(width, height);

    let x = screen.x() + (screen.width() - width) / 2;
    let y = screen.y() + screen.height() - height;
    window.move_(x, y);

    let Some(gdk_window) = window.window() else {
        tracing::warn!("window is not realised yet, strut skipped");
        return;
    };
    let Ok(x11_window) = gdk_window.downcast::<gdkx11::X11Window>() else {
        tracing::warn!("not an X11 window, strut skipped");
        return;
    };
    let reserved = BottomStrut {
        height: height as u32,
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
