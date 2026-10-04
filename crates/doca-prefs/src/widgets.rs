//! The Widgets tab: what each widget is set to, and where it shows.
//!
//! The Docks tab decides *which* docks show a widget, because that is a
//! property of the dock. This decides what the widget itself is set to, which
//! is one setting for the whole config no matter how many docks show it — the
//! countdown counts to one date. Saying so out loud is what the "Shown in"
//! line is for: a user who sets a date and sees nothing has a widget no dock
//! asked for, and the window should say that rather than look broken.

use std::cell::RefCell;
use std::rc::Rc;

use doca_ipc::{widget_key as key, EnvironmentInfo, WidgetSettings};
use gtk::prelude::*;

use crate::chrome::{hint, listed, pretty, scrolling, titled, Filling};

/// One setting, named the way the bus names it.
///
/// The two shapes are the two the daemon's `Setting` has, so a test here
/// asserts the exact triple that will go on the wire — a control that wrote
/// the right number to the wrong key is the failure worth catching, and it
/// looks identical from the outside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wrote {
    Text {
        widget: &'static str,
        key: &'static str,
        text: String,
    },
    Count {
        widget: &'static str,
        key: &'static str,
        count: u32,
    },
}

pub type Put = Rc<dyn Fn(Wrote)>;

/// Whether this widget has anything to set.
///
/// Derived from the contract rather than listed again here, so a widget that
/// gains a setting gains a page in this tab by saying so in one place.
pub fn takes_settings(id: &str) -> bool {
    key::ALL.iter().any(|(widget, _)| *widget == id)
}

/// Which docks show this widget, in the order the daemon listed them.
pub fn shown_in(docks: &[EnvironmentInfo], id: &str) -> Vec<String> {
    docks
        .iter()
        .filter(|dock| dock.widgets.iter().any(|held| held == id))
        .map(|dock| dock.name.clone())
        .collect()
}

/// The sentence under the heading, including the useful negative one.
pub fn shows(docks: &[EnvironmentInfo], id: &str) -> String {
    let names = shown_in(docks, id);
    if names.is_empty() {
        return "No dock shows this widget. Tick it for a dock in the Docks tab.".to_string();
    }
    format!("Shown in {}.", listed(&names))
}

/// Widget ids the docks ask for that nobody answers to.
///
/// The daemon drops these with a `warn` when it builds the hub, so they are
/// invisible everywhere else: the config says a dock shows one, the bar shows
/// nothing in its place, and this window used to agree with the bar. A typo in
/// a hand-edited TOML therefore looked exactly like a widget that had not been
/// turned on. First seen first, so the order does not shuffle between
/// refreshes.
pub fn unknown_in(docks: &[EnvironmentInfo]) -> Vec<String> {
    let mut strangers: Vec<String> = Vec::new();
    for dock in docks {
        for id in &dock.widgets {
            if !doca_ipc::WIDGETS.contains(&id.as_str()) && !strangers.contains(id) {
                strangers.push(id.clone());
            }
        }
    }
    strangers
}

/// Whether this is a widget the dock actually has.
pub fn is_known(id: &str) -> bool {
    doca_ipc::WIDGETS.contains(&id)
}

/// The sentence under the heading for a widget nothing answers to.
///
/// It names the docks asking for it and says where to take it out, because the
/// window cannot: the Docks tab's tick boxes are built from the widgets that
/// exist, so an id that does not exist has no box to untick.
pub fn stranger(docks: &[EnvironmentInfo], id: &str) -> String {
    let names = shown_in(docks, id);
    format!(
        "Nothing answers to this name, so {} shows nothing in its place. \
         Fix the spelling or remove it from the widgets list in \
         ~/.config/doca/config.toml.",
        if names.is_empty() {
            "the dock".to_string()
        } else {
            listed(&names)
        }
    )
}

