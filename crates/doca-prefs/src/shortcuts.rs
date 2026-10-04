//! The Shortcuts tab: the keys that switch docks.
//!
//! The one tab that does not write the dock's config. The keyboard belongs to
//! the desktop, so what this writes is GNOME's own list of custom keybindings
//! — and the tab says so on screen. A user who later goes looking for the key
//! in Settings → Keyboard has to find it where this put it, and a user who
//! wonders why the dock's settings window is writing the desktop's settings
//! deserves the sentence rather than the surprise.
//!
//! Like the other tabs, nothing here reaches the desktop: a control emits an
//! [`Ask`] and whoever wired the tab carries it out. That is what lets a test
//! assert which key a capture button took and which action it named, with no
//! GSettings schema and no dconf on the machine running the test.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use doca_ipc::EnvironmentInfo;
use gtk::gdk;
use gtk::prelude::*;

use crate::chrome::{hint, scrolling, section};
use crate::keys::{bindable, label, Action, Clash};

/// One change to the desktop's keybindings, as a control asks for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    Bind { action: Action, key: String },
    Clear(Action),
}

pub type Act = Rc<dyn Fn(Ask)>;

/// What one key press during capture means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Press {
    /// A modifier on its own — still waiting for the key it modifies.
    Waiting,
    /// Esc: leave whatever was bound alone.
    Cancel,
    /// Backspace or Delete: unbind.
    Clear,
    Take(String),
    /// Why that press cannot be a shortcut, in words for the window.
    Refuse(String),
}

/// The modifiers that make a shortcut a shortcut.
///
/// Shift is not among them, and that is not an oversight: `<Shift>e` is the
/// letter E. GNOME draws the same line, and a window that let it through would
/// hand back a desktop where typing a capital letter switches docks.
fn qualifying(state: gdk::ModifierType) -> gdk::ModifierType {
    state
        & (gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::MOD1_MASK
            | gdk::ModifierType::SUPER_MASK
            | gdk::ModifierType::HYPER_MASK
            | gdk::ModifierType::META_MASK)
}

/// The modifiers that go into the accelerator's name.
///
/// Shift counts here even though it does not count above: `<Shift><Super>e`
/// is a shortcut of its own, and a different one from `<Super>e`. What is
/// excluded is everything that is a *state* rather than a chord — Num Lock,
/// Caps Lock, the pointer buttons — because a shortcut that carried Num Lock
/// in its name would stop working the moment it was switched off.
fn named(state: gdk::ModifierType) -> gdk::ModifierType {
    qualifying(state) | (state & gdk::ModifierType::SHIFT_MASK)
}

/// Whether this keyval is a modifier rather than a key being modified.
///
/// Pressing Super before E sends Super first. Taking that for the shortcut
/// would make every capture `<Super>` and nothing else, so the press is let
/// through and the button keeps listening.
fn is_modifier(keyval: u32) -> bool {
    use gdk::keys::constants as key;
    [
        *key::Shift_L,
        *key::Shift_R,
        *key::Control_L,
        *key::Control_R,
        *key::Alt_L,
        *key::Alt_R,
        *key::Super_L,
        *key::Super_R,
        *key::Hyper_L,
        *key::Hyper_R,
        *key::Meta_L,
        *key::Meta_R,
        *key::Caps_Lock,
        *key::Shift_Lock,
        *key::Num_Lock,
        *key::ISO_Level3_Shift,
    ]
    .contains(&keyval)
}

/// What a press during capture is asking for.
///
/// Needs GTK initialised: the accelerator's spelling comes from
/// `gtk::accelerator_name`, so the string written to GSettings is the one
/// GNOME itself would have written rather than this window's own idea of how
/// to spell a key.
pub fn press(keyval: u32, state: gdk::ModifierType) -> Press {
    use gdk::keys::constants as key;
    if keyval == *key::Escape {
        return Press::Cancel;
    }
    if keyval == *key::BackSpace || keyval == *key::Delete {
        return Press::Clear;
    }
    if is_modifier(keyval) {
        return Press::Waiting;
    }
    if qualifying(state).is_empty() {
        return Press::Refuse(
            "A shortcut needs Ctrl, Alt or Super held with it — a key on its own, \
             or with only Shift, would answer every time it was typed."
                .to_string(),
        );
    }
    let modifiers = named(state);
    if !gtk::accelerator_valid(keyval, modifiers) {
        return Press::Refuse("That key cannot be a shortcut.".to_string());
    }
    match gtk::accelerator_name(keyval, modifiers) {
        Some(name) => Press::Take(name.to_string()),
        None => Press::Refuse("That key has no name to bind.".to_string()),
    }
}

