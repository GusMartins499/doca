//! The Docks tab: which docks exist, what each pins, and where each appears.
//!
//! This is the part of the config a terminal was the only way to reach.
//! Pinning from the bar works on the dock you are looking at — which is what a
//! click on the bar means — so giving a dock you are *not* standing in a
//! pinned app meant opening the TOML.
//!
//! Nothing here writes to disk or holds the bus: a control emits an [`Action`]
//! naming the dock it means, and whoever wired the tab sends it. That is what
//! lets a test assert that Remove removes the dock the user selected rather
//! than the one that happens to be on screen.

use std::cell::RefCell;
use std::rc::Rc;

use doca_ipc::{Application, EnvironmentInfo};
use gtk::prelude::*;

use crate::chrome::{hint, scrolling, section, titled, Filling};

/// One change to the docks, as a control asks for it.
///
/// An enum rather than a closure per control because every one of these names
/// a dock, and naming the wrong one is the bug worth designing against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Add(String),
    Remove(String),
    Rename { from: String, to: String },
    Workspaces { name: String, workspaces: Vec<i32> },
    Pin { name: String, id: String },
    Unpin { name: String, id: String },
    Reorder { name: String, order: Vec<String> },
    /// The docks themselves, in the order the cycle should walk them.
    ReorderDocks(Vec<String>),
    Widgets { name: String, widgets: Vec<String> },
}

pub type Act = Rc<dyn Fn(Action)>;

/// The workspaces a dock claims, as a person types them.
///
/// Counting from zero, because that is what the config file holds and what the
/// daemon stores: a window showing 1 for the workspace the file calls 0 would
/// make every hand-edited config read wrong.
pub fn show_workspaces(workspaces: &[i32]) -> String {
    workspaces
        .iter()
        .map(|index| index.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// What was typed, or why it cannot be used.
///
/// The daemon would quietly drop a negative and sort the rest; saying so here
/// instead means the field never silently disagrees with what it then shows.
pub fn parse_workspaces(typed: &str) -> Result<Vec<i32>, String> {
    let mut claimed: Vec<i32> = Vec::new();
    for piece in typed.split(|c: char| c == ',' || c.is_whitespace()) {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        let index: i32 = piece
            .parse()
            .map_err(|_| format!("{piece:?} is not a workspace number"))?;
        if index < 0 {
            return Err("there is no workspace below 0".to_string());
        }
        if !claimed.contains(&index) {
            claimed.push(index);
        }
    }
    claimed.sort_unstable();
    Ok(claimed)
}

/// Whether an app answers to what is being typed in the search box.
///
/// The id as well as the name, because the id is what the config holds, and
/// someone who has read their own TOML will type that.
pub fn matches(query: &str, app: &Application) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    app.name.to_lowercase().contains(&query) || app.id.to_lowercase().contains(&query)
}

/// One pin moved a step, or `None` when it is already at the end it is moving
/// towards — so a button at the edge does nothing rather than sending a
/// reorder that reorders nothing.
pub fn moved(order: &[String], index: usize, delta: isize) -> Option<Vec<String>> {
    let target = index.checked_add_signed(delta)?;
    if index >= order.len() || target >= order.len() {
        return None;
    }
    let mut moved = order.to_vec();
    moved.swap(index, target);
    Some(moved)
}

/// A dock's widgets after one of them was ticked or unticked.
///
/// The order the dock had is kept and a newly ticked widget goes at the end:
/// ticking a box says nothing about where the widget should sit, and
/// reshuffling the row over one tick would be the window inventing an order.
pub fn widgets_after(current: &[String], id: &str, on: bool) -> Vec<String> {
    let held = current.iter().any(|widget| widget == id);
    if on == held {
        // Nothing to say. Removing and re-appending would be the same *set*
        // and a different *row*: a widget the dock already shows would jump to
        // the end because its box was ticked a second time.
        return current.to_vec();
    }
    let mut wanted: Vec<String> = current.iter().filter(|widget| *widget != id).cloned().collect();
    if on {
        wanted.push(id.to_string());
    }
    wanted
}

/// The dock to show after the list changed under the selection.
///
/// To the list a rename is a removal and an addition, so the selected name may
/// be gone; falling back to the first keeps the pane from blanking after one.
pub fn selected_after<'a>(docks: &'a [EnvironmentInfo], was: Option<&str>) -> Option<&'a str> {
    was.and_then(|name| docks.iter().find(|dock| dock.name == name))
        .or_else(|| docks.first())
        .map(|dock| dock.name.as_str())
}

