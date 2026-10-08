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
/// The chassis knew two kinds of row and said the kinds a widget needs are
/// that widget's issue to add. [`Row::Week`] is the first of those, added by
/// #28 — the panel grows by a drawer here rather than by a second panel
/// somewhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// What the widget is showing now, in words rather than as a drawing.
    Heading { title: String, detail: String },
    /// Something the widget can be told to do.
    Action {
        action: &'static str,
        label: &'static str,
    },
    /// Seven days as bars, oldest first, today last and marked.
    Week { days: Vec<u32>, goal: u32 },
    /// An album cover at a size worth looking at.
    Cover { art: String },
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
/// being typed: a water panel says "1500 of 2000 ml" because the state says
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
        Ok(Body::Note(note)) => Row::Heading {
            title: name_of(&state.id),
            detail: note.text.lines().next().unwrap_or("empty note").to_string(),
        },
        Ok(Body::Music(music)) => Row::Heading {
            title: if music.title.is_empty() {
                name_of(&state.id)
            } else {
                music.title.clone()
            },
            detail: if music.artist.is_empty() {
                music.player.clone()
            } else {
                music.artist.clone()
            },
        },
        Ok(Body::Water(water)) => Row::Heading {
            title: format!("{} of {} ml", water.drunk, water.goal),
            detail: if water.drunk >= water.goal {
                "done for today".to_string()
            } else {
                format!("{} ml a bottle", water.bottle)
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
    // The cover sits above the controls for the reason the week does: it is
    // what you opened the panel to see. Only when there is one — a row that
    // is a picture of nothing is worse than no row.
    if let Ok(Body::Music(music)) = state.body() {
        if !music.art.is_empty() {
            rows.push(Row::Cover { art: music.art });
        }
    }
    // The week sits above the controls: it is what you came to look at, and
    // the controls are what you do about it.
    if let Ok(Body::Water(water)) = state.body() {
        rows.push(Row::Week {
            days: water.week.clone(),
            goal: water.goal,
        });
    }
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
        Row::Cover { art } => {
            let picture = gtk::Image::new();
            // Square and the width of the panel, which is the one size a
            // menu row can be sure of: the rows beside it are text at
            // `ROW_CHARS`, and a cover wider than them would stretch the menu
            // around itself.
            if let Some(pixbuf) = crate::dock::scaled_to_fill(art, COVER, COVER) {
                picture.set_from_pixbuf(Some(&pixbuf));
            }

            let row = gtk::MenuItem::new();
            row.add(&picture);
            // A picture, not a control.
            row.set_sensitive(false);
            row
        }
        Row::Week { days, goal } => {
            let area = gtk::DrawingArea::new();
            area.set_size_request(-1, WEEK_HEIGHT);
            let (days, goal) = (days.clone(), *goal);
            area.connect_draw(move |area, cr| {
                // The colour is read here, off the row that is really on
                // screen, and handed down — so the drawing itself is numbers
                // and a colour, and can be checked without a widget.
                let ink = area.style_context().color(gtk::StateFlags::NORMAL);
                draw_week(
                    &days,
                    goal,
                    ink,
                    cr,
                    area.allocated_width() as f64,
                    area.allocated_height() as f64,
                );
                glib::Propagation::Proceed
            });

            let row = gtk::MenuItem::new();
            row.add(&area);
            // A picture, not a control: insensitive keeps the keyboard off it
            // so the arrows go straight from the heading to the first thing
            // that does something.
            row.set_sensitive(false);
            row
        }
    }
}

/// How big the cover is in the panel, square.
const COVER: f64 = 180.0;

/// How tall the week's bars are drawn.
const WEEK_HEIGHT: i32 = 34;

/// The mark under today, and the room kept for it.
const TODAY_MARK: f64 = 3.0;

