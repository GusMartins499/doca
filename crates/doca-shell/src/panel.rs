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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

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

/// What typing into a row does: tell the daemon what it now says.
///
/// Separate from [`Invoke`] because it is a different kind of thing — an
/// action is a verb the widget declared, and this is a value only the widget
/// in question has a key for.
pub type Write = Rc<dyn Fn(&str, &str)>;

/// Everything a panel's rows can ask the daemon for.
///
/// One value rather than two arguments, because every row that needs either
/// is handed both and `fill` was growing a parameter per kind of row.
#[derive(Clone)]
pub struct Asks {
    pub invoke: Invoke,
    pub write: Write,
}

/// What a control does when it is chosen: tell the daemon so.
pub type Invoke = Rc<dyn Fn(&str, &str)>;

/// How the panel is put on screen: the click that opened it, applied again.
///
/// The same closure opens it and reopens it, so a panel that has to be
/// resized cannot come back somewhere other than where it was opened.
pub type Show = Rc<dyn Fn(&gtk::Menu)>;

/// Above the tile that asked, and the same way every time it is reopened.
///
/// Not `popup_at_widget`, which measures the widget's allocation against the
/// widget's own window. A tile is an `EventBox`, whose window already stands
/// where its allocation says, so the tile's offset in the bar was counted
/// twice: the panel came up as far below the tile's top as the tile is below
/// the bar's, over the tile it belongs to, and as far right again — which is
/// off the screen for a tile at the end of the bar, so the panel was pushed
/// back against the edge and squeezed to fit what was left. The tile's place
/// in its toplevel, measured once, is what the panel hangs from.
pub fn above(tile: &gtk::Widget) -> Show {
    let tile = tile.clone();
    Rc::new(move |menu: &gtk::Menu| {
        let place = tile.toplevel().and_then(|top| {
            let window = top.window()?;
            let (x, y) = tile.translate_coordinates(&top, 0, 0)?;
            let (width, height) = (tile.allocated_width(), tile.allocated_height());
            Some((window, gdk::Rectangle::new(x, y, width, height)))
        });
        match place {
            Some((window, rect)) => menu.popup_at_rect(
                &window,
                &rect,
                gdk::Gravity::NorthWest,
                gdk::Gravity::SouthWest,
                None,
            ),
            // A tile that is not on a window yet has nowhere to be above.
            None => menu.popup_at_pointer(None),
        }
    })
}

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
    /// Something you type into, rather than something you read.
    Text { value: String },
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
    // The note is edited where it lives, which is the whole of #30: the
    // preferences window is for choices, and a note is not a choice.
    if let Ok(Body::Note(note)) = state.body() {
        rows.push(Row::Text { value: note.text });
    }
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
pub struct Remembered(Rc<RefCell<HashMap<String, Shape>>>);

impl Remembered {
    pub fn of(&self, id: &str) -> Shape {
        self.0.borrow().get(id).copied().unwrap_or_default()
    }

    /// What the panel that is up holds now, rows and size both.
    pub fn note(&self, id: &str, panel: &gtk::Menu) {
        let (_, natural) = panel.preferred_size();
        let shape = Shape {
            rows: panel.children().len(),
            size: Some((natural.width, natural.height)),
        };
        self.0.borrow_mut().insert(id.to_string(), shape);
    }
}

/// The shape a panel is opened at, before its rows are in.
///
/// The size and not only the count: the rows waiting for an answer are a
/// line each, and the rows that answer are not — the week is a chart, the
/// heading is two lines — so a panel opened at the right number of rows was
/// still too short for them, and the controls ended up behind a scroll arrow
/// on every open after the first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub rows: usize,
    /// How big the panel stood, once something has been in it to measure.
    pub size: Option<(i32, i32)>,
}

