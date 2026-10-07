//! The panel a tile opens: the widget's controls, over the tile that asked.
//!
//! # Why this is a `GtkMenu`
//!
//! The choice is between a `GtkMenu`, a `GtkPopover` and a window of our own,
//! and on X11 it is settled by what the bar is. The bar sets
//! `_NET_WM_WINDOW_TYPE_DOCK`, and a dock window is one the window manager is
//! entitled not to give the keyboard to — which rules out a `GtkPopover`,
//! because a popover is a child of its toplevel's surface and gets keys only
//! through that toplevel's focus. A window of our own would have to ask for
//! focus itself, against a manager that may well refuse a dock, and would
//! have to implement dismissal, `Esc` and click-outside from nothing.
//!
//! A menu takes an explicit pointer *and* keyboard grab when it pops up. The
//! grab is what carries keys over a window the manager will not focus, and it
//! brings `Esc`, click-outside-to-dismiss and arrow-key navigation with it.
//! The folder grid already proved all of this over this same bar — it is a
//! menu for the same reason, and it already knows how to be positioned over a
//! tile — so this is the one surface here with evidence behind it rather than
//! an expectation.
//!
//! What it costs: a menu navigates between its items rather than within them,
//! so each control is an item of its own instead of a row of buttons. That is
//! a constraint on the layout, not on the content, and it is the reason every
//! row below is a `GtkMenuItem`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use doca_ipc::{Body, WidgetState};
use gtk::prelude::*;

/// How long a panel is left alone after it goes up, before it may be taken
/// down to be resized.
///
/// The folder grid's figure and the folder grid's reason: a menu put up by a
/// press and taken down and up again before the *release* of that same click
/// is dismissed by that release. Shared rather than copied, so there is one
/// number to change if it is ever wrong.
pub use crate::stack::SETTLED;

/// How faint a row is while it is still only the promise of one.
const WAITING: f64 = 0.3;

/// How many rows a panel nobody has opened yet is drawn with.
///
/// A heading and one control, which is the smallest panel any widget has.
pub const UNKNOWN: usize = 2;

/// What a control does when it is chosen: tell the daemon so.
pub type Invoke = Rc<dyn Fn(&str, &str)>;

/// How the panel is put on screen: the click that opened it, applied again.
///
/// The same closure opens it and reopens it, so a panel that has to be
/// resized cannot come back somewhere other than where it was opened.
pub type Show = Rc<dyn Fn(&gtk::Menu)>;

/// What a panel is made of.
///
/// The chassis knows two kinds of row. A widget's own issue adds the kinds it
/// needs — a week of bars, a list of events — and the panel grows by a drawer
/// here rather than by a second panel somewhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// What the widget is showing now, in words rather than as a drawing.
    Heading { title: String, detail: String },
    /// Something the widget can be told to do.
    Action {
        action: &'static str,
        label: &'static str,
    },
}

/// The widget's name as a person would write it: `time-progress` is
/// `Time progress`.
pub fn name_of(id: &str) -> String {
    let spaced = id.replace('-', " ");
    let mut letters = spaced.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + letters.as_str(),
        None => spaced,
    }
}

/// The panel this widget's state calls for.
///
/// The heading is read off the typed body, which is the point of the body
/// being typed: a water panel says "3 of 8 glasses" because the state says
/// three and eight, not because the daemon sent that sentence.
pub fn rows_for(state: &WidgetState) -> Vec<Row> {
    let heading = match state.body() {
        Ok(Body::Simple(simple)) => Row::Heading {
            title: if simple.label.is_empty() {
                name_of(&state.id)
            } else {
                simple.label.clone()
            },
            detail: simple.detail.clone(),
        },
        Ok(Body::Water(water)) => Row::Heading {
            title: format!("{} of {}", water.glasses, water.goal),
            detail: if water.glasses >= water.goal {
                "done for today".to_string()
            } else {
                "glasses".to_string()
            },
        },
        // A body this build cannot read still opens a panel that names the
        // widget: the controls below come from the id, not from the body, so
        // all of them still work.
        Err(_) => Row::Heading {
            title: name_of(&state.id),
            detail: String::new(),
        },
    };

    let mut rows = vec![heading];
    rows.extend(
        doca_ipc::widget_action::of(&state.id)
            .map(|(action, label)| Row::Action { action, label }),
    );
    rows
}