/// How a key reads on a button, or the word for having none.
fn shown(key: Option<&String>) -> String {
    match key {
        Some(key) => key.clone(),
        None => "Disabled".to_string(),
    }
}

/// The sentence for a key someone else already answers to.
pub fn said(clash: &Clash, key: &str) -> String {
    match clash {
        Clash::Ours(action) => {
            format!("{key} is already {} — clear that one first.", label(action))
        }
        Clash::Theirs(whose) => format!(
            "{key} is also bound to {whose:?} on this desktop. Both are set now, \
             and which one answers is the desktop's choice."
        ),
    }
}

/// What the tab knows, shared with every handler it connects.
#[derive(Default)]
struct State {
    /// The action whose button is listening for a key right now.
    capturing: RefCell<Option<Action>>,
    /// Whoever is carrying out what the controls ask for.
    ///
    /// Behind the shared `State` rather than owned by the `Tab` so that
    /// wiring after the rows are built still reaches them — the alternative
    /// is a cell cloned into each closure, where a later `wire` sets a copy
    /// nobody reads.
    act: RefCell<Option<Act>>,
}

impl State {
    fn ask(&self, ask: Ask) {
        if let Some(act) = self.act.borrow().as_ref() {
            act(ask);
        }
    }
}

/// One line of the tab: an action, what it is bound to, and the two buttons.
struct Row {
    action: Action,
    key: gtk::Button,
    /// Held only so a check can press it. The handler it carries is connected
    /// once, where the row is built.
    #[allow(dead_code)]
    clear: gtk::Button,
    /// What the button read before a capture started, to go back to on Esc.
    was: String,
}

pub struct Tab {
    pub root: gtk::Widget,
    /// Whether this desktop has anywhere to write at all.
    available: bool,
    /// Where the rows are rebuilt into when the docks change.
    lines: gtk::Box,
    trouble: gtk::Label,
    rows: RefCell<Vec<Rc<Row>>>,
    state: Rc<State>,
}

impl Tab {
    /// The tab, told whether this desktop keeps GNOME's custom keybindings.
    ///
    /// `available` false is KDE, sway, or a GNOME without
    /// gnome-settings-daemon's schemas: there is no list to write, and a grid
    /// of capture buttons would take a key press and drop it. One sentence
    /// instead, which is how this window already handles having no daemon.
    pub fn new(available: bool) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.set_margin(14);

        let lines = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let trouble = gtk::Label::new(None);
        trouble.set_widget_name("hint");
        trouble.set_xalign(0.0);
        trouble.set_line_wrap(true);

        if available {
            root.pack_start(&section("Switching docks"), false, false, 0);
            root.pack_start(&scrolling(&lines), true, true, 0);
            root.pack_start(
                &hint(
                    "The dock does not take keys for itself — GNOME does. These are \
                     written to the desktop's own keyboard shortcuts, where you can \
                     see and change them too. Click a key to set it, Backspace to \
                     clear it, Esc to leave it alone.",
                ),
                false,
                false,
                0,
            );
            root.pack_start(&trouble, false, false, 0);
        } else {
            let note = gtk::Label::new(Some(
                "This desktop does not keep GNOME's custom keyboard shortcuts.",
            ));
            let then = gtk::Label::new(Some(
                "Bind a key to Doca's D-Bus actions with your desktop's own keyboard \
                 settings — the README lists them.",
            ));
            then.set_line_wrap(true);
            then.set_max_width_chars(48);
            then.style_context().add_class("dim-label");
            root.set_valign(gtk::Align::Center);
            root.pack_start(&note, false, false, 0);
            root.pack_start(&then, false, false, 0);
        }