/// Why a typed date cannot be sent, if it cannot.
///
/// Checked here rather than left to the daemon because the daemon accepts any
/// text — the widget is what fails, and it fails by showing "bad date" in the
/// bar with no room to explain. Empty is not an error: it is how a countdown
/// is turned off.
pub fn date_trouble(typed: &str) -> Option<String> {
    if typed.trim().is_empty() || doca_ipc::parse_date(typed).is_some() {
        return None;
    }
    Some(format!("{} is not a date. Write it as 2026-12-25.", typed.trim()))
}

#[derive(Default)]
struct State {
    docks: RefCell<Vec<EnvironmentInfo>>,
    /// The ids in the list, in its order — the known ones, and then whatever
    /// the config asks for that nobody answers to. Read instead of indexing
    /// `WIDGETS` directly, which is what tied the list to that constant.
    shown: RefCell<Vec<String>>,
    selected: RefCell<Option<String>>,
    /// The note as it last stood, to tell a note that was edited from one that
    /// was only looked at. Without it every click away from the text view is a
    /// write and a `ConfigChanged` to everything listening, including on a tab
    /// switch where nothing was typed.
    note: RefCell<String>,
    /// Load-bearing here, unlike in the Docks tab. The two spin buttons write
    /// on every change of value, so filling them from the daemon writes
    /// straight back — measured: breaking this flag makes the "a refresh asks
    /// for nothing" check send the timer's minutes and the water goal on every
    /// refresh. The entries and the note go out when the user says they are
    /// done and would need no guard of their own.
    filling: Filling,
}

pub struct Tab {
    pub root: gtk::Widget,
    list: gtk::ListBox,
    heading: gtk::Label,
    where_shown: gtk::Label,
    pages: gtk::Stack,
    date: gtk::Entry,
    label: gtk::Entry,
    note: gtk::TextView,
    save_note: gtk::Button,
    minutes: gtk::Adjustment,
    goal: gtk::Adjustment,
    trouble: gtk::Label,
    state: Rc<State>,
}

/// The stack page shown for a widget that takes nothing.
const NOTHING: &str = "nothing";

impl Tab {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        root.set_margin(14);

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        let left = scrolling(&list);
        left.set_size_request(150, -1);
        root.pack_start(&left, false, false, 0);

        let right = gtk::Box::new(gtk::Orientation::Vertical, 8);
        right.set_hexpand(true);

        let heading = gtk::Label::new(None);
        heading.set_xalign(0.0);
        right.pack_start(&heading, false, false, 0);

        let where_shown = hint("");
        right.pack_start(&where_shown, false, false, 0);

        let pages = gtk::Stack::new();
        pages.set_vexpand(true);

        let date = gtk::Entry::new();
        date.set_placeholder_text(Some("2026-12-25"));
        let label = gtk::Entry::new();
        label.set_placeholder_text(Some("what the day is"));
        let countdown = gtk::Box::new(gtk::Orientation::Vertical, 8);
        countdown.pack_start(&titled("Date", &date), false, false, 0);
        countdown.pack_start(&titled("Caption", &label), false, false, 0);
        countdown.pack_start(
            &hint(
                "Press Enter to set either. An empty date turns the countdown off \
                 and leaves it showing a dash.",
            ),
            false,
            false,
            0,
        );
        pages.add_named(&countdown, key::COUNTDOWN);

        let note = gtk::TextView::new();
        note.set_wrap_mode(gtk::WrapMode::WordChar);
        note.set_accepts_tab(false);
        let save_note = gtk::Button::with_label("Save note");
        save_note.set_halign(gtk::Align::End);
        let note_page = gtk::Box::new(gtk::Orientation::Vertical, 8);
        note_page.pack_start(&scrolling(&note), true, true, 0);
        note_page.pack_start(
            &hint(
                "The first line is what the dock shows; the rest is the caption \
                 under it. Saved when you click away or press the button — Enter \
                 belongs in a note.",
            ),
            false,
            false,
            0,
        );
        note_page.pack_start(&save_note, false, false, 0);
        pages.add_named(&note_page, key::NOTE);