/// What each widget's panel held the last time it was opened.
///
/// For the reason the folder grid keeps one: a menu takes its window's size
/// when it pops up and never again, so the only way to resize it is to put it
/// down and up again, which blinks. A panel opened at the shape it turns out
/// to have does not move at all — and that is every open after the first.
#[derive(Clone, Default)]
pub struct Remembered(Rc<RefCell<HashMap<String, usize>>>);

impl Remembered {
    pub fn of(&self, id: &str) -> usize {
        self.0.borrow().get(id).copied().unwrap_or(UNKNOWN)
    }

    pub fn note(&self, id: &str, rows: usize) {
        self.0.borrow_mut().insert(id.to_string(), rows);
    }
}

/// A row that holds nothing yet: the shape of one at a fraction of its
/// weight.
///
/// Dimmed rather than styled, for the reason the grid's waiting cells are: a
/// menu wears whatever the desktop's menus wear, so a colour chosen here
/// would be a guess about a background this code never picked.
fn waiting_row() -> gtk::MenuItem {
    // A space and not nothing: an empty label is no lines tall, and the rows
    // would grow by a line when the words arrived.
    let label = gtk::Label::new(Some(" "));
    label.set_xalign(0.0);
    label.set_width_chars(ROW_CHARS);
    label.set_max_width_chars(ROW_CHARS);

    let row = gtk::MenuItem::new();
    row.add(&label);
    row.set_opacity(WAITING);
    // There is nothing to choose yet, and a row that looks pressable but is
    // not is worse than one that plainly is not.
    row.set_sensitive(false);
    row
}

/// How wide every row is, in characters.
///
/// A width rather than a cap, for the reason the grid's cells have one: the
/// panel is then as wide as its shape and not as wide as its longest label,
/// which is what lets it be opened at the size its answer will take before
/// the answer is known.
pub const ROW_CHARS: i32 = 18;

/// The panel, before anyone has said what is in it.
///
/// Opening waits on nothing. The controls are known from the widget's id, but
/// a panel's *contents* are not: a water panel shows the week, a calendar
/// panel shows what is next, and both of those are a question for the daemon
/// whose cost has no ceiling. So this is on screen in the frame the click
/// landed in, and [`fill`] puts the rows into it when they arrive.
pub fn opening(expected: usize) -> gtk::Menu {
    let menu = gtk::Menu::new();
    menu.set_widget_name("panel");
    menu.set_reserve_toggle_size(false);

    for _ in 0..expected.max(1) {
        menu.append(&waiting_row());
    }

    menu.show_all();
    menu
}

/// Put the widget's rows into the panel that is already on screen.
///
/// `expected` is what the panel was opened for. When the answer has the same
/// number of rows — which is every open after the first — the rows are
/// swapped and nothing moves. When it does not, the panel has to be popped
/// down and up again: a menu keeps the window it sized at popup, so rows
/// beyond it are not shown but scrolled to.
pub fn fill(
    menu: &gtk::Menu,
    expected: usize,
    id: &str,
    rows: &[Row],
    invoke: &Invoke,
    show: &Show,
    opened: Instant,
) {
    for row in menu.children() {
        menu.remove(&row);
    }

    for row in rows {
        menu.append(&drawn(id, row, invoke));
    }
    if rows.is_empty() {
        let empty = gtk::MenuItem::with_label("nothing to show");
        empty.set_sensitive(false);
        menu.append(&empty);
    }

    menu.show_all();
    if expected != rows.len().max(1) {
        reopen(menu, show, opened);
    }
}