impl Default for Shape {
    fn default() -> Self {
        Self { rows: UNKNOWN, size: None }
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
pub fn opening(expected: Shape) -> gtk::Menu {
    let menu = gtk::Menu::new();
    menu.set_widget_name("panel");
    menu.set_reserve_toggle_size(false);

    for _ in 0..expected.rows.max(1) {
        menu.append(&waiting_row());
    }
    // As big as it stood, or the window this pops up in is the size of the
    // rows that are only waiting.
    if let Some((width, height)) = expected.size {
        menu.set_size_request(width, height);
    }

    menu.show_all();
    menu
}

/// Put the widget's rows into the panel that is already on screen.
///
/// When the rows take the room the panel was opened with — which is every
/// open after the first — they are swapped in and nothing moves. When they do
/// not, the panel has to be popped down and up again: a menu keeps the window
/// it sized at popup, so rows beyond it are not shown but scrolled to. The
/// room is measured, not counted: the same number of rows is not the same
/// height when the ones waiting are a line and the ones arriving are not.
///
/// What comes back is how to bring those same rows up to date while the
/// panel stays up — see [`Live`].
pub fn fill(
    menu: &gtk::Menu,
    id: &str,
    rows: &[Row],
    asks: &Asks,
    show: &Show,
    opened: Instant,
) -> Live {
    for row in menu.children() {
        menu.remove(&row);
    }

    let mut updates = Vec::with_capacity(rows.len());
    for row in rows {
        let (item, update) = drawn(menu, id, row, asks);
        menu.append(&item);
        updates.push(update);
    }
    if rows.is_empty() {
        let empty = gtk::MenuItem::with_label("nothing to show");
        empty.set_sensitive(false);
        menu.append(&empty);
    }

    // Its own size from here on, whatever it was opened at.
    menu.set_size_request(-1, -1);
    menu.show_all();
    if !fits(menu) {
        reopen(menu, show, opened);
    }
    Live(Rc::new(RefCell::new(Filled { rows: rows.to_vec(), updates })))
}

/// Whether the window the panel is up in is the size its rows are.
fn fits(menu: &gtk::Menu) -> bool {
    let Some(window) = menu.toplevel() else { return false };
    let (_, natural) = menu.preferred_size();
    (window.allocated_width(), window.allocated_height()) == (natural.width, natural.height)
}

/// The rows of a panel that is up, and how to change each without taking it
/// down.
///
/// A panel that outlives its own controls (#28: drinking from it has to show
/// the week growing) is a panel whose contents move while it is open. Putting
/// new rows in would do it and would cost the keyboard its place — the item
/// it had selected is gone — so the rows are kept and told what they now say.
#[derive(Clone, Default)]
pub struct Live(Rc<RefCell<Filled>>);

#[derive(Default)]
struct Filled {
    rows: Vec<Row>,
    updates: Vec<Option<Update>>,
}

/// How one row takes on what a newer row of the same kind says.
type Update = Box<dyn Fn(&Row)>;

impl Live {
    /// Bring the panel to these rows, if it can be done in place.
    ///
    /// `false` when it cannot: the rows are a different shape — a cover that
    /// arrived, a widget that lost its body — and only [`fill`] can draw
    /// those. Rows of the same kind in the same places always can.
    pub fn refresh(&self, rows: &[Row]) -> bool {
        let mut filled = self.0.borrow_mut();
        let same_shape = filled.rows.len() == rows.len()
            && filled.rows.iter().zip(rows).all(|(was, now)| match (was, now) {
                // A control is its verb: one that changed is another control.
                (Row::Action { .. }, Row::Action { .. }) => was == now,
                _ => std::mem::discriminant(was) == std::mem::discriminant(now),
            });
        if !same_shape {
            return false;
        }
        for ((was, now), update) in filled.rows.iter().zip(rows).zip(&filled.updates) {
            if was != now {
                if let Some(update) = update {
                    update(now);
                }
            }
        }
        filled.rows = rows.to_vec();
        true
    }
}

/// The panel that is up, if there is one, and what to do with news of it.
///
/// One at a time, because a panel is a menu and a menu holds the grab: a
/// second one cannot go up while the first is. Kept apart from the tiles so
/// that a widget's signal reaches whichever of the two is showing it.
#[derive(Clone, Default)]
pub struct Showing(Rc<RefCell<Option<(String, Takes)>>>);

/// How the panel that is up takes on a widget's newer state.
pub type Takes = Rc<dyn Fn(&WidgetState)>;

impl Showing {
    /// This widget's panel is up, and this is how it takes a new state.
    pub fn now(&self, id: &str, update: Takes) {
        self.0.replace(Some((id.to_string(), update)));
    }

    /// This widget's panel went down — unless another went up since.
    pub fn gone(&self, id: &str) {
        let mut showing = self.0.borrow_mut();
        if showing.as_ref().is_some_and(|(up, _)| up == id) {
            *showing = None;
        }
    }

    /// A widget changed: tell its panel, if its panel is the one up.
    pub fn update(&self, state: &WidgetState) {
        // Cloned out first: the update may well put the panel down and up
        // again, and a panel going down says so through `gone`.
        let update = self
            .0
            .borrow()
            .as_ref()
            .filter(|(id, _)| *id == state.id)
            .map(|(_, update)| update.clone());
        if let Some(update) = update {
            update(state);
        }
    }
}

/// One row, as the thing on screen, and how to change it later.
fn drawn(menu: &gtk::Menu, id: &str, row: &Row, asks: &Asks) -> (gtk::MenuItem, Option<Update>) {
    match row {
        Row::Heading { title, detail } => {
            let name = heading_label(title);
            name.set_attributes(Some(&weighted(pango::Weight::Bold, 1.0)));

            // Always made, and hidden when there is nothing under the name:
            // a heading that gains a detail while it is up has somewhere to
            // put it.
            let under = heading_label(detail);
            under.set_attributes(Some(&weighted(pango::Weight::Normal, 0.85)));
            // Dimmed rather than coloured, for the reason the waiting
            // rows are: the panel wears whatever the desktop's menus
            // wear, and a colour chosen here would be a guess about a
            // background this code never picked.
            under.set_opacity(0.6);
            under.set_no_show_all(detail.is_empty());

            let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
            column.add(&name);
            column.add(&under);

            let heading = gtk::MenuItem::new();
            heading.add(&column);
            // Not a control. Leaving it insensitive also keeps the keyboard
            // off it, so the first arrow key lands on the first real one.
            heading.set_sensitive(false);
            let update: Update = Box::new(move |row| {
                if let Row::Heading { title, detail } = row {
                    name.set_text(title);
                    under.set_text(detail);
                    under.set_visible(!detail.is_empty());
                }
            });
            (heading, Some(update))
        }
        Row::Action { action, label } => {
            let control = gtk::MenuItem::with_label(label);
            let (id, action) = (id.to_string(), action.to_string());
            let invoke = asks.invoke.clone();
            control.connect_activate(move |_| invoke(&id, &action));
            stays_up(menu, &control);
            (control, None)
        }
        // Never told what the note now says: the newest note is the one
        // being typed here, and the daemon's echo of it would put the cursor
        // back at the start of a word.
        Row::Text { value } => (text_row(menu, id, value, &asks.write), None),
        Row::Cover { art } => {
            let picture = gtk::Image::new();
            // Square and the width of the panel, which is the one size a
            // menu row can be sure of: the rows beside it are text at
            // `ROW_CHARS`, and a cover wider than them would stretch the menu
            // around itself.
            let paint = {
                let picture = picture.clone();
                move |art: &str| match crate::dock::scaled_to_fill(art, COVER, COVER) {
                    Some(pixbuf) => picture.set_from_pixbuf(Some(&pixbuf)),
                    None => picture.clear(),
                }
            };
            paint(art);

            let row = gtk::MenuItem::new();
            row.add(&picture);
            // A picture, not a control.
            row.set_sensitive(false);
            let update: Update = Box::new(move |row| {
                if let Row::Cover { art } = row {
                    paint(art);
                }
            });
            (row, Some(update))
        }
        Row::Week { days, goal } => {
            let area = gtk::DrawingArea::new();
            area.set_size_request(-1, WEEK_HEIGHT);
            let week = Rc::new(RefCell::new((days.clone(), *goal)));
            let drawing = week.clone();
            area.connect_draw(move |area, cr| {
                // The colour is read here, off the row that is really on
                // screen, and handed down — so the drawing itself is numbers
                // and a colour, and can be checked without a widget.
                let ink = area.style_context().color(gtk::StateFlags::NORMAL);
                let (days, goal) = &*drawing.borrow();
                draw_week(
                    days,
                    *goal,
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
            let update: Update = Box::new(move |row| {
                if let Row::Week { days, goal } = row {
                    *week.borrow_mut() = (days.clone(), *goal);
                    area.queue_draw();
                }
            });
            (row, Some(update))
        }
    }
}

/// A control that does its thing and leaves the panel where it was.
///
/// # Why this is decided here and not per widget
///
/// #28 left it open: a `GtkMenuItem` takes its menu down when it is chosen,
/// so drinking from the water panel closed the panel and the week you were
/// looking at was gone before it could grow. Play and pause in the music
/// panel did the same. The decision is the chassis's, and it is that a panel
/// survives its controls — they are things you do *in* it, often several in
/// a row (two bottles, next and next), and `Esc` or a click outside is
/// already how a panel is put away.
///
/// Not by a property: `GtkMenuShell` takes the menu down on a release and on
/// `Return` whatever the item says about it. So both are taken before the
/// shell sees them, and the item is activated by hand — which is the same
/// `activate` the shell would have emitted, without the dismissal it adds.
fn stays_up(menu: &gtk::Menu, control: &gtk::MenuItem) {
    // Only a release that this item saw the press of. The click that opened
    // the panel releases somewhere too, and a panel that went up under the
    // pointer must not take that release for a choice.
    let pressed = Rc::new(std::cell::Cell::new(false));
    let pressing = pressed.clone();
    control.connect_button_press_event(move |_, _| {
        pressing.set(true);
        glib::Propagation::Proceed
    });
    control.connect_leave_notify_event({
        let pressed = pressed.clone();
        move |_, _| {
            pressed.set(false);
            glib::Propagation::Proceed
        }
    });
    control.connect_button_release_event(move |control, _| {
        if !pressed.replace(false) {
            return glib::Propagation::Proceed;
        }
        control.activate();
        glib::Propagation::Stop
    });

    // The keyboard's way of choosing it. Weak, because the handler is on the
    // menu and outlives the row: a panel filled again has new controls, and
    // the old ones' handlers must neither keep them alive nor answer for
    // them.
    let weak = control.downgrade();
    menu.connect_key_press_event(move |menu, event| {
        let Some(control) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if !chooses(event.keyval()) || menu.selected_item().as_ref() != Some(control.upcast_ref()) {
            return glib::Propagation::Proceed;
        }
        control.activate();
        glib::Propagation::Stop
    });
}

/// The keys a menu reads as "choose the selected item".
fn chooses(key: gdk::keys::Key) -> bool {
    matches!(
        key,
        gdk::keys::constants::Return
            | gdk::keys::constants::KP_Enter
            | gdk::keys::constants::ISO_Enter
            | gdk::keys::constants::space
            | gdk::keys::constants::KP_Space
    )
}

/// A row you type into.
///
/// # Why this works at all
///
/// A `GtkMenu` under a keyboard grab reads space as "activate the selected
/// item" and the arrows as "move between items", so a text widget dropped
/// into one gets letters and nothing else — you can type `ação` and not
/// `ação, não`. That was measured on this bar before this was written.
///
/// The fix is twelve lines: take the menu's key presses, hand them to the
/// text if the text has focus, and stop the signal so the shell never sees
/// them. Escape and Tab stay with the menu — one closes the panel, the other
/// leaves the row — and everything else, the arrows included, belongs to
/// whoever is being typed into. A note has newlines in it, so the arrows
/// have to move a cursor rather than a selection.
///
/// # When it is written
///
/// After [`QUIET`] of nobody typing, and again when the panel goes down. The
/// issue asked for this to be decided: a write a keystroke is a disk write a
/// keystroke, and a write only on close loses whatever was not closed. The
/// quiet interval means at most a moment of typing is ever at risk, and the
/// close catches the last word of it.
fn text_row(menu: &gtk::Menu, id: &str, value: &str, write: &Write) -> gtk::MenuItem {
    let text = gtk::TextView::new();
    text.set_widget_name("panel-text");
    text.set_wrap_mode(gtk::WrapMode::WordChar);
    text.set_size_request(-1, TEXT_HEIGHT);
    text.buffer().expect("a text view has a buffer").set_text(value);

    let frame = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    frame.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    frame.set_shadow_type(gtk::ShadowType::In);
    frame.add(&text);

    let row = gtk::MenuItem::new();
    row.add(&frame);

    // The keys, handed over.
    let typing = text.clone();
    menu.connect_key_press_event(move |_, event| {
        if menus_own(event.keyval()) || !typing.has_focus() {
            return glib::Propagation::Proceed;
        }
        typing.event(event);
        glib::Propagation::Stop
    });

    let saving = written(id, &text, write);
    let quiet: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let buffer = text.buffer().expect("a text view has a buffer");
    let waiting = quiet.clone();
    let after_quiet = saving.clone();
    buffer.connect_changed(move |_| {
        if let Some(pending) = waiting.replace(None) {
            pending.remove();
        }
        let saving = after_quiet.clone();
        let clearing = waiting.clone();
        let pending = glib::timeout_add_local_once(QUIET, move || {
            clearing.replace(None);
            saving();
        });
        waiting.replace(Some(pending));
    });

    // And once more on the way down, for the word that was still being typed.
    let on_close = saving.clone();
    let pending = quiet.clone();
    menu.connect_unmap(move |_| {
        if let Some(waiting) = pending.replace(None) {
            waiting.remove();
        }
        on_close();
    });

    row
}

/// Which keys stay with the menu while a row is being typed into.
///
/// Everything else goes to the text, and that is the whole of the fix: a
/// `GtkMenuShell` reads space as "activate the selected item" and the arrows
/// as "move between items", so without this a note gets letters and nothing
/// else — you type `ação, não` and it says `ação,não`. That was measured on
/// this bar with real key presses before the row was written.
///
/// Escape closes the panel and Tab leaves the row, and those are the two
/// things you cannot do from inside a text view. The arrows are *not* on
/// this list on purpose: a note has newlines in it, so up and down have to
/// move a cursor rather than a selection, and the menu gives up vertical
/// navigation for as long as the note has the focus.
fn menus_own(key: gdk::keys::Key) -> bool {
    matches!(
        key,
        gdk::keys::constants::Escape
            | gdk::keys::constants::Tab
            | gdk::keys::constants::ISO_Left_Tab
    )
}

/// Send what the text now says, if it says anything new.
fn written(id: &str, text: &gtk::TextView, write: &Write) -> Rc<dyn Fn()> {
    let (id, write) = (id.to_string(), write.clone());
    let buffer = text.buffer().expect("a text view has a buffer");
    let last = Rc::new(RefCell::new(current(&buffer)));
    Rc::new(move || {
        let now = current(&buffer);
        // Unchanged is not worth a write: the panel closing would otherwise
        // save a note nobody touched, every time it was opened to read one.
        if *last.borrow() == now {
            return;
        }
        *last.borrow_mut() = now.clone();
        write(&id, &now);
    })
}

fn current(buffer: &gtk::TextBuffer) -> String {
    let (from, to) = buffer.bounds();
    buffer.text(&from, &to, false).map(|text| text.to_string()).unwrap_or_default()
}

/// How long the typing has to stop before it is written down.
const QUIET: std::time::Duration = std::time::Duration::from_millis(600);

/// How tall the note is in the panel.
const TEXT_HEIGHT: i32 = 90;

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
        once_let_go(&menu, &show);
        menu.popdown();
    };

    let waited = opened.elapsed();
    if waited >= SETTLED {
        swap();
    } else {
        glib::timeout_add_local_once(SETTLED - waited, swap);
    }
}

/// How long a panel being reopened waits for its old grab to be let go of
/// before it goes up regardless.
const LET_GO: Duration = Duration::from_millis(300);

/// Show the panel again once the grab it is about to give up has been reported
/// gone, for the pointer and for the keyboard both.
///
/// A panel comes back in the same window it went down from, and GDK reports
/// the end of the old grab as a grab-broken event on that window — one per
/// device, each when that device next says something, which is a round trip
/// later and not a turn of the loop later. A panel already back up takes the
/// late one as its own grab breaking and closes itself: the panel came back
/// and was gone again in the same breath, on every first open. So it waits
/// for both; and for no longer than [`LET_GO`], so that a report that never
/// comes costs a moment and not the panel.
fn once_let_go(menu: &gtk::Menu, show: &Show) {
    let shown = Rc::new(Cell::new(false));
    let listening: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::default();
    let go = {
        let (menu, show, listening) = (menu.clone(), show.clone(), listening.clone());
        move || {
            if let Some(handler) = listening.borrow_mut().take() {
                menu.disconnect(handler);
            }
            if !shown.replace(true) {
                show(&menu);
            }
        }
    };

    let (pointer, keyboard) = (Rc::new(Cell::new(false)), Rc::new(Cell::new(false)));
    let soon = go.clone();
    let handler = menu.connect_grab_broken_event(move |_, broken| {
        if broken.is_keyboard() {
            keyboard.set(true);
        } else {
            pointer.set(true);
        }
        if pointer.get() && keyboard.get() {
            // After this event is done with, not inside it.
            glib::idle_add_local_once(soon.clone());
        }
        // The menu's own handler still has to hear the grab it held is gone.
        glib::Propagation::Proceed
    });
    listening.replace(Some(handler));
    glib::timeout_add_local_once(LET_GO, go);
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

    fn writes_nothing() -> Write {
        Rc::new(|_: &str, _: &str| {})
    }

    fn asks_nothing() -> Asks {
        Asks { invoke: invokes_nothing(), write: writes_nothing() }
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

    /// Let GTK and the X server both get on with it, for as long as a panel
    /// taken down to be resized may take to come back — which waits on the
    /// server, so a turn of the loop is not enough.
    fn settle_past_a_reopen() {
        let until = Instant::now() + LET_GO * 2;
        while Instant::now() < until {
            settle();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The shape these rows leave behind once a panel has held them — what
    /// every open after the first is opened at.
    fn opened_before(rows: &[Row]) -> Shape {
        let panels = Remembered::default();
        let window = a_window();
        let (show, _) = against(&window);
        let panel = opening(panels.of("water"));
        show(&panel);
        settle();
        fill(&panel, "water", rows, &asks_nothing(), &show, Instant::now() - SETTLED);
        settle_past_a_reopen();
        panels.note("water", &panel);
        panel.popdown();
        window.close();
        settle();
        panels.of("water")
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
                Row::Heading { .. }
                | Row::Week { .. }
                | Row::Cover { .. }
                | Row::Text { .. } => None,
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

    /// Which keys the menu keeps while a note is being typed into.
    ///
    /// The delivery is GTK's — a menu under a keyboard grab cannot be sent a
    /// synthetic key at all, which is why the Escape test fires `cancel`
    /// rather than pressing it. What is mine is this list, and it is the
    /// thing that would be lost: drop the handler and a note gets letters and
    /// no spaces, which looks like it works until somebody writes two words.
    ///
    /// That it really works was measured on this bar with real key presses
    /// before the row was written: `ação, não` typed into a panel over a
    /// `_NET_WM_WINDOW_TYPE_DOCK` window came out `ação,não` without the
    /// handler and whole with it.
    #[test]
    fn the_menu_keeps_only_the_keys_a_note_cannot_use() {
        use gdk::keys::constants as key;

        // Its own: one closes the panel, the other leaves the row, and
        // neither is a thing a text view can do.
        assert!(menus_own(key::Escape));
        assert!(menus_own(key::Tab));
        assert!(menus_own(key::ISO_Left_Tab));

        // The note's. Space is the one that matters — it is what a menu reads
        // as "activate" and what two words need.
        for typed in [key::space, key::a, key::ccedilla, key::Return, key::BackSpace] {
            assert!(!menus_own(typed), "the menu kept a key the note needed");
        }

        // And the arrows, which a selection would want and a cursor needs
        // more: a note has newlines in it.
        for arrow in [key::Up, key::Down, key::Left, key::Right] {
            assert!(!menus_own(arrow), "the arrows cannot move a cursor in a note");
        }
    }

    /// Closing a panel that was only read must not write anything: opening a
    /// note to look at it would otherwise touch the config every time.
    pub fn reading_a_note_and_closing_writes_nothing() {
        let panel = opening(Shape { rows: 1, size: None });
        let window = a_window();
        let (show, _) = against(&window);
        let typed = Rc::new(RefCell::new(Vec::<String>::new()));
        let noting = typed.clone();
        let write: Write = Rc::new(move |_: &str, text: &str| {
            noting.borrow_mut().push(text.to_string())
        });

        fill(
            &panel,
            "note",
            &[Row::Text { value: "call the dentist".to_string() }],
            &Asks { invoke: invokes_nothing(), write },
            &show,
            Instant::now() - SETTLED,
        );
        show(&panel);
        settle();
        panel.popdown();
        settle();

        assert!(
            typed.borrow().is_empty(),
            "a note nobody touched was written anyway: {:?}",
            typed.borrow()
        );
        window.close();
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
        let panel = opening(opened_before(&rows));
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
        let panel = opening(opened_before(&rows));
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
            "water",
            &rows,
            &asks_nothing(),
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
        let panel = opening(opened_before(&rows));
        let window = a_window();
        let (show, shown) = against(&window);
        show(&panel);
        settle();
        let before = window_of(&panel);

        fill(
            &panel,
            "water",
            &rows,
            &asks_nothing(),
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
        let panel = opening(Shape::default());
        let window = a_window();
        let (show, shown) = against(&window);
        show(&panel);
        settle();

        let rows = rows_for(&water(3, 8));
        assert_ne!(rows.len(), Shape::default().rows, "this test needs a wrong guess");
        fill(
            &panel,
            "water",
            &rows,
            &asks_nothing(),
            &show,
            Instant::now() - SETTLED,
        );
        settle();

        settle_past_a_reopen();
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

    /// Where a panel's window is on the screen: left, top, width, height.
    fn placed(menu: &gtk::Menu) -> (i32, i32, i32, i32) {
        let top = menu.toplevel().expect("a panel that is up has a window");
        let (_, x, y) = top.window().expect("and the window is real").origin();
        let (width, height) = window_of(menu);
        (x, y, width, height)
    }

    /// A panel hangs from the top of the tile that asked: above it, never
    /// over it.
    ///
    /// The tile is an `EventBox` partway down its window, as on the bar —
    /// a tile at the window's corner would hide an offset counted twice.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_panel_hangs_above_the_tile_that_asked() {
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.move_(100, 400);
        let tile = gtk::EventBox::new();
        tile.set_size_request(120, 50);
        tile.set_margin_top(40);
        tile.set_margin_start(200);
        tile.set_halign(gtk::Align::Start);
        tile.set_valign(gtk::Align::Start);
        window.add(&tile);
        window.show_all();
        settle();

        // Until the window manager has put the window where it goes: before
        // then the window's origin is still the corner it was created in.
        settle_past_a_reopen();

        let tile: gtk::Widget = tile.upcast();
        let panel = opening(Shape::default());
        above(&tile)(&panel);
        settle();

        let (_, left, top) = window.window().expect("a window that is up is real").origin();
        let (x, y) = tile.translate_coordinates(&window, 0, 0).expect("the tile is in the window");
        let (tile_left, tile_top) = (left + x, top + y);

        let (panel_left, panel_top, _, height) = placed(&panel);
        assert_eq!(
            (panel_left, panel_top + height),
            (tile_left, tile_top),
            "the panel's bottom-left corner is not on the tile's top-left one"
        );
        panel.popdown();
        window.close();
        settle();
    }

    /// The second open of a panel is shown whole too, not only the first —
    /// a remembered panel is opened before its rows are in, and rows that
    /// are only waiting are not as tall as the week they stand in for.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_panel_opened_again_is_shown_whole() {
        let panels = Remembered::default();
        let window = a_window();
        let (show, _) = against(&window);
        let rows = rows_for(&water(3, 8));

        for open in ["first", "second"] {
            let panel = opening(panels.of("water"));
            show(&panel);
            settle();
            fill(&panel, "water", &rows, &asks_nothing(), &show, Instant::now() - SETTLED);
            panels.note("water", &panel);
            settle_past_a_reopen();

            let (_, natural) = panel.preferred_size();
            let (width, height) = window_of(&panel);
            assert!(
                panel.is_visible() && width >= natural.width && height >= natural.height,
                "the {open} open is {width}x{height} for {}x{} of rows: the \
                 rest is behind a scroll arrow",
                natural.width,
                natural.height
            );
            panel.popdown();
            settle();
        }
        window.close();
        settle();
    }

    /// A panel opened before is opened at the size it was, which is what
    /// keeps the common case from blinking.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_panel_opened_before_is_drawn_at_the_size_it_was() {
        let panels = Remembered::default();
        assert_eq!(
            panels.of("water"),
            Shape::default(),
            "a panel nobody opened has no size to use"
        );

        let rows = rows_for(&water(3, 8));
        let window = a_window();
        let (show, _) = against(&window);
        let panel = opening(panels.of("water"));
        show(&panel);
        settle();
        fill(&panel, "water", &rows, &asks_nothing(), &show, Instant::now() - SETTLED);
        settle_past_a_reopen();
        panels.note("water", &panel);
        let (_, natural) = panel.preferred_size();
        panel.popdown();
        window.close();
        settle();

        // A heading, the week, and the three controls water declares — and
        // the room they took, which is not five lines.
        assert_eq!(
            panels.of("water"),
            Shape { rows: 5, size: Some((natural.width, natural.height)) }
        );
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
        let panel = opening(opened_before(&rows));
        let window = a_window();
        let (show, _) = against(&window);
        show(&panel);
        settle();
        fill(
            &panel,
            "water",
            &rows,
            &Asks { invoke, write: writes_nothing() },
            &show,
            Instant::now() - SETTLED,
        );
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

    /// A key press as the menu would get it through its grab.
    ///
    /// Built by hand rather than posted: under a grab a posted key arrives
    /// nowhere (see the Escape test). What the menu's bindings look up is the
    /// *hardware* keycode, so that is taken from the keymap, not made up.
    fn a_key(menu: &gtk::Menu, key: gdk::keys::Key) -> gdk::Event {
        use glib::translate::{IntoGlib, ToGlibPtr, ToGlibPtrMut};
        let window = menu
            .toplevel()
            .and_then(|top| top.window())
            .expect("a panel that is up has a window");
        let keycode = gdk::Keymap::for_display(&window.display())
            .and_then(|keymap| keymap.entries_for_keyval(*key).first().map(|entry| entry.keycode()))
            .expect("the keymap has no key for this");
        let mut event = gdk::Event::new(gdk::EventType::KeyPress);
        // SAFETY: a KeyPress event is a GdkEventKey; the window reference
        // handed over is the one `gdk_event_free` gives back.
        unsafe {
            let raw: *mut gdk::ffi::GdkEvent = event.to_glib_none_mut().0;
            let raw = raw as *mut gdk::ffi::GdkEventKey;
            (*raw).window = window.to_glib_full();
            (*raw).send_event = 1;
            (*raw).keyval = key.into_glib();
            (*raw).hardware_keycode = keycode as u16;
        }
        event
    }

    /// A click as a row would get it, pressed or released.
    fn a_click(menu: &gtk::Menu, kind: gdk::EventType) -> gdk::Event {
        use glib::translate::{ToGlibPtr, ToGlibPtrMut};
        let window = menu
            .toplevel()
            .and_then(|top| top.window())
            .expect("a panel that is up has a window");
        let mut event = gdk::Event::new(kind);
        // SAFETY: a button event is a GdkEventButton; the window reference
        // handed over is the one `gdk_event_free` gives back.
        unsafe {
            let raw: *mut gdk::ffi::GdkEvent = event.to_glib_none_mut().0;
            let raw = raw as *mut gdk::ffi::GdkEventButton;
            (*raw).window = window.to_glib_full();
            (*raw).send_event = 1;
            (*raw).button = 1;
        }
        event
    }

    /// The water panel, up and filled, with what its controls asked for.
    fn a_water_panel(
        drunk: u32,
    ) -> (gtk::Menu, gtk::Window, Live, Rc<RefCell<Vec<String>>>) {
        let asked: Rc<RefCell<Vec<String>>> = Rc::default();
        let telling = asked.clone();
        let invoke: Invoke = Rc::new(move |_: &str, action: &str| {
            telling.borrow_mut().push(action.to_string());
        });
        let rows = rows_for(&water(drunk, 2000));
        let panel = opening(opened_before(&rows));
        let window = a_window();
        let (show, _) = against(&window);
        show(&panel);
        settle();
        let live = fill(
            &panel,
            "water",
            &rows,
            &Asks { invoke, write: writes_nothing() },
            &show,
            Instant::now() - SETTLED,
        );
        settle();
        (panel, window, live, asked)
    }

    fn first_control(panel: &gtk::Menu) -> gtk::MenuItem {
        panel
            .children()
            .into_iter()
            .find(|row| row.is_sensitive())
            .and_then(|row| row.downcast::<gtk::MenuItem>().ok())
            .expect("the panel has no controls")
    }

    /// The decision #28 left to the chassis: a control does its thing and the
    /// panel stays, so drinking shows the week growing instead of taking it
    /// away.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn choosing_a_control_leaves_the_panel_up() {
        let (panel, window, _, asked) = a_water_panel(500);
        let drink = first_control(&panel);

        // By the keyboard: Return is what `GtkMenuShell` binds to "activate
        // and take the menu down", whatever the item says about it.
        panel.select_item(&drink);
        settle();
        panel.event(&a_key(&panel, gdk::keys::constants::Return));
        settle();
        assert_eq!(*asked.borrow(), vec!["drink"], "Return did not choose the control");
        assert!(panel.is_visible(), "choosing a control by Return took the panel down");

        // And by the pointer: a press and its release, on the row.
        drink.event(&a_click(&panel, gdk::EventType::ButtonPress));
        let taken = drink.event(&a_click(&panel, gdk::EventType::ButtonRelease));
        settle();
        assert!(taken, "the release went on to the menu, which takes the panel down");
        assert_eq!(*asked.borrow(), vec!["drink", "drink"], "the click did not choose it");
        assert!(panel.is_visible(), "choosing a control by clicking took the panel down");

        panel.popdown();
        window.close();
        settle();
    }

    /// The click that opened the panel releases somewhere, and if the panel
    /// went up under the pointer that is on a control. It is not a choice.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn the_release_of_the_click_that_opened_a_panel_chooses_nothing() {
        let (panel, window, _, asked) = a_water_panel(500);
        let drink = first_control(&panel);

        let taken = drink.event(&a_click(&panel, gdk::EventType::ButtonRelease));
        settle();
        assert!(!taken, "a release with no press was kept from the menu");
        assert!(asked.borrow().is_empty(), "a release nobody pressed chose {:?}", asked.borrow());

        panel.popdown();
        window.close();
        settle();
    }

    /// The panel shows what a control did, in place: the very rows that were
    /// there, saying something newer.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_panel_that_is_up_takes_the_widgets_news_in_place() {
        let (panel, window, live, _) = a_water_panel(500);
        let before = panel.children();
        let drink = first_control(&panel);
        panel.select_item(&drink);

        assert!(
            live.refresh(&rows_for(&water(1000, 2000))),
            "a newer state of the same widget could not be shown in place"
        );
        settle();

        assert_eq!(panel.children(), before, "the rows were remade, not updated");
        assert_eq!(
            panel.selected_item().as_ref(),
            Some(drink.upcast_ref::<gtk::Widget>()),
            "the keyboard lost its place to the update"
        );
        let heading = format!("{:?}", texts_in(before[0].clone()));
        assert!(heading.contains("1000 of 2000 ml"), "the heading still says {heading}");

        panel.popdown();
        window.close();
        settle();
    }

    /// Every label's text under a widget, in order.
    fn texts_in(widget: gtk::Widget) -> Vec<String> {
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            return vec![label.text().to_string()];
        }
        match widget.downcast::<gtk::Container>() {
            Ok(container) => container.children().into_iter().flat_map(texts_in).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Rows of another shape are not forced into the ones on screen.
    #[test]
    fn a_panel_of_another_shape_is_not_refreshed_in_place() {
        let live = Live(Rc::new(RefCell::new(Filled {
            rows: rows_for(&water(500, 2000)),
            updates: Vec::new(),
        })));
        assert!(!live.refresh(&rows_for(&clock())), "a clock's rows were poured into a water panel");
        assert!(
            live.refresh(&rows_for(&water(750, 2000))),
            "the same widget's next state was refused"
        );
    }

    /// A panel is navigable by keyboard, and the heading is not a stop on the
    /// way.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn the_keyboard_walks_the_controls_and_skips_the_heading() {
        let rows = rows_for(&water(1, 8));
        let panel = opening(opened_before(&rows));
        let window = a_window();
        let (show, _) = against(&window);
        show(&panel);
        settle();
        fill(&panel, "water", &rows, &asks_nothing(), &show, Instant::now() - SETTLED);
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