        let minutes = gtk::Adjustment::new(
            10.0,
            doca_ipc::MIN_TIMER_MINUTES as f64,
            doca_ipc::MAX_TIMER_MINUTES as f64,
            1.0,
            5.0,
            0.0,
        );
        let timer = gtk::Box::new(gtk::Orientation::Vertical, 8);
        timer.pack_start(
            &titled("Minutes", &spin(&minutes)),
            false,
            false,
            0,
        );
        timer.pack_start(
            &hint(
                "A timer that has not been started takes the new length at once. \
                 One already counting keeps its count and takes it at the next reset.",
            ),
            false,
            false,
            0,
        );
        pages.add_named(&timer, key::TIMER);

        let goal = gtk::Adjustment::new(
            8.0,
            doca_ipc::MIN_WATER_GOAL as f64,
            doca_ipc::MAX_WATER_GOAL as f64,
            1.0,
            4.0,
            0.0,
        );
        let water = gtk::Box::new(gtk::Orientation::Vertical, 8);
        water.pack_start(&titled("Glasses", &spin(&goal)), false, false, 0);
        water.pack_start(
            &hint("Today's count is kept; only the goal it is measured against moves."),
            false,
            false,
            0,
        );
        pages.add_named(&water, key::WATER);

        let nothing = hint("This widget takes no settings — it shows what it reads.");
        nothing.set_valign(gtk::Align::Start);
        pages.add_named(&nothing, NOTHING);

        // A stack can only show a child that is itself visible, and the
        // window's own `show_all` comes later than the first selection does.
        pages.show_all();
        right.pack_start(&pages, true, true, 0);

        let trouble = gtk::Label::new(None);
        trouble.set_widget_name("hint");
        trouble.set_xalign(0.0);
        trouble.set_line_wrap(true);
        trouble.style_context().add_class("error");
        right.pack_start(&trouble, false, false, 0);

        root.pack_start(&right, true, true, 0);