/// One row, as the thing on screen.
fn drawn(id: &str, row: &Row, invoke: &Invoke) -> gtk::MenuItem {
    match row {
        Row::Heading { title, detail } => {
            let name = heading_label(title);
            name.set_attributes(Some(&weighted(pango::Weight::Bold, 1.0)));

            let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
            column.add(&name);
            if !detail.is_empty() {
                let under = heading_label(detail);
                under.set_attributes(Some(&weighted(pango::Weight::Normal, 0.85)));
                // Dimmed rather than coloured, for the reason the waiting
                // rows are: the panel wears whatever the desktop's menus
                // wear, and a colour chosen here would be a guess about a
                // background this code never picked.
                under.set_opacity(0.6);
                column.add(&under);
            }

            let heading = gtk::MenuItem::new();
            heading.add(&column);
            // Not a control. Leaving it insensitive also keeps the keyboard
            // off it, so the first arrow key lands on the first real one.
            heading.set_sensitive(false);
            heading
        }
        Row::Action { action, label } => {
            let control = gtk::MenuItem::with_label(label);
            let (id, action) = (id.to_string(), action.to_string());
            let invoke = invoke.clone();
            control.connect_activate(move |_| invoke(&id, &action));
            control
        }
    }
}

/// A line of the heading, at the one width every row has.
fn heading_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_widget_name("panel-line");
    label.set_ellipsize(pango::EllipsizeMode::End);
    label.set_width_chars(ROW_CHARS);
    label.set_max_width_chars(ROW_CHARS);
    label
}

/// Weight and size as Pango attributes rather than as CSS.
///
/// The panel is a menu and wears the desktop's menu style; a stylesheet of
/// our own would have to re-declare a background and a foreground to be sure
/// of either, and then be wrong on the next desktop. An attribute changes the
/// weight of whatever colour the menu already uses.
fn weighted(weight: pango::Weight, scale: f64) -> pango::AttrList {
    let attributes = pango::AttrList::new();
    attributes.insert(pango::AttrInt::new_weight(weight));
    if scale != 1.0 {
        attributes.insert(pango::AttrFloat::new_scale(scale));
    }
    attributes
}