/// What the tab knows, shared with every handler it connects.
#[derive(Default)]
struct State {
    /// What the daemon last said, in the order the rows are in.
    known: RefCell<Vec<EnvironmentInfo>>,
    apps: RefCell<Vec<Application>>,
    selected: RefCell<Option<String>>,
    /// The same guard the Appearance tab carries, but a second line of
    /// defence here rather than the first. What actually keeps a refresh from
    /// writing back is that nothing in this tab writes on a value changing: a
    /// rename and a workspace list go out on `activate`, and a ticked box is
    /// compared against the dock the box was just filled from, so a fill
    /// always compares equal. Measured, not assumed — breaking this flag
    /// leaves the "a refresh asks for nothing" check passing, where wiring the
    /// name to `changed` fails it.
    ///
    /// It stays because `row_selected` fires while the list is being torn down
    /// and rebuilt, and a handler reading a row index from a list that is half
    /// gone would point the detail pane — and the next button press — at
    /// another dock.
    filling: Filling,
}

impl State {
    fn selected_dock(&self) -> Option<EnvironmentInfo> {
        let selected = self.selected.borrow().clone()?;
        self.known
            .borrow()
            .iter()
            .find(|dock| dock.name == selected)
            .cloned()
    }
}

pub struct Tab {
    pub root: gtk::Widget,
    docks: gtk::ListBox,
    pins: gtk::ListBox,
    name: gtk::Entry,
    workspaces: gtk::Entry,
    trouble: gtk::Label,
    add: gtk::Button,
    remove: gtk::Button,
    dock_up: gtk::Button,
    dock_down: gtk::Button,
    add_app: gtk::Button,
    unpin: gtk::Button,
    up: gtk::Button,
    down: gtk::Button,
    checks: Vec<(&'static str, gtk::CheckButton)>,
    state: Rc<State>,
}

impl Tab {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        root.set_margin(14);

        let left = gtk::Box::new(gtk::Orientation::Vertical, 6);
        left.set_size_request(140, -1);
        let docks = gtk::ListBox::new();
        docks.set_selection_mode(gtk::SelectionMode::Single);
        left.pack_start(&scrolling(&docks), true, true, 0);

        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let add = gtk::Button::with_label("Add");
        let remove = gtk::Button::with_label("Remove");
        buttons.pack_start(&add, true, true, 0);
        buttons.pack_start(&remove, true, true, 0);
        left.pack_start(&buttons, false, false, 0);

        let order = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let dock_up = gtk::Button::with_label("Up");
        let dock_down = gtk::Button::with_label("Down");
        order.pack_start(&dock_up, true, true, 0);
        order.pack_start(&dock_down, true, true, 0);
        left.pack_start(&order, false, false, 0);
        // The one thing about this list that is not obvious from looking at
        // it, and the reason the buttons are here at all: until now the cycle
        // order was the one piece of the config only a text editor could
        // reach.
        left.pack_start(
            &hint("This order is the order a key cycles through them."),
            false,
            false,
            0,
        );
        root.pack_start(&left, false, false, 0);

        let right = gtk::Box::new(gtk::Orientation::Vertical, 8);
        right.set_hexpand(true);

        let name = gtk::Entry::new();
        name.set_placeholder_text(Some("the dock's name"));
        right.pack_start(&titled("Name", &name), false, false, 0);

        let workspaces = gtk::Entry::new();
        workspaces.set_placeholder_text(Some("every workspace nothing else claims"));
        right.pack_start(&titled("Workspaces", &workspaces), false, false, 0);
        right.pack_start(
            &hint(
                "Numbers separated by commas, counting from 0 as the config file does. \
                 Left empty, this dock covers whatever no other dock asked for.",
            ),
            false,
            false,
            0,
        );

        let pins = gtk::ListBox::new();
        pins.set_selection_mode(gtk::SelectionMode::Single);
        right.pack_start(&section("Pinned apps"), false, false, 0);
        right.pack_start(&scrolling(&pins), true, true, 0);

        let pin_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let add_app = gtk::Button::with_label("Add app…");
        let unpin = gtk::Button::with_label("Remove");
        let up = gtk::Button::with_label("Up");
        let down = gtk::Button::with_label("Down");
        for button in [&add_app, &unpin, &up, &down] {
            pin_buttons.pack_start(button, false, false, 0);
        }
        right.pack_start(&pin_buttons, false, false, 0);