        let tab = Self {
            root: root.upcast(),
            list,
            heading,
            where_shown,
            pages,
            date,
            label,
            note,
            save_note,
            minutes,
            goal,
            trouble,
            state: Rc::new(State::default()),
        };
        tab.list_widgets(&[]);
        tab
    }

    /// Fill the list: every widget there is, and then any the config asks for
    /// that there is not.
    fn list_widgets(&self, strangers: &[String]) {
        let shown: Vec<String> = doca_ipc::WIDGETS
            .iter()
            .map(|id| id.to_string())
            .chain(strangers.iter().cloned())
            .collect();
        if *self.state.shown.borrow() == shown {
            return;
        }

        let selected = self.state.selected.borrow().clone();
        self.state.filling.while_filling(|| {
            for row in self.list.children() {
                self.list.remove(&row);
            }
            for id in &shown {
                let label = gtk::Label::new(None);
                label.set_xalign(0.0);
                label.set_margin(6);
                if is_known(id) {
                    label.set_text(&pretty(id));
                } else {
                    // The id as the file spells it, not prettified: what is
                    // wrong with it is usually the spelling.
                    label.set_markup(&format!(
                        "{}  <small>unknown</small>",
                        glib::markup_escape_text(id)
                    ));
                    label.style_context().add_class("dim-label");
                }
                let row = gtk::ListBoxRow::new();
                row.add(&label);
                self.list.add(&row);
            }
            self.list.show_all();
            if let Some(at) = selected
                .as_ref()
                .and_then(|id| shown.iter().position(|shown| shown == id))
            {
                if let Some(row) = self.list.row_at_index(at as i32) {
                    self.list.select_row(Some(&row));
                }
            }
        });
        self.state.shown.replace(shown);
    }

    /// Connect the controls to whatever is listening.
    pub fn wire(&self, put: Put) {
        let state = self.state.clone();
        let heading = self.heading.clone();
        let where_shown = self.where_shown.clone();
        let pages = self.pages.clone();
        let trouble = self.trouble.clone();
        self.list.connect_row_selected(move |_, row| {
            if state.filling.is_filling() {
                return;
            }
            let Some(row) = row else { return };
            let Some(id) = state.shown.borrow().get(row.index() as usize).cloned() else {
                return;
            };
            state.selected.replace(Some(id));
            trouble.set_text("");
            show_selected(&state, &heading, &where_shown, &pages);
        });

        for (entry, name) in [
            (&self.date, key::DATE),
            (&self.label, key::LABEL),
        ] {
            let writing = put.clone();
            let trouble = self.trouble.clone();
            // `activate` rather than `changed`: a date is nonsense until it is
            // finished being typed, and 2 is not the year 2026.
            entry.connect_activate(move |entry| {
                let text = entry.text().to_string();
                if name == key::DATE {
                    if let Some(why) = date_trouble(&text) {
                        trouble.set_text(&why);
                        return;
                    }
                }
                trouble.set_text("");
                writing(Wrote::Text {
                    widget: key::COUNTDOWN,
                    key: name,
                    text,
                });
            });
        }

        let writing = put.clone();
        let note = self.note.clone();
        let state = self.state.clone();
        let send_note: Rc<dyn Fn()> = Rc::new(move || {
            let buffer = note.buffer().expect("a text view has a buffer");
            let (from, to) = buffer.bounds();
            let text = buffer.text(&from, &to, false).unwrap_or_default().to_string();
            if *state.note.borrow() == text {
                return;
            }
            state.note.replace(text.clone());
            writing(Wrote::Text {
                widget: key::NOTE,
                key: key::TEXT,
                text,
            });
        });

        let sending = send_note.clone();
        self.save_note.connect_clicked(move |_| sending());
        let sending = send_note.clone();
        // Clicking away is what most people will do instead of finding the
        // button, and a note that was typed and lost is worse than a note
        // saved once more than it needed to be.
        self.note.connect_focus_out_event(move |_, _| {
            sending();
            glib::Propagation::Proceed
        });

        for (adjustment, widget, name) in [
            (&self.minutes, key::TIMER, key::MINUTES),
            (&self.goal, key::WATER, key::GOAL),
        ] {
            let writing = put.clone();
            let filling = self.state.filling.clone();
            adjustment.connect_value_changed(move |value| {
                if filling.is_filling() {
                    return;
                }
                writing(Wrote::Count {
                    widget,
                    key: name,
                    count: value.value().round().max(0.0) as u32,
                });
            });
        }
    }

    /// Say why the last thing the user asked for did not happen.
    pub fn complain(&self, why: &str) {
        self.trouble.set_text(why);
    }

    /// Show the settings the daemon reports, without writing any of it back.
    pub fn show(&self, settings: &WidgetSettings) {
        self.state.filling.while_filling(|| {
            self.date.set_text(&settings.countdown_date);
            self.label.set_text(&settings.countdown_label);
            if let Some(buffer) = self.note.buffer() {
                // Only when it differs: setting the buffer moves the cursor to
                // the end, and a `ConfigChanged` from somebody else's slider
                // must not do that to a note being typed.
                let (from, to) = buffer.bounds();
                let showing = buffer.text(&from, &to, false).unwrap_or_default();
                if showing != settings.note_text {
                    buffer.set_text(&settings.note_text);
                }
                self.state.note.replace(settings.note_text.clone());
            }
            self.minutes.set_value(settings.timer_minutes as f64);
            self.goal.set_value(settings.water_goal as f64);
        });
        self.trouble.set_text("");
    }

    /// Which docks show what, for the line under the heading. Called with the
    /// same list the Docks tab is given.
    pub fn show_docks(&self, docks: &[EnvironmentInfo]) {
        self.state.docks.replace(docks.to_vec());
        // A widget nobody answers to only exists in the docks' own lists, so
        // this is the only place it can be noticed.
        self.list_widgets(&unknown_in(docks));
        if self.state.selected.borrow().is_none() {
            // Nothing picked yet, so pick the first — a blank right-hand pane
            // reads as a tab that failed to load.
            if let Some(row) = self.list.row_at_index(0) {
                self.list.select_row(Some(&row));
            }
        }
        show_selected(
            &self.state,
            &self.heading,
            &self.where_shown,
            &self.pages,
        );
    }
}

