mod dock;
mod strut;

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

async fn drive(window: gtk::Window, items: gtk::Box, screen: gdk::Rectangle) -> Result<()> {
    let connection = zbus::Connection::session().await?;
    let proxy: Rc<PrimoDockProxy<'static>> = Rc::new(PrimoDockProxy::new(&connection).await?);
    tracing::info!("connected to primodockd");

    rebuild(&window, &items, &screen, proxy.clone()).await;

    let mut changes = proxy.receive_items_changed().await?;
    while futures_util::StreamExt::next(&mut changes).await.is_some() {
        rebuild(&window, &items, &screen, proxy.clone()).await;
    }
    Ok(())
}

async fn rebuild(
    window: &gtk::Window,
    items: &gtk::Box,
    screen: &gdk::Rectangle,
    proxy: Rc<PrimoDockProxy<'static>>,
) {
    let entries = proxy.list_items().await.unwrap_or_default();

    for child in items.children() {
        items.remove(&child);
    }

    if entries.is_empty() {
        let empty = gtk::Label::new(Some("nothing running, nothing pinned"));
        empty.set_widget_name("empty");
        items.add(&empty);
    } else {
        for entry in &entries {
            items.add(&dock::item_button(entry, proxy.clone()));
        }
    }
    items.show_all();

    let width = dock::bar_width(entries.len() as i32);
    let height = dock::bar_height();
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
