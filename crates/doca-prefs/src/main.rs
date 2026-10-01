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
mod chrome;
mod docks;
mod link;
mod widgets;

use std::rc::Rc;

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
    // Big enough for the widest tab — the Docks one — so switching tabs does
    // not resize the window out from under the pointer.
    window.set_default_size(640, 520);

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

    let look = appearance::Tab::new();
    let writing = link.clone();
    look.wire(Rc::new(move |key, value| writing.set(key, value)));
    notebook.append_page(&look.root, Some(&gtk::Label::new(Some("Appearance"))));

    // An Rc because the tab is both what the actions are sent from and where
    // a refusal is shown, and the closure that sends outlives this function.
    let docks = Rc::new(docks::Tab::new());
    let writing = link.clone();
    let complaining = docks.clone();
    docks.wire(Rc::new(move |action| {
        let writing = writing.clone();
        let complaining = complaining.clone();
        glib::spawn_future_local(async move {
            // No refresh here on success: the daemon announces the change and
            // the one follower below redraws, so a window that wrote and a
            // window that only watched end up showing the same thing.
            if let Err(why) = writing.apply(action).await {
                tracing::warn!("the dock change was refused: {why}");
                complaining.complain(&why);
            }
        });
    }));
    notebook.append_page(&docks.root, Some(&gtk::Label::new(Some("Docks"))));

    let gadgets = Rc::new(widgets::Tab::new());
    let writing = link.clone();
    let complaining = gadgets.clone();
    gadgets.wire(Rc::new(move |wrote| {
        let writing = writing.clone();
        let complaining = complaining.clone();
        glib::spawn_future_local(async move {
            if let Err(why) = writing.put(wrote).await {
                tracing::warn!("the widget setting was refused: {why}");
                complaining.complain(&why);
            }
        });
    }));
    notebook.append_page(&gadgets.root, Some(&gtk::Label::new(Some("Widgets"))));

    // The last tab of #21 arrives with its own slice: Shortcuts (F5). It is
    // named here rather than hidden so the window says what it will hold.
    notebook.append_page(
        &soon("The keys that switch docks, through GNOME's own keybindings."),
        Some(&gtk::Label::new(Some("Shortcuts"))),
    );

    window.add(&notebook);
    if let Some(apps) = link.applications().await {
        tracing::info!(applications = apps.len(), "offering the installed apps");
        docks.offer(apps);
    }
    refresh(&link, &look, &docks, &gadgets).await;
    window.show_all();

    // Someone else may be writing: a key bound to SetAppearance, the dock's
    // own context menu, a pin from the bar. The controls follow the config
    // rather than remembering what this window last sent.
    let Ok(mut changed) = link.proxy().receive_config_changed().await else {
        tracing::warn!("cannot follow the config; the controls may go stale");
        return;
    };
    // Said once the subscription exists, not once the window is on screen —
    // the window is drawn well before this, and anything watching from
    // outside has to know when the window can actually hear.
    tracing::info!("following the config");
    while futures_util::StreamExt::next(&mut changed).await.is_some() {
        tracing::info!("the config moved; following it");
        refresh(&link, &look, &docks, &gadgets).await;
    }
}

/// Put what the daemon says on the controls, in every tab that has any.
async fn refresh(
    link: &Link,
    look: &appearance::Tab,
    docks: &docks::Tab,
    gadgets: &widgets::Tab,
) {
    if let Some(appearance) = link.appearance().await {
        look.show(&appearance);
    }
    if let Some(environments) = link.environments().await {
        // Both tabs read the same list: the Docks tab to edit it, the Widgets
        // tab to say which docks show the widget being looked at.
        docks.show(&environments);
        gadgets.show_docks(&environments);
    }
    if let Some(settings) = link.widget_settings().await {
        gadgets.show(&settings);
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

    docks::on_a_display::showing_what_the_daemon_said_asks_for_nothing();
    docks::on_a_display::selecting_a_dock_shows_what_that_dock_holds();
    docks::on_a_display::a_change_names_the_dock_that_is_selected();
    docks::on_a_display::ticking_a_widget_keeps_the_ones_the_dock_already_had();
    docks::on_a_display::unpinning_names_the_app_the_row_points_at();
    docks::on_a_display::a_pin_at_the_top_cannot_be_moved_off_the_list();
    docks::on_a_display::moving_a_pin_down_sends_the_whole_new_order();
    docks::on_a_display::a_new_dock_is_asked_for_by_a_name_nothing_is_using();
    docks::on_a_display::the_last_dock_cannot_be_asked_to_go();
    docks::on_a_display::a_workspace_that_is_not_a_number_is_said_rather_than_sent();
    docks::on_a_display::renaming_waits_for_the_name_to_be_finished();

    widgets::on_a_display::showing_what_the_daemon_said_asks_for_nothing();
    widgets::on_a_display::the_controls_show_the_settings_they_were_given();
    widgets::on_a_display::selecting_a_widget_shows_that_widgets_own_page();
    widgets::on_a_display::a_widget_with_nothing_to_set_says_so();
    widgets::on_a_display::the_docks_that_show_a_widget_are_named();
    widgets::on_a_display::a_widget_no_dock_shows_is_told_where_to_turn_it_on();
    widgets::on_a_display::a_spin_writes_its_number_to_its_own_widget_and_key();
    widgets::on_a_display::the_water_goal_is_not_written_to_the_timer();
    widgets::on_a_display::a_finished_date_goes_out_with_the_countdowns_name_on_it();
    widgets::on_a_display::a_date_is_not_sent_until_it_is_finished();
    widgets::on_a_display::a_date_that_is_not_a_date_is_said_rather_than_sent();
    widgets::on_a_display::clearing_the_date_is_allowed();
    widgets::on_a_display::the_caption_is_written_to_the_caption_and_not_the_date();
    widgets::on_a_display::a_note_that_was_only_looked_at_is_not_written_back();
    widgets::on_a_display::saving_the_note_sends_every_line_of_it();
    widgets::on_a_display::a_refresh_leaves_the_cursor_where_it_was_in_an_unchanged_note();
}