/// Put the selected widget's name, whereabouts and page on screen.
fn show_selected(
    state: &Rc<State>,
    heading: &gtk::Label,
    where_shown: &gtk::Label,
    pages: &gtk::Stack,
) {
    let Some(id) = state.selected.borrow().clone() else {
        return;
    };
    let known = is_known(&id);
    heading.set_markup(&format!(
        "<b>{}</b>",
        glib::markup_escape_text(&if known { pretty(&id) } else { id.clone() })
    ));
    where_shown.set_text(&if known {
        shows(&state.docks.borrow(), &id)
    } else {
        stranger(&state.docks.borrow(), &id)
    });
    pages.set_visible_child_name(if known && takes_settings(&id) { &id } else { NOTHING });
}

fn spin(adjustment: &gtk::Adjustment) -> gtk::SpinButton {
    let spin = gtk::SpinButton::new(Some(adjustment), 1.0, 0);
    spin.set_halign(gtk::Align::Start);
    spin.set_numeric(true);
    spin
}

/// The checks that need real GTK widgets, run from `main`'s single init.
#[cfg(test)]
pub mod on_a_display {
    use super::*;

    type Written = Rc<RefCell<Vec<Wrote>>>;

    fn watched() -> (Tab, Written) {
        let written: Written = Rc::new(RefCell::new(Vec::new()));
        let tab = Tab::new();
        let recording = written.clone();
        tab.wire(Rc::new(move |wrote| recording.borrow_mut().push(wrote)));
        (tab, written)
    }

    fn settings() -> WidgetSettings {
        WidgetSettings {
            countdown_date: "2026-12-25".to_string(),
            countdown_label: "Christmas".to_string(),
            note_text: "milk\nand bread".to_string(),
            timer_minutes: 10,
            water_goal: 8,
        }
    }

    fn dock(name: &str, widgets: &[&str]) -> EnvironmentInfo {
        EnvironmentInfo {
            name: name.to_string(),
            workspaces: Vec::new(),
            current: false,
            pinned: Vec::new(),
            widgets: widgets.iter().map(|id| id.to_string()).collect(),
        }
    }

    fn at(tab: &Tab, id: &str) {
        let index = doca_ipc::WIDGETS
            .iter()
            .position(|known| *known == id)
            .expect("a widget that is on offer");
        let row = tab.list.row_at_index(index as i32).expect("a row per widget");
        tab.list.select_row(Some(&row));
    }

    fn note_of(tab: &Tab) -> String {
        let buffer = tab.note.buffer().expect("a text view has a buffer");
        let (from, to) = buffer.bounds();
        buffer.text(&from, &to, false).unwrap_or_default().to_string()
    }

    /// The one that matters: filling the controls must not write anything.
    pub fn showing_what_the_daemon_said_asks_for_nothing() {
        let (tab, written) = watched();

        tab.show_docks(&[dock("Work", &["timer"])]);
        tab.show(&settings());
        tab.show(&WidgetSettings {
            timer_minutes: 25,
            water_goal: 12,
            ..settings()
        });

        assert!(
            written.borrow().is_empty(),
            "the window answered its own refresh: {:?}",
            written.borrow()
        );
    }

    pub fn the_controls_show_the_settings_they_were_given() {
        let (tab, _) = watched();

        tab.show(&settings());

        assert_eq!(tab.date.text(), "2026-12-25");
        assert_eq!(tab.label.text(), "Christmas");
        assert_eq!(note_of(&tab), "milk\nand bread");
        assert_eq!(tab.minutes.value(), 10.0);
        assert_eq!(tab.goal.value(), 8.0);
    }

    pub fn selecting_a_widget_shows_that_widgets_own_page() {
        let (tab, _) = watched();
        tab.show_docks(&[dock("Work", &[])]);

        at(&tab, "timer");
        assert_eq!(tab.pages.visible_child_name().as_deref(), Some("timer"));

        at(&tab, "water");
        assert_eq!(tab.pages.visible_child_name().as_deref(), Some("water"));
    }