        Self {
            root: root.upcast(),
            available,
            lines,
            trouble,
            rows: RefCell::new(Vec::new()),
            state: Rc::new(State::default()),
        }
    }

    /// Connect the controls to whatever is listening.
    pub fn wire(&self, act: Act) {
        self.state.act.replace(Some(act));
    }

    /// Say why the last thing the user asked for did not happen.
    pub fn complain(&self, why: &str) {
        self.trouble.style_context().add_class("error");
        self.trouble.set_text(why);
    }

    /// Say what happened, when it happened but deserves a word.
    ///
    /// A key another application already holds is bound anyway — refusing
    /// would be this window deciding who owns a key it does not own either —
    /// but it is never bound in silence.
    pub fn warn(&self, about: &str) {
        self.trouble.style_context().remove_class("error");
        self.trouble.set_text(about);
    }

    /// Put the docks and their keys on screen.
    ///
    /// Rebuilt rather than reconciled: the list is the cycle plus one row per
    /// dock, so a dock added, removed or renamed changes which rows exist, and
    /// there is no scroll position or half-typed field in here to keep.
    pub fn show(&self, docks: &[EnvironmentInfo], bound: &HashMap<Action, String>) {
        if !self.available {
            return;
        }
        // A capture in flight is dropped: the row it belonged to may not be
        // one of the rows about to exist.
        self.state.capturing.replace(None);
        self.trouble.style_context().remove_class("error");
        self.trouble.set_text("");

        for old in self.lines.children() {
            self.lines.remove(&old);
        }
        self.rows.borrow_mut().clear();

        let names: Vec<String> = docks.iter().map(|dock| dock.name.clone()).collect();
        let mut actions = vec![(Action::Cycle, "Next dock".to_string())];
        actions.extend(
            names
                .iter()
                .map(|name| (Action::Switch(name.clone()), format!("Go to {name}"))),
        );

        for (action, reads) in actions {
            let line = self.line(&action, &reads, bound.get(&action), &names);
            self.lines.pack_start(&line, false, false, 0);
        }
        self.lines.show_all();
    }

    /// One row, wired.
    fn line(
        &self,
        action: &Action,
        reads: &str,
        key: Option<&String>,
        docks: &[String],
    ) -> gtk::Box {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);

        let name = gtk::Label::new(Some(reads));
        name.set_xalign(0.0);
        name.set_hexpand(true);
        line.pack_start(&name, true, true, 0);

        let button = gtk::Button::with_label(&shown(key));
        button.set_size_request(160, -1);
        let clear = gtk::Button::with_label("Clear");
        clear.set_sensitive(key.is_some());
        line.pack_start(&button, false, false, 0);
        line.pack_start(&clear, false, false, 0);

        let row = Rc::new(Row {
            action: action.clone(),
            key: button.clone(),
            clear: clear.clone(),
            was: shown(key),
        });
        self.rows.borrow_mut().push(row.clone());

        // A dock whose name cannot own a slot of its own is said here, rather
        // than discovered when its key turns out to move another dock's.
        if let Err(why) = bindable(action, docks) {
            for dead in [&button, &clear] {
                dead.set_sensitive(false);
                dead.set_tooltip_text(Some(&why));
            }
            name.set_tooltip_text(Some(&why));
            return line;
        }

        let state = self.state.clone();
        let listening = row.clone();
        let trouble = self.trouble.clone();
        button.connect_clicked(move |button| {
            trouble.style_context().remove_class("error");
            trouble.set_text("");
            state.capturing.replace(Some(listening.action.clone()));
            button.set_label("Press a key…");
            // A button only hears a key press while it has the focus.
            button.grab_focus();
        });

        let state = self.state.clone();
        let asking = row.clone();
        let trouble = self.trouble.clone();
        button.connect_key_press_event(move |_, event| {
            pressed(&state, &asking, &trouble, *event.keyval(), event.state())
        });

        let state = self.state.clone();
        let asking = row.clone();
        clear.connect_clicked(move |_| state.ask(Ask::Clear(asking.action.clone())));

        line
    }
}

