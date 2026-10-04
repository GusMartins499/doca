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
mod keys;
mod link;
mod shortcuts;
mod widgets;

use std::rc::Rc;

use gtk::prelude::*;
use tracing_subscriber::EnvFilter;

use crate::keys::{Desktop, Gnome};
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

    // The desktop, not the daemon: this is the one tab that writes GNOME's
    // settings rather than the dock's. `None` is a desktop that keeps no such
    // list, and the tab says so rather than drawing buttons that would take a
    // key press and drop it.
    let desktop = Gnome::found().map(Rc::new);
    if desktop.is_none() {
        tracing::info!("no GNOME custom keybindings on this desktop");
    }
    let hotkeys = Rc::new(shortcuts::Tab::new(desktop.is_some()));
    if let Some(desktop) = desktop.clone() {
        let saying = hotkeys.clone();
        let following = link.clone();
        hotkeys.wire(Rc::new(move |ask| {
            carry_out(&*desktop, &saying, ask);
            // Written to the desktop, so no `ConfigChanged` is coming to
            // redraw this tab. It asks the desktop back for what it now
            // holds, which is also how a refusal leaves the old key on screen.
            let saying = saying.clone();
            let following = following.clone();
            let desktop = desktop.clone();
            glib::spawn_future_local(async move {
                if let Some(docks) = following.environments().await {
                    show_keys(&*desktop, &saying, &docks);
                }
            });
        }));
    }
    notebook.append_page(&hotkeys.root, Some(&gtk::Label::new(Some("Shortcuts"))));

    window.add(&notebook);
    if let Some(apps) = link.applications().await {
        tracing::info!(applications = apps.len(), "offering the installed apps");
        docks.offer(apps);
    }
    refresh(&link, &look, &docks, &gadgets, desktop.as_deref(), &hotkeys).await;
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
        refresh(&link, &look, &docks, &gadgets, desktop.as_deref(), &hotkeys).await;
    }
}

/// Carry out one keybinding change, and say what it cost.
///
/// A key another application holds is bound anyway and said; a key one of our
/// own actions holds is refused and said. The asymmetry is deliberate: this
/// window owns both sides of the second collision and clearing one is a click
/// away, where unbinding a stranger's key would be the dock deciding who owns
/// a key it does not own either.
fn carry_out(desktop: &impl Desktop, tab: &shortcuts::Tab, ask: shortcuts::Ask) {
    match ask {
        shortcuts::Ask::Bind { action, key } => {
            match keys::clash(desktop, &key, &action, &ours(desktop)) {
                Some(mine @ keys::Clash::Ours(_)) => {
                    tab.complain(&shortcuts::said(&mine, &key));
                }
                Some(theirs) => {
                    keys::bind(desktop, &action, &key);
                    tab.warn(&shortcuts::said(&theirs, &key));
                }
                None => keys::bind(desktop, &action, &key),
            }
        }
        shortcuts::Ask::Clear(action) => keys::unbind(desktop, &action),
    }
}

/// The Doca actions the desktop currently has a key for.
///
/// Read from the desktop rather than from the dock's list of environments, so
/// that a slot left behind by a dock that has since been renamed is still
/// recognised as ours when a key collides with it.
fn ours(desktop: &impl Desktop) -> Vec<keys::Action> {
    keys::all(desktop)
        .into_iter()
        .filter_map(|(slot, held)| {
            let named = held.name.strip_prefix("Doca: ")?;
            let action = if named == "cycle environment" {
                keys::Action::Cycle
            } else {
                keys::Action::Switch(named.to_string())
            };
            (keys::slot(&action) == slot).then_some(action)
        })
        .collect()
}

/// Put the keys the desktop holds on the Shortcuts tab.
fn show_keys(desktop: &impl Desktop, tab: &shortcuts::Tab, docks: &[doca_ipc::EnvironmentInfo]) {
    let mut actions = vec![keys::Action::Cycle];
    actions.extend(
        docks
            .iter()
            .map(|dock| keys::Action::Switch(dock.name.clone())),
    );
    let bound = keys::bound(desktop, &actions);
    // Said with a count because it is the only way, from outside, to tell a
    // window that read the desktop's keys from a window that drew empty rows.
    tracing::info!(keys = bound.len(), "showing the keys the desktop holds");
    tab.show(docks, &bound);
}

/// Put what the daemon says on the controls, in every tab that has any.
async fn refresh(
    link: &Link,
    look: &appearance::Tab,
    docks: &docks::Tab,
    gadgets: &widgets::Tab,
    desktop: Option<&Gnome>,
    hotkeys: &shortcuts::Tab,
) {
    if let Some(appearance) = link.appearance().await {
        look.show(&appearance);
    }
    if let Some(environments) = link.environments().await {
        // Both tabs read the same list: the Docks tab to edit it, the Widgets
        // tab to say which docks show the widget being looked at.
        docks.show(&environments);
        gadgets.show_docks(&environments);
        // A dock added, removed or renamed changes which rows the Shortcuts
        // tab has — and a rename leaves the old dock's slot behind, which is
        // why the keys are read again rather than carried over.
        if let Some(desktop) = desktop {
            show_keys(desktop, hotkeys, &environments);
        }
    }
    if let Some(settings) = link.widget_settings().await {
        gadgets.show(&settings);
    }
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
    docks::on_a_display::moving_a_dock_sends_the_whole_new_order();
    docks::on_a_display::a_dock_at_the_top_cannot_be_moved_off_the_list();
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
    widgets::on_a_display::a_widget_nobody_answers_to_still_appears_in_the_list();
    widgets::on_a_display::picking_a_stranger_says_what_is_wrong_and_offers_nothing();
    widgets::on_a_display::a_list_rebuilt_around_a_stranger_keeps_what_was_selected();
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

    shortcuts::on_a_display::showing_what_the_daemon_said_asks_for_nothing();
    shortcuts::on_a_display::there_is_a_row_for_the_cycle_and_one_for_each_dock();
    shortcuts::on_a_display::a_key_already_bound_is_what_the_button_reads();
    shortcuts::on_a_display::a_captured_key_names_the_row_it_was_pressed_on();
    shortcuts::on_a_display::a_key_pressed_without_clicking_first_binds_nothing();
    shortcuts::on_a_display::a_modifier_on_its_own_keeps_the_button_listening();
    shortcuts::on_a_display::escape_leaves_the_key_that_was_there();
    shortcuts::on_a_display::backspace_asks_for_the_key_to_go();
    shortcuts::on_a_display::the_clear_button_asks_for_the_same_thing();
    shortcuts::on_a_display::a_key_with_no_modifier_is_refused_and_said();
    shortcuts::on_a_display::shift_alone_is_not_enough_of_a_modifier();
    shortcuts::on_a_display::shift_with_a_real_modifier_is_fine();
    shortcuts::on_a_display::a_lock_that_happens_to_be_on_is_not_part_of_the_shortcut();
    shortcuts::on_a_display::a_dock_that_cannot_own_a_slot_cannot_be_captured();
    shortcuts::on_a_display::a_key_one_of_our_own_actions_holds_is_named_as_ours();
    shortcuts::on_a_display::a_key_another_application_holds_is_bound_but_never_in_silence();
    shortcuts::on_a_display::a_desktop_without_gnome_s_shortcuts_says_so_instead();
}