    /// Ten of the twelve take no settings, and a blank pane would read as a
    /// tab that failed rather than a widget with nothing to set.
    /// The issue's own words: it should appear marked as unknown rather than
    /// vanishing quietly. Before this it was not in the list at all, because
    /// the list was the contract's twelve and nothing else.
    pub fn a_widget_nobody_answers_to_still_appears_in_the_list() {
        let tab = Tab::new();

        tab.show_docks(&[dock("Work", &["clock", "clok"])]);

        let shown = tab.state.shown.borrow().clone();
        assert_eq!(
            shown.len(),
            doca_ipc::WIDGETS.len() + 1,
            "the stranger did not join the list"
        );
        assert_eq!(shown.last().map(String::as_str), Some("clok"));
        assert_eq!(tab.list.children().len(), shown.len(), "a row short");
    }

    /// Picking it must say what is wrong, and must not offer settings for a
    /// widget that does not exist to take them.
    pub fn picking_a_stranger_says_what_is_wrong_and_offers_nothing() {
        let (tab, _) = watched();
        tab.show_docks(&[dock("Work", &["clok"])]);

        let at = tab.state.shown.borrow().iter().position(|id| id == "clok").unwrap();
        let row = tab.list.row_at_index(at as i32).expect("a row for it");
        tab.list.select_row(Some(&row));

        assert_eq!(tab.state.selected.borrow().as_deref(), Some("clok"));
        assert!(tab.where_shown.text().contains("config.toml"), "{}", tab.where_shown.text());
        assert_eq!(
            tab.pages.visible_child_name().map(|n| n.to_string()).as_deref(),
            Some(NOTHING),
            "a widget that does not exist was offered settings"
        );
    }

    /// The list is rebuilt when a stranger appears or goes, and rebuilding a
    /// list fires `row_selected` — which must not be read as the user picking
    /// something, nor lose what they had picked.
    pub fn a_list_rebuilt_around_a_stranger_keeps_what_was_selected() {
        let (tab, asked) = watched();
        tab.show_docks(&[dock("Work", &["clock"])]);
        let at = tab.state.shown.borrow().iter().position(|id| id == "water").unwrap();
        tab.list.select_row(tab.list.row_at_index(at as i32).as_ref());
        asked.borrow_mut().clear();

        tab.show_docks(&[dock("Work", &["clock", "clok"])]);

        assert_eq!(
            tab.state.selected.borrow().as_deref(),
            Some("water"),
            "the selection moved when the list grew"
        );
        assert!(asked.borrow().is_empty(), "a rebuild wrote: {:?}", asked.borrow());
    }

    pub fn a_widget_with_nothing_to_set_says_so() {
        let (tab, _) = watched();
        tab.show_docks(&[dock("Work", &[])]);

        at(&tab, "clock");

        assert_eq!(tab.pages.visible_child_name().as_deref(), Some(NOTHING));
    }

    pub fn the_docks_that_show_a_widget_are_named() {
        let (tab, _) = watched();

        tab.show_docks(&[dock("Work", &["timer"]), dock("Home", &["timer", "clock"])]);
        at(&tab, "timer");

        assert_eq!(tab.where_shown.text(), "Shown in Work and Home.");
    }

    /// The question this tab exists to answer: a date was set and nothing
    /// appeared, because no dock asked for the countdown.
    pub fn a_widget_no_dock_shows_is_told_where_to_turn_it_on() {
        let (tab, _) = watched();

        tab.show_docks(&[dock("Work", &["clock"])]);
        at(&tab, "countdown");

        assert!(
            tab.where_shown.text().contains("Docks tab"),
            "it says {:?} instead of where to turn it on",
            tab.where_shown.text()
        );
    }

    pub fn a_spin_writes_its_number_to_its_own_widget_and_key() {
        let (tab, written) = watched();
        tab.show(&settings());

        tab.minutes.set_value(25.0);

        assert_eq!(
            *written.borrow(),
            vec![Wrote::Count {
                widget: "timer",
                key: "minutes",
                count: 25
            }]
        );
    }