/// What a key press on a row's capture button does.
///
/// A function of its own, not for reuse but for reach: the handler above is
/// one line calling this, so a test that drives this drives what a real press
/// drives. Synthesising a `GdkEventKey` by hand would test the synthesis.
fn pressed(
    state: &Rc<State>,
    row: &Rc<Row>,
    trouble: &gtk::Label,
    keyval: u32,
    modifiers: gdk::ModifierType,
) -> glib::Propagation {
    // Nobody asked this button to listen. A key that reaches it is a key
    // somebody typed while it happened to hold the focus — Tab onto the row
    // and the next keystroke would otherwise be bound.
    if state.capturing.borrow().as_ref() != Some(&row.action) {
        return glib::Propagation::Proceed;
    }
    match press(keyval, modifiers) {
        // Not done capturing: the button keeps its prompt and keeps listening.
        Press::Waiting => return glib::Propagation::Stop,
        Press::Cancel => {
            state.capturing.replace(None);
            row.key.set_label(&row.was);
        }
        Press::Clear => {
            state.capturing.replace(None);
            state.ask(Ask::Clear(row.action.clone()));
        }
        Press::Take(key) => {
            state.capturing.replace(None);
            state.ask(Ask::Bind {
                action: row.action.clone(),
                key,
            });
        }
        Press::Refuse(why) => {
            state.capturing.replace(None);
            row.key.set_label(&row.was);
            trouble.style_context().add_class("error");
            trouble.set_text(&why);
        }
    }
    // Swallowed either way: a key that reached the button during capture must
    // not also be a key the window acts on.
    glib::Propagation::Stop
}

/// The checks that need real GTK widgets, run from `main`'s single init.
#[cfg(test)]
pub mod on_a_display {
    use super::*;

    type Asked = Rc<RefCell<Vec<Ask>>>;

    fn watched() -> (Tab, Asked) {
        let asked: Asked = Rc::new(RefCell::new(Vec::new()));
        let tab = Tab::new(true);
        let recording = asked.clone();
        tab.wire(Rc::new(move |ask| recording.borrow_mut().push(ask)));
        (tab, asked)
    }

    fn dock(name: &str) -> EnvironmentInfo {
        EnvironmentInfo {
            name: name.to_string(),
            workspaces: Vec::new(),
            current: false,
            pinned: Vec::new(),
            widgets: Vec::new(),
        }
    }

    fn switch(name: &str) -> Action {
        Action::Switch(name.to_string())
    }

    fn nothing() -> HashMap<Action, String> {
        HashMap::new()
    }

    /// Clicking the button is part of pressing a key on it: the capture has to
    /// have been asked for.
    fn capture(tab: &Tab, at: usize) -> Rc<Row> {
        let row = tab.rows.borrow()[at].clone();
        row.key.emit_clicked();
        row
    }

    fn hit(tab: &Tab, row: &Rc<Row>, keyval: u32, modifiers: gdk::ModifierType) {
        pressed(&tab.state, row, &tab.trouble, keyval, modifiers);
    }

    pub fn showing_what_the_daemon_said_asks_for_nothing() {
        let (tab, asked) = watched();

        tab.show(&[dock("Work"), dock("Personal")], &nothing());
        tab.show(&[dock("Only")], &nothing());

        assert!(
            asked.borrow().is_empty(),
            "the window answered its own refresh: {:?}",
            asked.borrow()
        );
    }

    pub fn there_is_a_row_for_the_cycle_and_one_for_each_dock() {
        let tab = Tab::new(true);

        tab.show(&[dock("Work"), dock("Personal")], &nothing());

        let rows = tab.rows.borrow();
        assert_eq!(rows.len(), 3, "the cycle plus two docks");
        assert_eq!(rows[0].action, Action::Cycle);
        assert_eq!(rows[1].action, switch("Work"));
        assert_eq!(rows[2].action, switch("Personal"));
    }