        right.pack_start(&section("Widgets"), false, false, 0);
        let grid = gtk::Grid::new();
        grid.set_column_spacing(10);
        let mut checks = Vec::new();
        for (at, id) in doca_ipc::WIDGETS.iter().enumerate() {
            let check = gtk::CheckButton::with_label(id);
            grid.attach(&check, (at % 4) as i32, (at / 4) as i32, 1, 1);
            checks.push((*id, check));
        }
        right.pack_start(&grid, false, false, 0);

        // Where a refusal lands. The daemon tells a caller why — "a dock
        // called Work already exists" — and that sentence belongs next to the
        // field the user typed it in, not in a log nobody is reading.
        let trouble = gtk::Label::new(None);
        trouble.set_widget_name("hint");
        trouble.set_xalign(0.0);
        trouble.set_line_wrap(true);
        trouble.style_context().add_class("error");
        right.pack_start(&trouble, false, false, 0);

        root.pack_start(&right, true, true, 0);

        Self {
            root: root.upcast(),
            docks,
            pins,
            name,
            workspaces,
            trouble,
            add,
            remove,
            dock_up,
            dock_down,
            add_app,
            unpin,
            up,
            down,
            checks,
            state: Rc::new(State::default()),
        }
    }

    /// Say why the last thing the user asked for did not happen.
    pub fn complain(&self, why: &str) {
        self.trouble.set_text(why);
    }

    fn clear_complaint(&self) {
        self.trouble.set_text("");
    }

    /// Connect the controls to whatever is listening.
    pub fn wire(&self, act: Act) {
        let state = self.state.clone();
        let name = self.name.clone();
        let workspaces = self.workspaces.clone();
        let pins = self.pins.clone();
        let checks = self.checks.clone();
        let docks = self.docks.clone();
        // Selecting a dock only moves the pane; nothing is written.
        self.docks.connect_row_selected(move |_, row| {
            if state.filling.is_filling() {
                return;
            }
            let Some(row) = row else { return };
            let picked = state
                .known
                .borrow()
                .get(row.index() as usize)
                .map(|dock| dock.name.clone());
            if let Some(picked) = picked {
                state.selected.replace(Some(picked));
                fill_detail(&state, &name, &workspaces, &pins, &checks);
            }
        });
        let _ = docks;

        let state = self.state.clone();
        let sending = act.clone();
        self.add.connect_clicked(move |_| {
            // A name has to be unique and the daemon is the one that knows
            // which are taken, so the window asks for a free one rather than
            // offering "New dock" and being refused.
            let taken: Vec<String> = state
                .known
                .borrow()
                .iter()
                .map(|dock| dock.name.clone())
                .collect();
            sending(Action::Add(free_name(&taken)));
        });

        let state = self.state.clone();
        let sending = act.clone();
        self.remove.connect_clicked(move |_| {
            if let Some(dock) = state.selected_dock() {
                sending(Action::Remove(dock.name));
            }
        });

        let state = self.state.clone();
        let sending = act.clone();
        // On activate, not on every keystroke: a rename sent letter by letter
        // would ask the daemon to create "W", "Wo", "Wor" and would move the
        // name out from under the user's own cursor.
        self.name.connect_activate(move |entry| {
            if state.filling.is_filling() {
                return;
            }
            let Some(dock) = state.selected_dock() else { return };
            let typed = entry.text().trim().to_string();
            if typed.is_empty() || typed == dock.name {
                return;
            }
            state.selected.replace(Some(typed.clone()));
            sending(Action::Rename {
                from: dock.name,
                to: typed,
            });
        });

        let state = self.state.clone();
        let sending = act.clone();
        let complaining = self.trouble.clone();
        self.workspaces.connect_activate(move |entry| {
            if state.filling.is_filling() {
                return;
            }
            let Some(dock) = state.selected_dock() else { return };
            match parse_workspaces(&entry.text()) {
                Ok(workspaces) => {
                    complaining.set_text("");
                    sending(Action::Workspaces {
                        name: dock.name,
                        workspaces,
                    });
                }
                Err(why) => complaining.set_text(&why),
            }
        });

        let state = self.state.clone();
        let sending = act.clone();
        let pins = self.pins.clone();
        self.unpin.connect_clicked(move |_| {
            let Some(dock) = state.selected_dock() else { return };
            let Some(row) = pins.selected_row() else { return };
            if let Some(id) = dock.pinned.get(row.index() as usize) {
                sending(Action::Unpin {
                    name: dock.name,
                    id: id.clone(),
                });
            }
        });

        for (button, delta) in [(&self.dock_up, -1isize), (&self.dock_down, 1isize)] {
            let state = self.state.clone();
            let sending = act.clone();
            let docks = self.docks.clone();
            button.connect_clicked(move |_| {
                let Some(row) = docks.selected_row() else { return };
                let at = row.index() as usize;
                let order: Vec<String> = state
                    .known
                    .borrow()
                    .iter()
                    .map(|dock| dock.name.clone())
                    .collect();
                // No re-selecting the row that lands: the refresh that follows
                // picks the selection back up by name, so the dock the user
                // was looking at stays the dock they are looking at.
                if let Some(order) = moved(&order, at, delta) {
                    sending(Action::ReorderDocks(order));
                }
            });
        }

        for (button, delta) in [(&self.up, -1isize), (&self.down, 1isize)] {
            let state = self.state.clone();
            let sending = act.clone();
            let pins = self.pins.clone();
            button.connect_clicked(move |_| {
                let Some(dock) = state.selected_dock() else { return };
                let Some(row) = pins.selected_row() else { return };
                let at = row.index() as usize;
                if let Some(order) = moved(&dock.pinned, at, delta) {
                    // The row moves with the pin, or the next press would move
                    // whatever landed under the selection instead.
                    let landed = at.saturating_add_signed(delta);
                    sending(Action::Reorder {
                        name: dock.name,
                        order,
                    });
                    if let Some(row) = pins.row_at_index(landed as i32) {
                        pins.select_row(Some(&row));
                    }
                }
            });
        }

        for (id, check) in &self.checks {
            let state = self.state.clone();
            let sending = act.clone();
            let id = *id;
            check.connect_toggled(move |check| {
                if state.filling.is_filling() {
                    return;
                }
                let Some(dock) = state.selected_dock() else { return };
                let widgets = widgets_after(&dock.widgets, id, check.is_active());
                if widgets == dock.widgets {
                    // A tick that changes nothing is not worth a save and a
                    // signal that makes every reader rebuild.
                    return;
                }
                sending(Action::Widgets {
                    name: dock.name,
                    widgets,
                });
            });
        }

        let state = self.state.clone();
        let sending = act.clone();
        let root = self.root.clone();
        self.add_app.connect_clicked(move |_| {
            let Some(dock) = state.selected_dock() else { return };
            let sending = sending.clone();
            pick_app(&root, &state.apps.borrow(), move |id| {
                sending(Action::Pin {
                    name: dock.name.clone(),
                    id,
                })
            });
        });
    }