    /// Two spins side by side, both writing a count: sending the water goal
    /// to the timer would look identical from the outside and set the wrong
    /// thing.
    pub fn the_water_goal_is_not_written_to_the_timer() {
        let (tab, written) = watched();
        tab.show(&settings());

        tab.goal.set_value(12.0);

        assert_eq!(
            *written.borrow(),
            vec![Wrote::Count {
                widget: "water",
                key: "goal",
                count: 12
            }]
        );
    }

    pub fn a_finished_date_goes_out_with_the_countdowns_name_on_it() {
        let (tab, written) = watched();
        tab.show(&settings());

        tab.date.set_text("2027-01-01");
        tab.date.emit_activate();

        assert_eq!(
            *written.borrow(),
            vec![Wrote::Text {
                widget: "countdown",
                key: "date",
                text: "2027-01-01".to_string()
            }]
        );
    }

    /// Nothing may go out while the date is still being typed: `2` is a year,
    /// `2026-1` is a January, and both would be written and shown.
    pub fn a_date_is_not_sent_until_it_is_finished() {
        let (tab, written) = watched();
        tab.show(&settings());

        tab.date.set_text("2027-01-01");

        assert!(
            written.borrow().is_empty(),
            "a half-typed date was sent: {:?}",
            written.borrow()
        );
    }

    pub fn a_date_that_is_not_a_date_is_said_rather_than_sent() {
        let (tab, written) = watched();
        tab.show(&settings());

        tab.date.set_text("next tuesday");
        tab.date.emit_activate();

        assert!(
            written.borrow().is_empty(),
            "it sent {:?} instead of complaining",
            written.borrow()
        );
        assert!(
            tab.trouble.text().contains("not a date"),
            "it said {:?}",
            tab.trouble.text()
        );
    }

    /// Empty is how a countdown is turned off, so it is not a bad date.
    pub fn clearing_the_date_is_allowed() {
        let (tab, written) = watched();
        tab.show(&settings());

        tab.date.set_text("");
        tab.date.emit_activate();

        assert_eq!(
            *written.borrow(),
            vec![Wrote::Text {
                widget: "countdown",
                key: "date",
                text: String::new()
            }]
        );
        assert_eq!(tab.trouble.text(), "");
    }

    pub fn the_caption_is_written_to_the_caption_and_not_the_date() {
        let (tab, written) = watched();
        tab.show(&settings());

        tab.label.set_text("Holiday");
        tab.label.emit_activate();

        assert_eq!(
            *written.borrow(),
            vec![Wrote::Text {
                widget: "countdown",
                key: "label",
                text: "Holiday".to_string()
            }]
        );
    }

    /// A click away from a note nobody typed in is not a write — and would
    /// otherwise be one on every tab switch, announced to everything
    /// listening on the bus.
    pub fn a_note_that_was_only_looked_at_is_not_written_back() {
        let (tab, written) = watched();
        tab.show(&settings());

        tab.save_note.emit_clicked();
        tab.save_note.emit_clicked();

        assert!(
            written.borrow().is_empty(),
            "an untouched note was written: {:?}",
            written.borrow()
        );
    }

    pub fn saving_the_note_sends_every_line_of_it() {
        let (tab, written) = watched();
        tab.show(&settings());
        let buffer = tab.note.buffer().expect("a text view has a buffer");
        buffer.set_text("Dentist\nThursday at four");

        tab.save_note.emit_clicked();

        assert_eq!(
            *written.borrow(),
            vec![Wrote::Text {
                widget: "note",
                key: "text",
                text: "Dentist\nThursday at four".to_string()
            }]
        );
    }