    pub fn a_key_already_bound_is_what_the_button_reads() {
        let tab = Tab::new(true);
        let bound = HashMap::from([(Action::Cycle, "<Super>e".to_string())]);

        tab.show(&[dock("Work")], &bound);

        let rows = tab.rows.borrow();
        assert_eq!(reads(&rows[0].key), "<Super>e");
        assert!(rows[0].clear.is_sensitive(), "a bound key can be cleared");
        assert_eq!(reads(&rows[1].key), "Disabled");
        assert!(!rows[1].clear.is_sensitive(), "nothing to clear");
    }

    fn reads(button: &gtk::Button) -> String {
        button.label().map(|text| text.to_string()).unwrap_or_default()
    }

    /// The press has to carry the action of the row it landed on, not the one
    /// that happens to be first.
    pub fn a_captured_key_names_the_row_it_was_pressed_on() {
        let (tab, asked) = watched();
        tab.show(&[dock("Work"), dock("Personal")], &nothing());

        let row = capture(&tab, 2);
        hit(&tab, &row, *gdk::keys::constants::e, gdk::ModifierType::SUPER_MASK);

        assert_eq!(
            asked.borrow().as_slice(),
            [Ask::Bind {
                action: switch("Personal"),
                key: "<Super>e".to_string(),
            }]
        );
    }

    /// A press on a button nobody asked to listen is a button press, not a
    /// shortcut — otherwise tabbing onto the row would bind whatever came next.
    pub fn a_key_pressed_without_clicking_first_binds_nothing() {
        let (tab, asked) = watched();
        tab.show(&[dock("Work")], &nothing());

        let row = tab.rows.borrow()[0].clone();
        hit(&tab, &row, *gdk::keys::constants::e, gdk::ModifierType::SUPER_MASK);

        assert!(asked.borrow().is_empty(), "{:?}", asked.borrow());
    }

    pub fn a_modifier_on_its_own_keeps_the_button_listening() {
        let (tab, asked) = watched();
        tab.show(&[dock("Work")], &nothing());

        let row = capture(&tab, 0);
        hit(&tab, &row, *gdk::keys::constants::Super_L, gdk::ModifierType::empty());
        hit(&tab, &row, *gdk::keys::constants::e, gdk::ModifierType::SUPER_MASK);

        assert_eq!(
            asked.borrow().as_slice(),
            [Ask::Bind {
                action: Action::Cycle,
                key: "<Super>e".to_string(),
            }],
            "the modifier was taken for the shortcut"
        );
    }

    pub fn escape_leaves_the_key_that_was_there() {
        let (tab, asked) = watched();
        let bound = HashMap::from([(Action::Cycle, "<Super>e".to_string())]);
        tab.show(&[dock("Work")], &bound);

        let row = capture(&tab, 0);
        hit(&tab, &row, *gdk::keys::constants::Escape, gdk::ModifierType::empty());

        assert!(asked.borrow().is_empty(), "esc wrote something");
        assert_eq!(
            reads(&row.key),
            "<Super>e",
            "the button did not go back to what it was showing"
        );
    }

    pub fn backspace_asks_for_the_key_to_go() {
        let (tab, asked) = watched();
        let bound = HashMap::from([(Action::Cycle, "<Super>e".to_string())]);
        tab.show(&[dock("Work")], &bound);

        let row = capture(&tab, 0);
        hit(&tab, &row, *gdk::keys::constants::BackSpace, gdk::ModifierType::empty());

        assert_eq!(asked.borrow().as_slice(), [Ask::Clear(Action::Cycle)]);
    }

    pub fn the_clear_button_asks_for_the_same_thing() {
        let (tab, asked) = watched();
        let bound = HashMap::from([(Action::Cycle, "<Super>e".to_string())]);
        tab.show(&[dock("Work")], &bound);

        tab.rows.borrow()[0].clear.emit_clicked();

        assert_eq!(asked.borrow().as_slice(), [Ask::Clear(Action::Cycle)]);
    }