    /// The apps the picker can offer. Read once: the set of installed
    /// applications does not change while a settings window is open, and
    /// re-reading it on every `ConfigChanged` would put a few hundred entries
    /// on the bus because someone moved a slider.
    pub fn offer(&self, apps: Vec<Application>) {
        self.state.apps.replace(apps);
    }

    /// Show the docks the daemon reports, without writing any of it back.
    pub fn show(&self, docks: &[EnvironmentInfo]) {
        let was = self.state.selected.borrow().clone();
        let selected = selected_after(docks, was.as_deref()).map(str::to_string);
        self.state.known.replace(docks.to_vec());
        self.state.selected.replace(selected.clone());

        self.state.filling.while_filling(|| {
            for row in self.docks.children() {
                self.docks.remove(&row);
            }
            for dock in docks {
                let label = gtk::Label::new(Some(&dock.name));
                label.set_xalign(0.0);
                label.set_margin(6);
                if dock.current {
                    // The dock on screen, so a rename or a removal is not a
                    // surprise about which one the user was looking at.
                    label.set_markup(&format!(
                        "<b>{}</b>",
                        glib::markup_escape_text(&dock.name)
                    ));
                }
                let row = gtk::ListBoxRow::new();
                row.add(&label);
                self.docks.add(&row);
            }
            self.docks.show_all();
            if let Some(at) = docks.iter().position(|dock| Some(&dock.name) == selected.as_ref()) {
                if let Some(row) = self.docks.row_at_index(at as i32) {
                    self.docks.select_row(Some(&row));
                }
            }
            // The last dock cannot go — the daemon refuses, and a button that
            // is always refused is better greyed out than argued with.
            self.remove.set_sensitive(docks.len() > 1);
        });

        fill_detail(
            &self.state,
            &self.name,
            &self.workspaces,
            &self.pins,
            &self.checks,
        );
        self.clear_complaint();
    }
}

