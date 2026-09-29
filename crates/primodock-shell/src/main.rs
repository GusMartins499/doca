mod dock;
mod strut;
mod widget_tile;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use anyhow::Result;
use gtk::prelude::*;
use primodock_ipc::PrimoDockProxy;
use tracing_subscriber::EnvFilter;

use crate::strut::BottomStrut;

const STYLE: &str = "
    window { background: transparent; }
    #bar {
        background: rgba(28,28,30,0.82);
        border-radius: 18px;
        border: 1px solid rgba(255,255,255,0.08);
        padding: 10px;
    }
    #item { border-radius: 12px; padding: 8px; }
    #item:hover { background: rgba(255,255,255,0.10); }
    #indicator-idle { background: transparent; }
    #indicator { background: rgba(255,255,255,0.45); border-radius: 2px; }
    #indicator-active { background: #4c8dff; border-radius: 2px; }
    #empty { color: rgba(255,255,255,0.55); font-size: 13px; padding: 12px; }
    #environment {
        background: rgba(255,255,255,0.08);
        border-radius: 12px;
        padding: 0 10px;
    }
    #environment:hover { background: rgba(255,255,255,0.16); }
    #environment-name {
        color: #e6e6e6;
        font-size: 12px;
        font-weight: 600;
    }
    #separator { background: rgba(255,255,255,0.12); }
    #widget { border-radius: 12px; padding: 6px 8px; }
    #widget:hover { background: rgba(255,255,255,0.10); }
    #widget.active { background: rgba(76,141,255,0.18); }
    #widget-label { color: #f2f2f2; font-size: 15px; font-weight: 600; }
    #widget-detail { color: rgba(255,255,255,0.55); font-size: 10px; }
    #widget-progress {
        min-height: 3px;
        background: rgba(255,255,255,0.14);
    }
    #widget-progress progress { background: #4c8dff; min-height: 3px; }
";

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
    css.load_from_data(STYLE.as_bytes())?;
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
        if let Err(e) = drive(window, items, screen).await {
            tracing::error!("daemon link failed: {e:#}");
        }
    });

    gtk::main();
    Ok(())
}

type Tiles = Rc<RefCell<HashMap<String, widget_tile::WidgetTile>>>;

async fn drive(window: gtk::Window, items: gtk::Box, screen: gdk::Rectangle) -> Result<()> {
    let connection = zbus::Connection::session().await?;
    let proxy: Rc<PrimoDockProxy<'static>> = Rc::new(PrimoDockProxy::new(&connection).await?);
    let tiles: Tiles = Rc::new(RefCell::new(HashMap::new()));
    tracing::info!("connected to primodockd");

    rebuild(&window, &items, &screen, proxy.clone(), tiles.clone()).await;

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
        rebuild(&window, &items, &screen, proxy.clone(), tiles.clone()).await;
    }
    Ok(())
}

async fn rebuild(
    window: &gtk::Window,
    items: &gtk::Box,
    screen: &gdk::Rectangle,
    proxy: Rc<PrimoDockProxy<'static>>,
    tiles: Tiles,
) {
    let entries = proxy.list_items().await.unwrap_or_default();
    let widgets = proxy.list_widgets().await.unwrap_or_default();
    let environments = proxy.list_environments().await.unwrap_or_default();

    for child in items.children() {
        items.remove(&child);
    }

    if dock::switcher_is_useful(environments.len()) {
        let current = environments
            .iter()
            .find(|environment| environment.current)
            .map(|environment| environment.name.clone())
            .unwrap_or_default();
        items.add(&dock::environment_switcher(&current, proxy.clone()));
        items.add(&dock::divider());
    }

    if entries.is_empty() {
        let empty = gtk::Label::new(Some("nothing running, nothing pinned"));
        empty.set_widget_name("empty");
        items.add(&empty);
    } else {
        let icon_size = dock::icon_size_for(
            entries.len() as i32,
            widgets.len() as i32,
            screen.width() - if dock::switcher_is_useful(environments.len()) { 0 } else { dock::SWITCHER_WIDTH },
        );
        for entry in &entries {
            items.add(&dock::item_button(entry, icon_size, proxy.clone()));
        }
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