    /// A letter with no modifier would be bound to the letter: press E and the
    /// dock switches, for ever, including while typing into a password field.
    pub fn a_key_with_no_modifier_is_refused_and_said() {
        let (tab, asked) = watched();
        tab.show(&[dock("Work")], &nothing());

        let row = capture(&tab, 0);
        hit(&tab, &row, *gdk::keys::constants::e, gdk::ModifierType::empty());

        assert!(asked.borrow().is_empty(), "a bare letter was bound");
        assert!(
            tab.trouble.text().contains("Super"),
            "no reason given: {:?}",
            tab.trouble.text()
        );
        assert_eq!(reads(&row.key), "Disabled", "the button kept the prompt");
    }

    /// Shift is not a modifier for this purpose: `<Shift>e` is E.
    pub fn shift_alone_is_not_enough_of_a_modifier() {
        let (tab, asked) = watched();
        tab.show(&[dock("Work")], &nothing());

        let row = capture(&tab, 0);
        hit(&tab, &row, *gdk::keys::constants::E, gdk::ModifierType::SHIFT_MASK);

        assert!(asked.borrow().is_empty(), "shift and a letter was bound");
    }

    pub fn shift_with_a_real_modifier_is_fine() {
        let (tab, asked) = watched();
        tab.show(&[dock("Work")], &nothing());

        let row = capture(&tab, 0);
        hit(
            &tab,
            &row,
            *gdk::keys::constants::e,
            gdk::ModifierType::SUPER_MASK | gdk::ModifierType::SHIFT_MASK,
        );

        assert_eq!(
            asked.borrow().as_slice(),
            [Ask::Bind {
                action: Action::Cycle,
                key: "<Shift><Super>e".to_string(),
            }]
        );
    }

    /// Num Lock happening to be on must not become part of the shortcut, or
    /// the key would stop working the moment it is turned off.
    pub fn a_lock_that_happens_to_be_on_is_not_part_of_the_shortcut() {
        let (tab, asked) = watched();
        tab.show(&[dock("Work")], &nothing());

        let row = capture(&tab, 0);
        hit(
            &tab,
            &row,
            *gdk::keys::constants::e,
            gdk::ModifierType::SUPER_MASK | gdk::ModifierType::MOD2_MASK,
        );

        assert_eq!(
            asked.borrow().as_slice(),
            [Ask::Bind {
                action: Action::Cycle,
                key: "<Super>e".to_string(),
            }]
        );
    }

    /// Two docks whose names slug the same would share one slot. The row says
    /// so instead of binding a key that moves the other dock's.
    pub fn a_dock_that_cannot_own_a_slot_cannot_be_captured() {
        let (tab, asked) = watched();
        tab.show(&[dock("My Work"), dock("My-Work")], &nothing());

        {
            let rows = tab.rows.borrow();
            for row in [&rows[1], &rows[2]] {
                assert!(!row.key.is_sensitive(), "{:?} was offered a key", row.action);
                assert!(
                    row.key.tooltip_text().is_some_and(|why| why.contains("rename")),
                    "no reason on the row: {:?}",
                    row.key.tooltip_text()
                );
            }
            assert!(rows[0].key.is_sensitive(), "the cycle is always bindable");
        }

        assert!(asked.borrow().is_empty());
    }

    /// The window's own refusal names the action, because the window owns both
    /// sides of it and clearing one is the fix.
    pub fn a_key_one_of_our_own_actions_holds_is_named_as_ours() {
        let about = said(&Clash::Ours(switch("Work")), "<Super>e");

        assert!(about.contains("Doca: Work"), "{about}");
        assert!(about.contains("clear"), "{about}");
    }

    /// Someone else's key is not taken away. It is said.
    pub fn a_key_another_application_holds_is_bound_but_never_in_silence() {
        let about = said(&Clash::Theirs("Open terminal".to_string()), "<Super>e");

        assert!(about.contains("Open terminal"), "{about}");
        assert!(about.contains("Both are set"), "{about}");
    }

    /// A desktop with no such schemas draws no capture buttons at all.
    pub fn a_desktop_without_gnome_s_shortcuts_says_so_instead() {
        let tab = Tab::new(false);

        tab.show(&[dock("Work")], &nothing());

        assert!(
            tab.rows.borrow().is_empty(),
            "capture buttons on a desktop with nowhere to write"
        );
    }
}