/// Take the panel down and put it up again, which is the only way a menu
/// changes the size of its window.
///
/// Not if it is already gone: a panel whose contents took long enough to
/// arrive that the pointer went elsewhere must not be thrown back up.
fn reopen(menu: &gtk::Menu, show: &Show, opened: Instant) {
    if !menu.is_visible() {
        return;
    }
    let menu = menu.clone();
    let show = show.clone();
    let swap = move || {
        if !menu.is_visible() {
            return;
        }
        menu.popdown();
        // Up again on the next turn, not this one: the one just dismissed is
        // still handing back its grab, and a popup into that is swallowed.
        let menu = menu.clone();
        let show = show.clone();
        glib::idle_add_local_once(move || show(&menu));
    };

    let waited = opened.elapsed();
    if waited >= SETTLED {
        swap();
    } else {
        glib::timeout_add_local_once(SETTLED - waited, swap);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::cell::Cell;

    fn clock() -> WidgetState {
        WidgetState::new(
            "clock",
            Body::Simple(doca_ipc::Simple {
                label: "17:21".to_string(),
                detail: "Thu 01/01".to_string(),
                progress: doca_ipc::NO_PROGRESS,
                active: false,
            }),
        )
    }

    fn water(glasses: u32, goal: u32) -> WidgetState {
        WidgetState::new("water", Body::Water(doca_ipc::Water { glasses, goal }))
    }

    fn invokes_nothing() -> Invoke {
        Rc::new(|_: &str, _: &str| {})
    }

    /// Let GTK get on with whatever the last call asked for.
    fn settle() {
        for _ in 0..200 {
            if !gtk::events_pending() {
                break;
            }
            gtk::main_iteration_do(false);
        }
    }

    /// Somewhere for a panel to hang off, standing in for the tile.
    ///
    /// `popup_at_pointer` reads the pointer out of an event and a test has
    /// none to give it, so these pop up against a widget — which is also how
    /// the real code positions a panel over the tile that asked.
    fn against(window: &gtk::Window) -> (Show, Rc<Cell<usize>>) {
        let window = window.clone();
        let times = Rc::new(Cell::new(0));
        let counting = times.clone();
        let show: Show = Rc::new(move |menu: &gtk::Menu| {
            counting.set(counting.get() + 1);
            menu.popup_at_widget(
                &window,
                gdk::Gravity::NorthWest,
                gdk::Gravity::SouthWest,
                None,
            );
        });
        (show, times)
    }

    fn a_window() -> gtk::Window {
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.show_all();
        settle();
        window
    }

    fn window_of(menu: &gtk::Menu) -> (i32, i32) {
        let allocation = menu
            .toplevel()
            .map(|top| top.allocation())
            .expect("a panel that is up has a window");
        (allocation.width(), allocation.height())
    }

    #[test]
    fn a_widget_id_reads_as_a_name_a_person_would_write() {
        assert_eq!(name_of("clock"), "Clock");
        assert_eq!(name_of("time-progress"), "Time progress");
        assert_eq!(name_of(""), "");
    }

    #[test]
    fn a_panel_opens_with_what_the_widget_is_showing_at_the_top() {
        let rows = rows_for(&clock());

        assert_eq!(
            rows[0],
            Row::Heading {
                title: "17:21".to_string(),
                detail: "Thu 01/01".to_string()
            }
        );
    }

    /// The point of a typed body: the sentence is the panel's to write,
    /// because the numbers arrived as numbers.
    #[test]
    fn a_water_panel_says_what_the_count_means() {
        let rows = rows_for(&water(3, 8));

        assert_eq!(
            rows[0],
            Row::Heading {
                title: "3 of 8".to_string(),
                detail: "glasses".to_string()
            }
        );
        assert_eq!(
            rows_for(&water(8, 8))[0],
            Row::Heading {
                title: "8 of 8".to_string(),
                detail: "done for today".to_string()
            }
        );
    }

    /// Every widget's panel has the widget's own controls in it, and a widget
    /// that takes no orders still gets a panel.
    #[test]
    fn a_panel_offers_the_controls_that_widget_declares() {
        let water = rows_for(&water(1, 8));

        let offered: Vec<&str> = water
            .iter()
            .filter_map(|row| match row {
                Row::Action { action, .. } => Some(*action),
                Row::Heading { .. } => None,
            })
            .collect();
        assert_eq!(offered, vec!["drink", "undo", "reset"]);

        let clock = rows_for(&clock());
        assert_eq!(clock.len(), 1, "a clock has nothing to be told");
        assert!(matches!(clock[0], Row::Heading { .. }));
    }

    /// Nine of the twelve widgets have no variant of their own, and their
    /// panels still have to be panels.
    #[test]
    fn every_widget_on_offer_has_a_panel_with_something_in_it() {
        for id in doca_ipc::WIDGETS {
            let state = WidgetState::new(
                id,
                Body::Simple(doca_ipc::Simple {
                    label: String::new(),
                    detail: String::new(),
                    progress: doca_ipc::NO_PROGRESS,
                    active: false,
                }),
            );

            let rows = rows_for(&state);

            assert!(!rows.is_empty(), "{id} opens an empty panel");
            assert_eq!(
                rows[0],
                Row::Heading {
                    title: name_of(id),
                    detail: String::new()
                },
                "{id} has nothing at the top of its panel to say what it is"
            );
        }
    }

    /// The whole of the item: the panel is up before the daemon has answered.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_panel_opens_before_the_daemon_has_answered() {
        let rows = rows_for(&water(3, 8));
        let panel = opening(rows.len());
        let window = a_window();
        let (show, _) = against(&window);
        show(&panel);
        settle();

        assert!(panel.is_visible(), "the click put nothing on screen");
        assert_eq!(panel.children().len(), rows.len());
        assert!(
            panel.children().iter().all(|row| !row.is_sensitive()),
            "a row that holds nothing yet can still be chosen"
        );

        fill(
            &panel,
            rows.len(),
            "water",
            &rows,
            &invokes_nothing(),
            &show,
            Instant::now() - SETTLED,
        );
        settle();

        assert!(panel.is_visible(), "filling the panel closed it");
        assert_eq!(panel.children().len(), rows.len());
        assert_eq!(
            panel
                .children()
                .iter()
                .filter(|row| row.is_sensitive())
                .count(),
            3,
            "the three controls arrived but cannot be chosen"
        );
        panel.popdown();
        window.close();
        settle();
    }

    /// A panel opened at the shape it turns out to have does not move at all.
    /// This is the open that happens over and over.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_panel_opened_at_the_shape_it_turns_out_to_have_does_not_move() {
        let rows = rows_for(&water(3, 8));
        let panel = opening(rows.len());
        let window = a_window();
        let (show, shown) = against(&window);
        show(&panel);
        settle();
        let before = window_of(&panel);

        fill(
            &panel,
            rows.len(),
            "water",
            &rows,
            &invokes_nothing(),
            &show,
            Instant::now() - SETTLED,
        );
        settle();

        assert_eq!(
            shown.get(),
            1,
            "the panel was taken down and put up again for a shape it already had"
        );
        assert_eq!(
            window_of(&panel),
            before,
            "the panel resized under the pointer even though the guess was right"
        );
        panel.popdown();
        window.close();
        settle();
    }

    /// And the other half: a guess that was wrong still shows every row,
    /// rather than hiding the rest behind a scroll arrow.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_panel_larger_than_it_opened_for_is_still_shown_whole() {
        let panel = opening(UNKNOWN);
        let window = a_window();
        let (show, shown) = against(&window);
        show(&panel);
        settle();

        let rows = rows_for(&water(3, 8));
        assert_ne!(rows.len(), UNKNOWN, "this test needs a wrong guess");
        fill(
            &panel,
            UNKNOWN,
            "water",
            &rows,
            &invokes_nothing(),
            &show,
            Instant::now() - SETTLED,
        );
        settle();

        assert_eq!(shown.get(), 2, "the panel never came back at its new size");
        let (_, natural) = panel.preferred_size();
        let (width, height) = window_of(&panel);
        assert!(
            width >= natural.width && height >= natural.height,
            "the panel is {width}x{height} for {}x{} of rows: the rest is \
             behind a scroll arrow",
            natural.width,
            natural.height
        );
        panel.popdown();
        window.close();
        settle();
    }

    /// A panel opened before is opened at the size it was, which is what
    /// keeps the common case from blinking.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_panel_opened_before_is_drawn_at_the_size_it_was() {
        let panels = Remembered::default();

        assert_eq!(panels.of("water"), UNKNOWN, "a panel nobody opened has no size to use");
        panels.note("water", rows_for(&water(3, 8)).len());

        assert_eq!(panels.of("water"), 4);
        assert_eq!(
            opening(panels.of("water")).children().len(),
            4,
            "the second open guessed again instead of using what it saw"
        );
    }

    /// Choosing a control tells the daemon, and nothing else does.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn choosing_a_control_asks_the_daemon_for_it() {
        let asked: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(Vec::new()));
        let telling = asked.clone();
        let invoke: Invoke = Rc::new(move |id: &str, action: &str| {
            telling.borrow_mut().push((id.to_string(), action.to_string()));
        });

        let rows = rows_for(&water(1, 8));
        let panel = opening(rows.len());
        let window = a_window();
        let (show, _) = against(&window);
        show(&panel);
        settle();
        fill(&panel, rows.len(), "water", &rows, &invoke, &show, Instant::now() - SETTLED);
        settle();

        // The heading first: it must not be a control.
        for row in panel.children() {
            if let Ok(item) = row.downcast::<gtk::MenuItem>() {
                if item.is_sensitive() {
                    item.activate();
                }
            }
        }
        settle();

        assert_eq!(
            *asked.borrow(),
            vec![
                ("water".to_string(), "drink".to_string()),
                ("water".to_string(), "undo".to_string()),
                ("water".to_string(), "reset".to_string()),
            ],
            "the controls and the heading did not ask for what they say"
        );
        panel.popdown();
        window.close();
        settle();
    }

    /// A panel is navigable by keyboard, and the heading is not a stop on the
    /// way.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn the_keyboard_walks_the_controls_and_skips_the_heading() {
        let rows = rows_for(&water(1, 8));
        let panel = opening(rows.len());
        let window = a_window();
        let (show, _) = against(&window);
        show(&panel);
        settle();
        fill(&panel, rows.len(), "water", &rows, &invokes_nothing(), &show, Instant::now() - SETTLED);
        settle();

        panel.select_first(true);
        settle();

        let selected = panel
            .selected_item()
            .expect("a panel with controls in it selects one");
        assert!(selected.is_sensitive(), "the keyboard landed on the heading");
        assert_eq!(
            panel.children().iter().position(|row| *row == selected),
            Some(1),
            "the first arrow key did not land on the first control"
        );
        panel.popdown();
        window.close();
        settle();
    }
}