/// Fill the right-hand pane from the selected dock.
fn fill_detail(
    state: &Rc<State>,
    name: &gtk::Entry,
    workspaces: &gtk::Entry,
    pins: &gtk::ListBox,
    checks: &[(&'static str, gtk::CheckButton)],
) {
    let dock = state.selected_dock();
    state.filling.while_filling(|| {
        name.set_text(dock.as_ref().map(|d| d.name.as_str()).unwrap_or(""));
        workspaces.set_text(
            &dock
                .as_ref()
                .map(|d| show_workspaces(&d.workspaces))
                .unwrap_or_default(),
        );

        for row in pins.children() {
            pins.remove(&row);
        }
        if let Some(dock) = &dock {
            for id in &dock.pinned {
                let label = gtk::Label::new(Some(id));
                label.set_xalign(0.0);
                label.set_margin(4);
                let row = gtk::ListBoxRow::new();
                row.add(&label);
                pins.add(&row);
            }
        }
        pins.show_all();

        for (id, check) in checks {
            let on = dock
                .as_ref()
                .is_some_and(|dock| dock.widgets.iter().any(|held| held == id));
            check.set_active(on);
        }
    });
}

/// A name no dock is using yet.
fn free_name(taken: &[String]) -> String {
    let base = "New dock";
    if !taken.iter().any(|name| name == base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or_else(|| base.to_string())
}

/// Ask which app to pin, with a box to type in.
///
/// A dialog rather than a combo box in the tab: there are a few hundred
/// installed applications on an ordinary desktop, and the only way to find one
/// in a list that long is to type at it.
fn pick_app(near: &gtk::Widget, apps: &[Application], chosen: impl Fn(String) + 'static) {
    let parent = near
        .toplevel()
        .and_then(|top| top.downcast::<gtk::Window>().ok());
    let dialog = gtk::Dialog::with_buttons(
        Some("Pin an application"),
        parent.as_ref(),
        gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
        &[
            ("Cancel", gtk::ResponseType::Cancel),
            ("Pin", gtk::ResponseType::Accept),
        ],
    );
    dialog.set_default_size(340, 380);

    let body = dialog.content_area();
    body.set_spacing(6);
    body.set_margin(10);
    let search = gtk::SearchEntry::new();
    body.pack_start(&search, false, false, 0);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    body.pack_start(&scrolling(&list), true, true, 0);

    let shown: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let fill = {
        let list = list.clone();
        let apps = apps.to_vec();
        let shown = shown.clone();
        move |query: &str| {
            for row in list.children() {
                list.remove(&row);
            }
            let mut ids = Vec::new();
            for app in apps.iter().filter(|app| matches(query, app)) {
                let label = gtk::Label::new(None);
                label.set_xalign(0.0);
                label.set_margin(4);
                label.set_markup(&format!(
                    "{}  <small>{}</small>",
                    glib::markup_escape_text(&app.name),
                    glib::markup_escape_text(&app.id)
                ));
                let row = gtk::ListBoxRow::new();
                row.add(&label);
                list.add(&row);
                ids.push(app.id.clone());
            }
            shown.replace(ids);
            list.show_all();
        }
    };
    fill("");
    let searching = fill.clone();
    search.connect_search_changed(move |entry| searching(&entry.text()));

    // Double-clicking a name is pinning it: nobody looks for the Pin button
    // after they have already picked the thing.
    let accepting = dialog.clone();
    list.connect_row_activated(move |_, _| accepting.response(gtk::ResponseType::Accept));

    dialog.connect_response(move |dialog, answer| {
        if answer == gtk::ResponseType::Accept {
            let picked = list
                .selected_row()
                .and_then(|row| shown.borrow().get(row.index() as usize).cloned());
            if let Some(id) = picked {
                chosen(id);
            }
        }
        unsafe {
            dialog.destroy();
        }
    });
    dialog.show_all();
}

/// The checks that need real GTK widgets, run from `main`'s single init.
#[cfg(test)]
pub mod on_a_display {
    use super::*;

    type Asked = Rc<RefCell<Vec<Action>>>;

    fn watched() -> (Tab, Asked) {
        let asked: Asked = Rc::new(RefCell::new(Vec::new()));
        let tab = Tab::new();
        let recording = asked.clone();
        tab.wire(Rc::new(move |action| recording.borrow_mut().push(action)));
        (tab, asked)
    }

    fn dock(name: &str, workspaces: &[i32], pinned: &[&str], widgets: &[&str]) -> EnvironmentInfo {
        EnvironmentInfo {
            name: name.to_string(),
            workspaces: workspaces.to_vec(),
            current: false,
            pinned: pinned.iter().map(|id| id.to_string()).collect(),
            widgets: widgets.iter().map(|id| id.to_string()).collect(),
        }
    }

    fn two() -> Vec<EnvironmentInfo> {
        vec![
            dock("Work", &[0, 1], &["code", "firefox"], &["clock"]),
            dock("Personal", &[2], &["spotify"], &[]),
        ]
    }

    /// The one that matters: filling the controls must not write anything.
    ///
    /// Showing the docks fills a list (which fires `row_selected`), two entries
    /// (`changed`) and twelve check buttons (`toggled`). Without the guard,
    /// opening the window would rename a dock to its own name and rewrite every
    /// widget list — and with two writers, for ever.
    pub fn showing_what_the_daemon_said_asks_for_nothing() {
        let (tab, asked) = watched();

        tab.show(&two());
        tab.show(&[dock("Only", &[], &[], &["cpu", "clock"])]);

        assert!(
            asked.borrow().is_empty(),
            "the window answered its own refresh: {:?}",
            asked.borrow()
        );
    }

    pub fn selecting_a_dock_shows_what_that_dock_holds() {
        let (tab, asked) = watched();
        tab.show(&two());

        let row = tab.docks.row_at_index(1).expect("a row per dock");
        tab.docks.select_row(Some(&row));

        assert_eq!(tab.name.text(), "Personal");
        assert_eq!(tab.workspaces.text(), "2");
        assert_eq!(tab.pins.children().len(), 1, "Personal pins one app");
        for (id, check) in &tab.checks {
            assert!(!check.is_active(), "{id} is not one of Personal's widgets");
        }
        assert!(
            asked.borrow().is_empty(),
            "looking at a dock changed it: {:?}",
            asked.borrow()
        );
    }

    /// Every write has to name the dock the user is editing. The dock on
    /// screen is a different question, and the one these controls must not
    /// quietly fall back to.
    pub fn a_change_names_the_dock_that_is_selected() {
        let (tab, asked) = watched();
        let mut docks = two();
        docks[0].current = true; // Work is on screen; Personal is being edited.
        tab.show(&docks);
        let row = tab.docks.row_at_index(1).unwrap();
        tab.docks.select_row(Some(&row));
        asked.borrow_mut().clear();

        tab.workspaces.set_text("3, 3, 1");
        tab.workspaces.emit_activate();

        let asked = asked.borrow();
        assert_eq!(
            asked.as_slice(),
            [Action::Workspaces {
                name: "Personal".to_string(),
                workspaces: vec![1, 3],
            }]
        );
    }

    pub fn ticking_a_widget_keeps_the_ones_the_dock_already_had() {
        let (tab, asked) = watched();
        tab.show(&[dock("Work", &[], &[], &["clock", "cpu"])]);
        asked.borrow_mut().clear();

        let (_, water) = tab
            .checks
            .iter()
            .find(|(id, _)| *id == "water")
            .expect("water is on offer");
        water.set_active(true);

        let asked = asked.borrow();
        assert_eq!(
            asked.as_slice(),
            [Action::Widgets {
                name: "Work".to_string(),
                widgets: vec!["clock".to_string(), "cpu".to_string(), "water".to_string()],
            }],
            "a tick must not reshuffle the row"
        );
    }

    pub fn unpinning_names_the_app_the_row_points_at() {
        let (tab, asked) = watched();
        tab.show(&two());
        let row = tab.pins.row_at_index(1).expect("Work pins two apps");
        tab.pins.select_row(Some(&row));
        asked.borrow_mut().clear();

        tab.unpin.emit_clicked();

        let asked = asked.borrow();
        assert_eq!(
            asked.as_slice(),
            [Action::Unpin {
                name: "Work".to_string(),
                id: "firefox".to_string(),
            }]
        );
    }

    /// The cycle order, which until this slice only a text editor could set.
    pub fn moving_a_dock_sends_the_whole_new_order() {
        let (tab, asked) = watched();
        tab.show(&[dock("Work", &[], &[], &[]), dock("Personal", &[], &[], &[])]);
        let row = tab.docks.row_at_index(1).expect("a row per dock");
        tab.docks.select_row(Some(&row));
        asked.borrow_mut().clear();

        tab.dock_up.emit_clicked();

        assert_eq!(
            asked.borrow().as_slice(),
            [Action::ReorderDocks(vec![
                "Personal".to_string(),
                "Work".to_string(),
            ])],
            "a reorder has to carry every dock, or the daemon would read it as a drop"
        );
    }

    /// A button at the edge does nothing rather than sending an order that
    /// reorders nothing — the same rule the pins follow.
    pub fn a_dock_at_the_top_cannot_be_moved_off_the_list() {
        let (tab, asked) = watched();
        tab.show(&two());
        let row = tab.docks.row_at_index(0).expect("a row per dock");
        tab.docks.select_row(Some(&row));
        asked.borrow_mut().clear();

        tab.dock_up.emit_clicked();

        assert!(asked.borrow().is_empty(), "{:?}", asked.borrow());
    }

    pub fn a_pin_at_the_top_cannot_be_moved_off_the_list() {
        let (tab, asked) = watched();
        tab.show(&two());
        let row = tab.pins.row_at_index(0).unwrap();
        tab.pins.select_row(Some(&row));
        asked.borrow_mut().clear();

        tab.up.emit_clicked();

        assert!(
            asked.borrow().is_empty(),
            "the first pin moved up to nowhere: {:?}",
            asked.borrow()
        );
    }

    pub fn moving_a_pin_down_sends_the_whole_new_order() {
        let (tab, asked) = watched();
        tab.show(&two());
        let row = tab.pins.row_at_index(0).unwrap();
        tab.pins.select_row(Some(&row));
        asked.borrow_mut().clear();

        tab.down.emit_clicked();

        let asked = asked.borrow();
        assert_eq!(
            asked.as_slice(),
            [Action::Reorder {
                name: "Work".to_string(),
                order: vec!["firefox".to_string(), "code".to_string()],
            }]
        );
    }

    pub fn a_new_dock_is_asked_for_by_a_name_nothing_is_using() {
        let (tab, asked) = watched();
        tab.show(&[dock("New dock", &[], &[], &[])]);
        asked.borrow_mut().clear();

        tab.add.emit_clicked();

        let asked = asked.borrow();
        assert_eq!(asked.as_slice(), [Action::Add("New dock 2".to_string())]);
    }

    /// The daemon refuses to remove the last dock, so the button says so
    /// before the user presses it.
    pub fn the_last_dock_cannot_be_asked_to_go() {
        let (tab, _) = watched();

        tab.show(&[dock("Only", &[], &[], &[])]);
        assert!(!tab.remove.is_sensitive());

        tab.show(&two());
        assert!(tab.remove.is_sensitive());
    }

    pub fn a_workspace_that_is_not_a_number_is_said_rather_than_sent() {
        let (tab, asked) = watched();
        tab.show(&two());
        asked.borrow_mut().clear();

        tab.workspaces.set_text("the second one");
        tab.workspaces.emit_activate();

        assert!(
            asked.borrow().is_empty(),
            "nonsense went to the daemon: {:?}",
            asked.borrow()
        );
        assert!(
            tab.trouble.text().contains("workspace"),
            "the window said nothing: {:?}",
            tab.trouble.text()
        );
    }

    pub fn renaming_waits_for_the_name_to_be_finished() {
        let (tab, asked) = watched();
        tab.show(&two());
        asked.borrow_mut().clear();

        // Typing, as far as the entry is concerned.
        tab.name.set_text("Wor");
        tab.name.set_text("Workshop");
        assert!(
            asked.borrow().is_empty(),
            "a rename went out mid-word: {:?}",
            asked.borrow()
        );

        tab.name.emit_activate();

        let asked = asked.borrow();
        assert_eq!(
            asked.as_slice(),
            [Action::Rename {
                from: "Work".to_string(),
                to: "Workshop".to_string(),
            }]
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, name: &str) -> Application {
        Application {
            id: id.to_string(),
            name: name.to_string(),
            icon: String::new(),
        }
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

    fn ids(order: &[&str]) -> Vec<String> {
        order.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn a_dock_claiming_nothing_shows_an_empty_field() {
        assert_eq!(show_workspaces(&[]), "");
        assert_eq!(parse_workspaces(""), Ok(Vec::new()));
    }

    #[test]
    fn what_the_field_shows_is_what_the_field_accepts() {
        let claimed = vec![0, 2, 3];

        assert_eq!(parse_workspaces(&show_workspaces(&claimed)), Ok(claimed));
    }

    #[test]
    fn workspaces_are_tidied_the_same_way_the_daemon_would_tidy_them() {
        assert_eq!(parse_workspaces("3, 1, 1, 0"), Ok(vec![0, 1, 3]));
        assert_eq!(parse_workspaces("1 2"), Ok(vec![1, 2]), "spaces will do too");
    }

    #[test]
    fn a_workspace_that_is_not_a_number_is_refused_with_the_word_in_it() {
        let refused = parse_workspaces("1, two").unwrap_err();

        assert!(refused.contains("two"), "unhelpful: {refused}");
    }

    #[test]
    fn there_is_no_workspace_below_zero() {
        assert!(parse_workspaces("-1").is_err());
    }

    #[test]
    fn searching_finds_an_app_by_name_or_by_the_id_the_config_holds() {
        let firefox = app("firefox-esr", "Firefox");

        assert!(matches("fire", &firefox));
        assert!(matches("FIRE", &firefox), "case is not what anyone means");
        assert!(matches("esr", &firefox), "the id is what the TOML shows");
        assert!(!matches("chrome", &firefox));
        assert!(matches("  ", &firefox), "an empty box hides nothing");
    }

    #[test]
    fn a_pin_moves_one_place_and_takes_nothing_else_with_it() {
        let order = ids(&["a", "b", "c"]);

        assert_eq!(moved(&order, 1, 1), Some(ids(&["a", "c", "b"])));
        assert_eq!(moved(&order, 1, -1), Some(ids(&["b", "a", "c"])));
    }

    #[test]
    fn a_pin_at_either_end_has_nowhere_to_go() {
        let order = ids(&["a", "b"]);

        assert_eq!(moved(&order, 0, -1), None);
        assert_eq!(moved(&order, 1, 1), None);
        assert_eq!(moved(&order, 7, 1), None, "a row that is not there");
    }

    #[test]
    fn every_order_a_move_can_produce_holds_the_same_pins() {
        let order = ids(&["a", "b", "c"]);

        for at in 0..order.len() {
            for delta in [-1isize, 1] {
                if let Some(after) = moved(&order, at, delta) {
                    let mut sorted = after.clone();
                    sorted.sort();
                    assert_eq!(sorted, ids(&["a", "b", "c"]), "a move lost or added a pin");
                }
            }
        }
    }

    #[test]
    fn ticking_a_widget_adds_it_at_the_end_and_leaves_the_rest_in_place() {
        let current = ids(&["clock", "cpu"]);

        assert_eq!(
            widgets_after(&current, "water", true),
            ids(&["clock", "cpu", "water"])
        );
    }

    #[test]
    fn unticking_a_widget_takes_only_that_one_out() {
        let current = ids(&["clock", "cpu", "water"]);

        assert_eq!(widgets_after(&current, "cpu", false), ids(&["clock", "water"]));
    }

    #[test]
    fn ticking_a_widget_a_dock_already_shows_changes_nothing() {
        let current = ids(&["clock", "cpu"]);

        assert_eq!(widgets_after(&current, "clock", true), current);
    }

    #[test]
    fn the_selection_survives_a_refresh_that_did_not_touch_it() {
        let docks = vec![dock("Work"), dock("Personal")];

        assert_eq!(selected_after(&docks, Some("Personal")), Some("Personal"));
    }

    #[test]
    fn a_selection_that_was_renamed_away_falls_back_rather_than_blanking() {
        let docks = vec![dock("Workshop"), dock("Personal")];

        assert_eq!(selected_after(&docks, Some("Work")), Some("Workshop"));
    }

    #[test]
    fn nothing_selected_picks_the_first_dock() {
        assert_eq!(selected_after(&[dock("Work")], None), Some("Work"));
        assert_eq!(selected_after(&[], None), None);
    }

    #[test]
    fn a_new_dock_is_named_around_the_ones_that_exist() {
        assert_eq!(free_name(&[]), "New dock");
        assert_eq!(free_name(&ids(&["New dock"])), "New dock 2");
        assert_eq!(free_name(&ids(&["New dock", "New dock 2"])), "New dock 3");
        assert_eq!(free_name(&ids(&["Work"])), "New dock");
    }
}
