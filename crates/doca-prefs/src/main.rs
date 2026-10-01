//! The preferences window: changing the dock without a terminal.
//!
//! A binary of its own rather than a window inside `doca-shell`, for three
//! reasons. The shell is the process that draws the bar and reserves the
//! screen's edge, and a fault in a settings window must not take that down.
//! The window only exists while it is open, and leaves memory when it closes.
//! And the daemon stays the single source of truth for the config, which is
//! what makes this window just another caller on the bus — no different from
//! a keybinding.

mod appearance;
mod link;

use gtk::prelude::*;
use tracing_subscriber::EnvFilter;

use crate::link::Link;

const APP_ID: &str = "io.github.gusmartins499.Doca.Prefs";

const STYLE: &str = "
    #hint { font-size: 11px; }
    #trouble { padding: 24px; }
";

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("doca_prefs=info")),
        )
        .init();

    // An application id is what makes this single-instance: a second launch
    // reaches the first over the bus and its activate puts the window it
    // already has back in front, instead of opening a second one that would
    // write over the first.
    let app = gtk::Application::new(Some(APP_ID), Default::default());
    app.connect_activate(build);
    app.run()
}

fn build(app: &gtk::Application) {
    if let Some(open) = app.windows().first() {
        open.present();
        return;
    }

    let window = gtk::ApplicationWindow::new(app);
    window.set_title("Doca Preferences");
    window.set_default_size(560, 420);

    if let Some(screen) = gtk::prelude::WidgetExt::screen(&window) {
        let css = gtk::CssProvider::new();
        if css.load_from_data(STYLE.as_bytes()).is_ok() {
            gtk::StyleContext::add_provider_for_screen(
                &screen,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    }

    window.show_all();

    let opening = window.clone();
    glib::spawn_future_local(async move {
        match Link::open().await {
            Ok(link) => furnish(&opening, link).await,
            // A window full of controls that cannot reach the dock is worse
            // than a sentence saying so: every one of them would look like it
            // had worked and changed nothing.
            Err(e) => {
                tracing::warn!("no daemon to talk to: {e:#}");
                opening.add(&trouble(
                    "Doca is not running.",
                    "Start it and open this window again.",
                ));
                opening.show_all();
            }
        }
    });
}

async fn furnish(window: &gtk::ApplicationWindow, link: Link) {
    let notebook = gtk::Notebook::new();
    let tab = appearance::Tab::new();
    let writing = link.clone();
    tab.wire(std::rc::Rc::new(move |key, value| writing.set(key, value)));
    notebook.append_page(&tab.root, Some(&gtk::Label::new(Some("Appearance"))));

    // The other three tabs of #21 arrive with their own slices: Docks (F3),
    // Widgets (F4) and Shortcuts (F5). They are named here rather than hidden
    // so the window says what it will hold, and each slice fills one in.
    for (name, holds) in [
        ("Docks", "Which docks exist, what each pins, and the workspaces it claims."),
        ("Widgets", "Which widgets each dock shows, and the settings they take."),
        ("Shortcuts", "The keys that switch docks, through GNOME's own keybindings."),
    ] {
        notebook.append_page(
            &soon(holds),
            Some(&gtk::Label::new(Some(name))),
        );
    }

    window.add(&notebook);
    if let Some(appearance) = link.appearance().await {
        tab.show(&appearance);
    }
    window.show_all();

    // Someone else may be writing: a key bound to SetAppearance, or the dock's
    // own context menu. The controls follow the config rather than remembering
    // what this window last sent.
    let Ok(mut changed) = link.proxy().receive_config_changed().await else {
        tracing::warn!("cannot follow the config; the controls may go stale");
        return;
    };
    while futures_util::StreamExt::next(&mut changed).await.is_some() {
        if let Some(appearance) = link.appearance().await {
            tracing::info!(theme = %appearance.theme, "the config moved; following it");
            tab.show(&appearance);
        }
    }
}

fn soon(holds: &str) -> gtk::Widget {
    let note = gtk::Label::new(Some(holds));
    note.set_widget_name("trouble");
    note.set_line_wrap(true);
    note.set_max_width_chars(44);
    note.set_valign(gtk::Align::Center);
    note.style_context().add_class("dim-label");
    note.upcast()
}

fn trouble(what: &str, then: &str) -> gtk::Widget {
    let said = gtk::Box::new(gtk::Orientation::Vertical, 6);
    said.set_widget_name("trouble");
    said.set_valign(gtk::Align::Center);

    let first = gtk::Label::new(Some(what));
    let second = gtk::Label::new(Some(then));
    second.style_context().add_class("dim-label");
    said.add(&first);
    said.add(&second);
    said.upcast()
}

/// The checks that need a real X display, run in the order GTK demands.
///
/// ```text
/// DISPLAY=:9 cargo test -p doca-prefs -- --ignored
/// ```
#[cfg(test)]
#[test]
#[ignore = "needs an X display; run inside scripts/dev-session.sh"]
fn on_a_display() {
    gtk::init().expect("no X display");

    appearance::on_a_display::showing_what_the_daemon_said_sends_nothing_back();
    appearance::on_a_display::the_controls_show_the_look_they_were_given();
    appearance::on_a_display::a_lens_turned_off_shows_as_a_switch_that_is_off();
    appearance::on_a_display::moving_a_control_sends_exactly_the_key_that_moved();
    appearance::on_a_display::turning_the_lens_on_sends_a_scale_that_is_really_on();
}