/// A week of days as bars, in whatever colour the desktop's menus use.
///
/// `ink` is read off the row that is on screen rather than picked here, for
/// the reason the heading is dimmed rather than coloured: the panel wears the
/// desktop's menu style, and a colour chosen in this file would be a guess
/// about a background it never selected. The groove and the fill are that one
/// ink at different strengths, so they are right on any of them.
///
/// The size is handed in rather than read off a widget, which is what lets
/// this be checked by drawing it onto a surface and counting — a drawer that
/// asks an unrealised widget how wide it is draws into a sliver and says
/// nothing is wrong.
fn draw_week(
    days: &[u32],
    goal: u32,
    ink: gdk::RGBA,
    cr: &gtk::cairo::Context,
    width: f64,
    height: f64,
) {
    if width <= 0.0 || height <= 0.0 || days.is_empty() {
        return;
    }
    let gap = 4.0;
    let count = days.len() as f64;
    let bar = ((width - gap * (count - 1.0)) / count).max(1.0);
    let tall = (height - TODAY_MARK - 2.0).max(1.0);

    for (at, ml) in days.iter().enumerate() {
        let x = at as f64 * (bar + gap);
        let today = at + 1 == days.len();
        let share = if goal == 0 {
            0.0
        } else {
            (*ml as f64 / goal as f64).clamp(0.0, 1.0)
        };

        cr.set_source_rgba(ink.red(), ink.green(), ink.blue(), ink.alpha() * 0.16);
        cr.rectangle(x, 0.0, bar, tall);
        let _ = cr.fill();

        if share > 0.0 {
            // Today at full strength and the days behind it quieter: the week
            // is there to be glanced at, and the one you can still change is
            // the one worth looking at.
            let strength = if today { 1.0 } else { 0.5 };
            cr.set_source_rgba(ink.red(), ink.green(), ink.blue(), ink.alpha() * strength);
            let filled = tall * share;
            cr.rectangle(x, tall - filled, bar, filled);
            let _ = cr.fill();
        }

        if today {
            // Marked underneath rather than by its colour alone, because a
            // today with nothing drunk in it has no bar to be brighter than
            // the others.
            cr.set_source_rgba(ink.red(), ink.green(), ink.blue(), ink.alpha());
            cr.rectangle(x, height - TODAY_MARK, bar, TODAY_MARK);
            let _ = cr.fill();
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

    fn water(drunk: u32, goal: u32) -> WidgetState {
        WidgetState::new(
            "water",
            Body::Water(doca_ipc::Water { drunk, goal, bottle: 500, week: vec![0, 250, 0, 500, 1000, 750, drunk] }),
        )
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

    fn music(title: &str, art: &str) -> WidgetState {
        WidgetState::new(
            "music",
            Body::Music(doca_ipc::Music {
                title: title.to_string(),
                artist: "João Gilberto".to_string(),
                player: "rhythmbox".to_string(),
                playing: true,
                art: art.to_string(),
            }),
        )
    }

    /// The cover is what you opened the panel for, so it is above the
    /// controls — and it is the panel's job, not the tile's: the tile has
    /// room for a thumbnail with words on it, and this has room for the
    /// picture.
    #[test]
    fn a_music_panel_puts_the_cover_above_its_controls() {
        let rows = rows_for(&music("Garota de Ipanema", "/tmp/cover.png"));

        let cover = rows
            .iter()
            .position(|row| matches!(row, Row::Cover { .. }))
            .expect("the panel has no cover in it");
        let first_control = rows
            .iter()
            .position(|row| matches!(row, Row::Action { .. }))
            .expect("the panel has no controls");

        assert!(cover < first_control);
        assert_eq!(rows[cover], Row::Cover { art: "/tmp/cover.png".to_string() });
    }

    /// A track with no cover gets no row, rather than a row that is a picture
    /// of nothing. The controls are the same either way — they come from the
    /// id, not from the body.
    #[test]
    fn a_track_with_no_cover_gets_no_row_for_one() {
        let rows = rows_for(&music("Some radio stream", ""));

        assert!(!rows.iter().any(|row| matches!(row, Row::Cover { .. })));
        assert_eq!(
            rows.iter().filter(|row| matches!(row, Row::Action { .. })).count(),
            3,
            "the controls went with the cover"
        );
    }

    /// The week is in the panel and nowhere else: the tile has room for the
    /// day, and seven days is what you open the panel to see.
    #[test]
    fn a_water_panel_carries_the_week_above_its_controls() {
        let rows = rows_for(&water(750, 2000));

        let week = rows
            .iter()
            .position(|row| matches!(row, Row::Week { .. }))
            .expect("the panel has no week in it");
        let first_control = rows
            .iter()
            .position(|row| matches!(row, Row::Action { .. }))
            .expect("the panel has no controls");

        assert!(
            week < first_control,
            "the week is below the controls, which is not what you opened it for"
        );
        assert_eq!(
            rows[week],
            Row::Week {
                // The fixture's week, with today in the last place.
                days: vec![0, 250, 0, 500, 1000, 750, 750],
                goal: 2000
            }
        );
    }

    /// And only water has one. A clock with a week of bars under it would be
    /// a drawer that ran on the wrong body.
    #[test]
    fn a_widget_with_nothing_to_show_a_week_of_has_none() {
        assert!(!rows_for(&clock())
            .iter()
            .any(|row| matches!(row, Row::Week { .. })));
    }

    /// The point of a typed body: the sentence is the panel's to write,
    /// because the numbers arrived as numbers.
    #[test]
    fn a_water_panel_says_what_the_count_means() {
        let rows = rows_for(&water(1500, 2000));

        assert_eq!(
            rows[0],
            Row::Heading {
                title: "1500 of 2000 ml".to_string(),
                // What the next press will add, which is the one number the
                // panel knows and the tile has no room to say.
                detail: "500 ml a bottle".to_string()
            }
        );
        assert_eq!(
            rows_for(&water(2000, 2000))[0],
            Row::Heading {
                title: "2000 of 2000 ml".to_string(),
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
                Row::Heading { .. } | Row::Week { .. } | Row::Cover { .. } => None,
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

    /// Each bar in the week is as tall as that day was full, and today is
    /// marked whether or not anything has been drunk in it.
    ///
    /// Counted in pixels for the reason the bottle's test is: a drawing is
    /// the one part of this nothing else can reach, and the bottle's first
    /// version was full at every level with every other test passing.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn the_weeks_bars_stand_for_the_days_they_are_drawn_from() {
        let (width, height) = (210, WEEK_HEIGHT);
        let painted = |days: Vec<u32>| -> Vec<usize> {
            let mut surface =
                gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, width, height)
                    .expect("no surface");
            {
                let cr = gtk::cairo::Context::new(&surface).expect("no context");
                let ink = gdk::RGBA::new(1.0, 1.0, 1.0, 1.0);
                draw_week(&days, 2000, ink, &cr, width as f64, height as f64);
            }
            surface.flush();
            let stride = surface.stride() as usize;
            let data = surface.data().expect("the surface is still borrowed");
            // How much ink each seventh of the row got, which is one bar.
            (0..7)
                .map(|at| {
                    let from = at * width as usize / 7;
                    let to = (at + 1) * width as usize / 7;
                    let mut ink = 0usize;
                    for row in 0..height as usize {
                        for column in from..to {
                            ink += data[row * stride + column * 4 + 3] as usize;
                        }
                    }
                    ink
                })
                .collect()
        };

        let climbing = painted(vec![0, 300, 600, 900, 1200, 1500, 1800]);
        for pair in climbing.windows(2) {
            assert!(
                pair[1] > pair[0],
                "a fuller day is not a taller bar: {climbing:?}"
            );
        }

        // Today is the last place, and it is marked even when it is empty —
        // a day with nothing in it has no bar to be brighter than the rest.
        let empty_today = painted(vec![0, 0, 0, 0, 0, 0, 0]);
        assert!(
            empty_today[6] > empty_today[5],
            "today is not marked on a day nothing has been drunk: {empty_today:?}"
        );
    }

    /// Cancelling the panel closes it and lets go of the grab — which is
    /// what Escape and a click outside both do.
    ///
    /// Both come free with a `GtkMenu` and neither was ever checked, which is
    /// the worst way for a thing to be true: free behaviour is exactly what
    /// goes missing in a change of surface, and nothing here would have said
    /// so. The click outside is not simulated — there is no pointer to click
    /// with — so what is asserted is the mechanism underneath it: a menu that
    /// holds the pointer and keyboard grab is a menu that dismisses on a
    /// press anywhere else, and a panel that stopped holding it would fail
    /// here rather than in somebody's hand.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_panel_closes_on_escape_and_holds_the_grab_that_dismisses_it() {
        let rows = rows_for(&water(3, 8));
        let panel = opening(rows.len());
        let window = a_window();
        let (show, _) = against(&window);
        show(&panel);
        settle();
        assert!(panel.is_visible(), "the panel never opened");

        assert_eq!(
            gtk::grab_get_current().map(|held| held.type_()),
            Some(panel.type_()),
            "the panel is up without the grab, so a click outside would not close it"
        );

        // `cancel` is the keybinding signal `GtkMenuShell` binds Escape to,
        // so firing it is firing what Escape fires. The key itself is not
        // synthesised: a menu under a grab takes keys through the grab rather
        // than through its own window, and `gtk_test_widget_send_key` posts
        // into the window — the event arrives nowhere and the test passes or
        // fails on the harness instead of on the panel. That Escape really
        // reaches this signal over a window of type DOCK was measured with a
        // real X key press before this surface was chosen.
        panel.cancel();
        settle();

        assert!(!panel.is_visible(), "Escape left the panel on screen");
        assert!(
            gtk::grab_get_current().is_none(),
            "the panel closed but kept the grab, which freezes the bar under it"
        );

        window.close();
        settle();
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

        // A heading, the week, and the three controls water declares.
        assert_eq!(panels.of("water"), 5);
        assert_eq!(
            opening(panels.of("water")).children().len(),
            5,
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
        // The first row that does something, rather than a fixed place: the
        // panel grew a week between the heading and the controls, and it will
        // grow more — what has to hold is that the arrow skips everything
        // that is only there to be looked at.
        assert_eq!(
            panel.children().iter().position(|row| *row == selected),
            panel.children().iter().position(|row| row.is_sensitive()),
            "the first arrow key did not land on the first control"
        );
        panel.popdown();
        window.close();
        settle();
    }
}