    /// A refresh arrives for anything anyone writes, including a slider in
    /// another window. Re-setting the buffer would jump the cursor to the end
    /// of a note being typed, so an unchanged note is left alone.
    pub fn a_refresh_leaves_the_cursor_where_it_was_in_an_unchanged_note() {
        let (tab, _) = watched();
        tab.show(&settings());
        let buffer = tab.note.buffer().expect("a text view has a buffer");
        let third = buffer.iter_at_offset(3);
        buffer.place_cursor(&third);

        tab.show(&settings());

        let insert = buffer.get_insert().expect("a buffer has an insertion point");
        let cursor = buffer.iter_at_mark(&insert);
        assert_eq!(cursor.offset(), 3, "the cursor was moved by a refresh");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dock(name: &str, widgets: &[&str]) -> EnvironmentInfo {
        EnvironmentInfo {
            name: name.to_string(),
            workspaces: Vec::new(),
            current: false,
            pinned: Vec::new(),
            widgets: widgets.iter().map(|id| id.to_string()).collect(),
        }
    }

    #[test]
    fn a_widget_nobody_answers_to_is_picked_out_of_the_docks_lists() {
        let docks = vec![
            dock("Work", &["clock", "clok"]),
            dock("Home", &["clok", "watr"]),
        ];

        assert_eq!(unknown_in(&docks), vec!["clok", "watr"], "first seen, once each");
        assert!(unknown_in(&[dock("Work", &["clock", "water"])]).is_empty());
    }

    #[test]
    fn what_is_said_about_a_stranger_names_the_docks_and_the_way_out() {
        let docks = vec![dock("Work", &["clok"]), dock("Home", &["clok"])];

        let said = stranger(&docks, "clok");

        assert!(said.contains("Work and Home"), "{said}");
        assert!(said.contains("config.toml"), "no way out offered: {said}");
    }

    #[test]
    fn the_four_widgets_with_settings_are_the_four_the_contract_names() {
        let with: Vec<&str> = doca_ipc::WIDGETS
            .iter()
            .copied()
            .filter(|id| takes_settings(id))
            .collect();

        assert_eq!(with, vec!["timer", "countdown", "water", "note"]);
    }

    #[test]
    fn a_widget_that_only_reads_the_system_has_nothing_to_set() {
        for id in ["clock", "battery", "cpu", "network", "music", "stopwatch"] {
            assert!(!takes_settings(id), "{id} was offered settings it has none of");
        }
    }

    #[test]
    fn only_the_docks_that_hold_the_widget_are_listed() {
        let docks = [dock("Work", &["timer", "clock"]), dock("Home", &["clock"])];

        assert_eq!(shown_in(&docks, "timer"), vec!["Work".to_string()]);
        assert_eq!(
            shown_in(&docks, "clock"),
            vec!["Work".to_string(), "Home".to_string()]
        );
        assert!(shown_in(&docks, "water").is_empty());
    }

    #[test]
    fn a_widget_nothing_shows_is_told_where_to_turn_it_on() {
        let said = shows(&[dock("Work", &["clock"])], "water");

        assert!(said.contains("No dock"), "it said {said:?}");
        assert!(said.contains("Docks tab"), "it said {said:?}");
    }

    #[test]
    fn the_docks_showing_a_widget_are_read_out_as_a_sentence() {
        let docks = [dock("Work", &["clock"]), dock("Home", &["clock"])];

        assert_eq!(shows(&docks, "clock"), "Shown in Work and Home.");
    }

    #[test]
    fn a_real_date_has_nothing_wrong_with_it() {
        assert_eq!(date_trouble("2026-12-25"), None);
        assert_eq!(date_trouble(" 2026-12-25 "), None);
    }

    /// Empty is how the countdown is turned off, not a mistake to complain at.
    #[test]
    fn an_empty_date_is_not_a_complaint() {
        assert_eq!(date_trouble(""), None);
        assert_eq!(date_trouble("   "), None);
    }

    #[test]
    fn a_date_that_is_not_one_is_refused_with_the_right_shape_shown() {
        let why = date_trouble("25/12/2026").expect("that is not a date");

        assert!(why.contains("25/12/2026"), "it said {why:?}");
        assert!(why.contains("2026-12-25"), "it did not say the shape: {why:?}");
    }

    /// The daemon would take `2026-13-01` happily and the widget would show
    /// "bad date" in the bar. This is the only place that can say why.
    #[test]
    fn a_month_that_does_not_exist_is_caught_before_it_is_sent() {
        assert!(date_trouble("2026-13-01").is_some());
        assert!(date_trouble("2026-02-40").is_some());
    }
}
