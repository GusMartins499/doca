mod strut;

use anyhow::Result;
use gtk::prelude::*;
use primodock_ipc::PrimoDockProxy;
use tracing_subscriber::EnvFilter;

use crate::strut::BottomStrut;

const BAR_HEIGHT: i32 = 64;
const BAR_WIDTH_RATIO: f64 = 0.6;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("primodock_shell=info")),
        )
        .init();

    gtk::init()?;

    let monitor = gdk::Display::default()
        .and_then(|d| d.primary_monitor())
        .ok_or_else(|| anyhow::anyhow!("no primary monitor"))?;
    let screen = monitor.geometry();

    let bar_width = (screen.width() as f64 * BAR_WIDTH_RATIO) as i32;
    let bar_x = screen.x() + (screen.width() - bar_width) / 2;
    let bar_y = screen.y() + screen.height() - BAR_HEIGHT;

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
    window.set_size_request(bar_width, BAR_HEIGHT);

    if let Some(visual) = gtk::prelude::WidgetExt::screen(&window).and_then(|s| s.rgba_visual()) {
        window.set_visual(Some(&visual));
    }

    let label = gtk::Label::new(Some("connecting to primodockd…"));
    label.set_widget_name("status");
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    root.set_widget_name("bar");
    root.set_halign(gtk::Align::Center);
    root.set_valign(gtk::Align::Center);
    root.add(&label);
    window.add(&root);

    let css = gtk::CssProvider::new();
    css.load_from_data(
        b"#bar { background: transparent; }
          #status { color: #e6e6e6; font-size: 13px; font-weight: 500; }
          window { background: rgba(28,28,30,0.78); border-radius: 18px; }",
    )?;
    gtk::StyleContext::add_provider_for_screen(
        &gtk::prelude::WidgetExt::screen(&window).unwrap(),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    window.connect_realize(move |w| {
        let Some(gdk_window) = w.window() else { return };
        let Ok(x11_window) = gdk_window.downcast::<gdkx11::X11Window>() else {
            tracing::error!("not an X11 session; the shell layer needs replacing for Wayland");
            return;
        };
        let xid = x11_window.xid() as u32;
        let reserved = BottomStrut {
            height: BAR_HEIGHT as u32,
            start_x: bar_x as u32,
            end_x: (bar_x + bar_width) as u32,
        };
        match strut::apply(xid, &reserved) {
            Ok(()) => tracing::info!(xid, "strut applied"),
            Err(e) => tracing::error!("strut failed: {e:#}"),
        }
    });

    window.show_all();
    window.move_(bar_x, bar_y);

    glib::spawn_future_local(async move {
        if let Err(e) = drive(label).await {
            tracing::error!("daemon link failed: {e:#}");
        }
    });

    gtk::main();
    Ok(())
}

async fn drive(label: gtk::Label) -> Result<()> {
    let connection = zbus::Connection::session().await?;
    let proxy = PrimoDockProxy::new(&connection).await?;
    tracing::info!("connected to primodockd");

    refresh(&proxy, &label).await;

    let mut windows = proxy.receive_windows_changed().await?;
    let mut workspace = proxy.receive_workspace_changed().await?;

    loop {
        futures_util::select! {
            _ = futures_util::StreamExt::next(&mut windows) => refresh(&proxy, &label).await,
            _ = futures_util::StreamExt::next(&mut workspace) => refresh(&proxy, &label).await,
            complete => break,
        }
    }
    Ok(())
}

async fn refresh(proxy: &PrimoDockProxy<'_>, label: &gtk::Label) {
    let windows = proxy.list_windows().await.unwrap_or_default();
    let current = proxy.current_workspace().await.unwrap_or(0);
    let total = proxy.workspace_count().await.unwrap_or(1);
    let here = windows.iter().filter(|w| w.workspace == current).count();
    let focused = windows
        .iter()
        .find(|w| w.active)
        .map(|w| w.app_id.as_str())
        .unwrap_or("—");

    label.set_text(&format!(
        "{} windows · {} here · workspace {}/{} · focus: {}",
        windows.len(),
        here,
        current + 1,
        total,
        focused
    ));
}
