//! The tiles, which are drawn rather than assembled.
//!
//! A tile used to be two `GtkLabel`s and a `GtkProgressBar` in a column, 88
//! pixels wide whatever the icons beside it were doing. That is the shape of
//! a tile that can only ever say two words and a percentage: there is nowhere
//! in it to put a ring round a number, a row of bars for the week, or a
//! gradient, because every one of those is a drawing and not a label.
//!
//! So each kind of widget gets a drawer of its own, taking the [`Body`] that
//! kind sends and the [`Look`] the bar is wearing. Nothing in here can reach
//! the daemon: a tile holds the state it was last given and a closure that
//! opens a panel, and the drawing path touches neither the bus nor anything
//! that could block on it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::fit::Level;
use crate::motion::{self, Life};
use doca_ipc::{Body, Simple, Tile, Water, WidgetState, NO_PROGRESS};
use gtk::cairo;
use gtk::prelude::*;

use crate::theme::Palette;

/// How tall a tile is: the icons' own height, so a tile sits in exactly the
/// slot an icon would and the bar's height is unchanged by having one.
pub fn height_of(icon_size: i32) -> i32 {
    icon_size
}

/// What a tile is drawn at: the size the icons are, and the colours the theme
/// paints with.
///
/// One value rather than two arguments because every tile takes both and
/// neither ever changes without a rebuild — a tile asked to redraw itself
/// mid-frame reads this and nothing else.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub icon_size: i32,
    pub palette: Palette,
    /// How much of itself the tile is still drawing. Everything but
    /// [`Level::Full`] is a crowded bar giving way — see `fit`.
    pub level: Level,
}

/// What a click on a tile does: ask for this widget's panel, over this tile.
///
/// A closure rather than anything that knows what a panel is, for the reason
/// the folder grid takes one: a tile that cannot be built without a daemon on
/// the other end cannot be checked without one either.
pub type Expand = Rc<dyn Fn(&str, &gtk::Widget)>;

/// What a tile's left button does once the hand has moved it past the drag
/// threshold: #18, P3, "reorder by dragging, items and widgets".
///
/// Points are root coordinates. `Lift` also says where on the tile it was
/// taken hold of, so what is carried stays under the hand where it took it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    Lift { at: (f64, f64), grab: (f64, f64) },
    Carry { at: (f64, f64) },
    Drop { at: (f64, f64) },
}

/// How much of the bar's card shows round a tile being carried.
pub const CARRY_PAD: i32 = 6;

/// Whoever moves tiles: told which tile, and what the hand did with it.
pub type Grip = Rc<dyn Fn(&str, Gesture)>;

#[derive(Clone)]
pub struct WidgetTile {
    pub root: gtk::Widget,
    area: gtk::DrawingArea,
    /// What the drawing path reads, and the only thing it reads.
    shown: Rc<RefCell<WidgetState>>,
    look: Rc<Cell<Look>>,
    /// How far onto the bar the tile has got, on the icons' own curve.
    ///
    /// A dock switched for another takes its tiles with it and brings its
    /// own, and a tile that blinked in and out beside icons that grew and
    /// shrank was the part of the switch that still read as a rebuild.
    life: Rc<Cell<Life>>,
    ticking: Rc<Cell<bool>>,
    /// What choosing the tile does — a click, or `Enter` while it has the
    /// keyboard. One closure for both, so the two cannot open different
    /// things.
    opens: Rc<dyn Fn()>,
    /// In the hand: the tile is a hole the size of itself, and a picture of
    /// it is carried at the pointer.
    held: Rc<Cell<bool>>,
    /// How much of its room the tile holds, besides arriving and leaving:
    /// less than all of it for the frames after it moved slot, while the
    /// room it left closes over the same frames. See `Shelf::carry_to`.
    room: Rc<Cell<f64>>,
    grip: Rc<RefCell<Option<Grip>>>,
    /// What the panel says while the note is being typed into it.
    ///
    /// Kept apart from `shown` because the daemon is always behind the
    /// typing: an answer to the write of a moment ago carries the text of a
    /// moment ago, and shown as it came, the last letters would vanish from
    /// the tile and come back.
    typed: Rc<RefCell<Option<String>>>,
}

impl WidgetTile {
    pub fn new(state: &WidgetState, look: Look, expand: Expand) -> Self {
        let shown = Rc::new(RefCell::new(state.clone()));
        let held = Rc::new(Cell::new(look));
        let life = Rc::new(Cell::new(Life::settled()));

        let area = gtk::DrawingArea::new();
        area.set_widget_name("widget-face");
        let painting = shown.clone();
        let wearing = held.clone();
        let living = life.clone();
        area.connect_draw(move |area, cr| {
            let presence = living.get().presence();
            if presence < 1.0 {
                draw_passing(&painting.borrow(), wearing.get(), presence, area, cr);
            } else {
                draw(&painting.borrow(), wearing.get(), area, cr);
            }
            glib::Propagation::Proceed
        });

        let tile = gtk::EventBox::new();
        tile.set_widget_name("widget");
        // So that "the keyboard goes back to the bar when the panel closes"
        // is literally true rather than a hope: a widget that cannot take
        // focus cannot be given it back, and `grab_focus` on one is a call
        // that quietly does nothing.
        tile.set_can_focus(true);
        tile.add(&area);

        let id = state.id.clone();
        let root: gtk::Widget = tile.clone().upcast();
        let opening = root.clone();
        let opens: Rc<dyn Fn()> = Rc::new(move || expand(&id, &opening));
        let grip: Rc<RefCell<Option<Grip>>> = Rc::default();
        tile.add_events(gdk::EventMask::BUTTON_RELEASE_MASK | gdk::EventMask::BUTTON1_MOTION_MASK);
        // Either button opens the panel, and nothing else does: the controls
        // live in the panel, so there is one way in rather than a left click
        // that does one thing and a right click that undoes it. The left one
        // opens it on the release, so the same press can pick the tile up —
        // the rule the icons follow.
        let pressed: Rc<Cell<Option<(f64, f64)>>> = Rc::default();
        let moved = Rc::new(Cell::new(false));
        let clicked = opens.clone();
        let pressing = pressed.clone();
        tile.connect_button_press_event(move |_, event| {
            match event.button() {
                1 if event.event_type() == gdk::EventType::ButtonPress => {
                    pressing.set(Some(event.position()));
                }
                1 => {}
                _ => clicked(),
            }
            glib::Propagation::Stop
        });
        let (lifting, carrying, moving) = (grip.clone(), pressed.clone(), moved.clone());
        let named = state.id.clone();
        tile.connect_motion_notify_event(move |tile, event| {
            let Some(from) = carrying.get() else {
                return glib::Propagation::Proceed;
            };
            let at = event.root();
            if !moving.get() {
                let (x, y) = event.position();
                if !tile.drag_check_threshold(from.0 as i32, from.1 as i32, x as i32, y as i32) {
                    return glib::Propagation::Proceed;
                }
                moving.set(true);
                if let Some(grip) = lifting.borrow().clone() {
                    grip(&named, Gesture::Lift { at, grab: from });
                }
            }
            if let Some(grip) = lifting.borrow().clone() {
                grip(&named, Gesture::Carry { at });
            }
            glib::Propagation::Proceed
        });
        let clicked = opens.clone();
        let dropping = grip.clone();
        let named = state.id.clone();
        tile.connect_button_release_event(move |_, event| {
            if event.button() != 1 {
                return glib::Propagation::Stop;
            }
            let was = pressed.replace(None);
            if moved.replace(false) {
                if let Some(grip) = dropping.borrow().clone() {
                    grip(&named, Gesture::Drop { at: event.root() });
                }
            } else if was.is_some() {
                clicked();
            }
            glib::Propagation::Stop
        });

        let tile = Self {
            root,
            area,
            shown,
            look: held,
            life,
            ticking: Rc::new(Cell::new(false)),
            opens,
            held: Rc::new(Cell::new(false)),
            room: Rc::new(Cell::new(1.0)),
            grip,
            typed: Rc::default(),
        };
        tile.resize();
        tile.mark();
        tile.speak();
        tile
    }

    /// Tell a screen reader what this tile says, and that it opens something.
    ///
    /// A tile is a drawing, and a drawing says nothing to anyone who cannot
    /// see it. The name is the tile's state in words — the same words the
    /// panel's heading uses — and the role is a button, because choosing it
    /// opens the panel.
    fn speak(&self) {
        if let Some(accessible) = self.root.accessible() {
            use atk::prelude::*;
            accessible.set_role(atk::Role::PushButton);
            accessible.set_name(&spoken(&self.shown.borrow()));
        }
    }

    /// Open this widget's panel over the tile, as a click on it does.
    pub fn open(&self) {
        (self.opens)();
    }

    /// Hand the tile to whoever moves tiles.
    fn grip_with(&self, grip: Option<Grip>) {
        self.grip.replace(grip);
    }

    /// The tile as it is drawn now, on a card of the bar's own, for carrying
    /// at the pointer.
    ///
    /// The card is the point: on the bar a tile has no background of its
    /// own — the bar is its background — so the tile alone, lifted off it,
    /// was its words floating over whatever was under the pointer. Seen in
    /// the sandbox. The card is painted from the same `#ground` rules the bar
    /// is, so it is the bar's colour, edge and corners in every theme, with
    /// `CARRY_PAD` of it showing round the tile.
    fn snapshot(&self) -> Option<gtk::gdk_pixbuf::Pixbuf> {
        let (width, height) = (self.area.allocated_width(), self.area.allocated_height());
        if width <= 0 || height <= 0 {
            return None;
        }
        let (outer_w, outer_h) = (width + CARRY_PAD * 2, height + CARRY_PAD * 2);
        // In device pixels, the way the float takes every picture. The scale
        // is on the context and not the surface, because
        // `pixbuf_get_from_surface` counts raw pixels whatever a surface
        // says its density is.
        let density = self.area.scale_factor();
        let (raw_w, raw_h) = (outer_w * density, outer_h * density);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, raw_w, raw_h).ok()?;
        {
            let cr = cairo::Context::new(&surface).ok()?;
            cr.scale(density as f64, density as f64);
            let card = gtk::DrawingArea::new();
            card.set_widget_name("ground");
            let context = card.style_context();
            let (w, h) = (outer_w as f64, outer_h as f64);
            gtk::render_background(&context, &cr, 0.0, 0.0, w, h);
            gtk::render_frame(&context, &cr, 0.0, 0.0, w, h);
            cr.translate(CARRY_PAD as f64, CARRY_PAD as f64);
            self.root.draw(&cr);
        }
        gdk::pixbuf_get_from_surface(&surface, 0, 0, raw_w, raw_h)
    }

    /// Picked up, or put down. A tile in the hand is drawn by nothing: the
    /// room it holds is where it will go.
    fn hold(&self, held: bool) {
        self.held.set(held);
        self.pass();
    }

    /// Start from nothing and grow onto the bar.
    fn arrive(&self) {
        self.life.set(Life::arriving());
        self.pass();
        self.animate();
    }

    /// Shrink off the bar, from wherever it had got to.
    fn leave(&self) {
        if !self.life.get().leaving() {
            self.life.set(self.life.get().reversed());
        }
        self.animate();
    }

    /// Asked for again before it had finished leaving: it turns round where
    /// it stands, the way an app reopened mid-close does on the row.
    fn stay(&self) {
        if self.life.get().leaving() {
            self.life.set(self.life.get().reversed());
            self.animate();
        }
    }

    /// The width the widget declared, before any arriving or leaving.
    fn declared(&self) -> i32 {
        declared_width(&self.shown.borrow(), self.look.get())
    }

    /// One frame of the passage: the room it holds, and how much of it shows.
    ///
    /// The room is the same fraction of the declared width that an icon
    /// takes of its slot, so a tile and an icon arriving together open the
    /// bar at the same rate. What is drawn in it is the tile at its own size,
    /// scaled the way an arriving icon is and faded with it.
    fn pass(&self) {
        self.resize();
        let shown = if self.held.get() {
            0.0
        } else {
            self.life.get().presence()
        };
        self.root.set_opacity(shown);
        self.area.queue_draw();
    }

    /// Run the passage on the compositor's clock, and stop once it is over.
    ///
    /// A tile that has finished leaving takes itself off the bar here, the
    /// way a departing icon lets go of its slot only once it is gone.
    fn animate(&self) {
        if self.ticking.replace(true) {
            return;
        }
        let tile = self.clone();
        self.root.add_tick_callback(move |_, _| {
            tile.pass();
            if tile.life.get().moving() {
                return glib::ControlFlow::Continue;
            }
            tile.ticking.set(false);
            if tile.life.get().gone() {
                tile.unshelve();
            }
            glib::ControlFlow::Break
        });
    }

    /// Off whatever it was packed in, if it still is.
    fn unshelve(&self) {
        if let Some(parent) = self
            .root
            .parent()
            .and_then(|parent| parent.downcast::<gtk::Container>().ok())
        {
            parent.remove(&self.root);
        }
    }

    /// How much wider this tile would be at its full declared size.
    fn unfolding(&self) -> i32 {
        (self.declared() - self.area.size_request().0).max(0)
    }

    /// What this tile is showing now.
    pub fn update(&self, state: &WidgetState) {
        let state = &written_over(state, self.typed.borrow().as_deref());
        let resize = self.shown.borrow().tile() != state.tile();
        *self.shown.borrow_mut() = state.clone();
        if resize {
            self.resize();
        }
        self.mark();
        self.speak();
        self.area.queue_draw();
    }

    /// The panel says this now, or has come down and the daemon has the
    /// last word again.
    fn typing(&self, text: Option<&str>) {
        self.typed.replace(text.map(str::to_string));
        if text.is_some() {
            let shown = self.shown.borrow().clone();
            self.update(&shown);
        }
    }

    /// The bar changed size or colour under a tile that is staying.
    pub fn relook(&self, look: Look) {
        if self.look.get() == look {
            return;
        }
        self.look.set(look);
        self.resize();
        self.area.queue_draw();
    }

    /// Ask for exactly the size the widget declared.
    ///
    /// A size request and not a minimum: a tile that grew to fit its own text
    /// would move its neighbours every time a track title changed, so the
    /// text is ellipsised into the declared width instead.
    fn resize(&self) {
        let look = self.look.get();
        let width =
            (self.declared() as f64 * self.life.get().presence() * self.room.get()).round() as i32;
        let size = (width, height_of(look.icon_size));
        if self.area.size_request() != size {
            self.area.set_size_request(size.0, size.1);
        }
    }

    /// The one thing still left to the stylesheet: the wash of accent behind
    /// a tile that is marking itself.
    fn mark(&self) {
        let active = matches!(
            self.shown.borrow().body(),
            Ok(Body::Simple(Simple { active: true, .. }))
        ) || matches!(
            self.shown.borrow().body(),
            Ok(Body::Water(Water { drunk, goal, .. })) if drunk >= goal
        ) || matches!(
            self.shown.borrow().body(),
            Ok(Body::Claude(doca_ipc::Claude { near: true, .. }))
        );
        let style = self.root.style_context();
        if active {
            style.add_class("active");
        } else {
            style.remove_class("active");
        }
    }
}

/// One drawer per [`Body`], and the whole of what a tile is.
///
/// A body this build cannot read draws a dash rather than nothing: a blank
/// space on the bar is indistinguishable from a bar that has gone wrong,
/// whereas a dash says "there is a widget here and it has nothing to say".
fn draw(state: &WidgetState, look: Look, area: &gtk::DrawingArea, cr: &cairo::Context) {
    let width = area.allocated_width() as f64;
    let height = area.allocated_height() as f64;
    draw_in(state, look, area, cr, width, height);
}

/// The width a widget declared for its tile, at the size the bar is wearing.
fn declared_width(state: &WidgetState, look: Look) -> i32 {
    look.level
        .width(state.tile().unwrap_or(Tile::Wide), look.icon_size)
}

/// A tile on its way onto the bar or off it.
///
/// Drawn at its own declared size and scaled about the middle of the room it
/// holds so far — `motion::entry_scale`, the curve an icon arriving is drawn
/// on — rather than laid out again into a room too narrow for it, which
/// would ellipsise every word on the way in and out.
fn draw_passing(
    state: &WidgetState,
    look: Look,
    presence: f64,
    area: &gtk::DrawingArea,
    cr: &cairo::Context,
) {
    let width = declared_width(state, look) as f64;
    let height = area.allocated_height() as f64;
    let scale = motion::entry_scale(presence);
    cr.translate(area.allocated_width() as f64 / 2.0, height / 2.0);
    cr.scale(scale, scale);
    cr.translate(-width / 2.0, -height / 2.0);
    draw_in(state, look, area, cr, width, height);
}

fn draw_in(
    state: &WidgetState,
    look: Look,
    area: &gtk::DrawingArea,
    cr: &cairo::Context,
    width: f64,
    height: f64,
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }

    match state.body() {
        // A widget that measures a share wears it on a card; the rest of the
        // simple ones, and a meter with nothing to measure, are two lines.
        Ok(Body::Simple(simple)) => match gauge_of(&state.id, &simple) {
            Some(gauge) => draw_meter(&simple, gauge, look, area, cr, width, height),
            None => draw_simple(&simple, look, area, cr, width, height),
        },
        Ok(Body::Water(water)) => draw_water(&water, look, area, cr, width, height),
        Ok(Body::Music(music)) => draw_music(&music, look, area, cr, width, height),
        Ok(Body::Note(note)) => draw_note(&note, look, area, cr, width, height),
        Ok(Body::Calendar(calendar)) => draw_calendar(&calendar, look, area, cr, width, height),
        Ok(Body::Claude(claude)) => draw_claude(&claude, look, area, cr, width, height),
        Err(_) => {
            let layout = line(area, "\u{2014}", text_size(look.icon_size, 0.30), true, width);
            let (_, logical) = layout.pixel_extents();
            paint(cr, &layout, 0.0, (height - logical.height() as f64) / 2.0, look.palette.detail);
        }
    }
}

/// A note's state with the text the panel has now, on the paper it came on.
/// Anything else, or a note nobody is typing into, passes through.
fn written_over(state: &WidgetState, typed: Option<&str>) -> WidgetState {
    match (typed, state.body()) {
        (Some(text), Ok(Body::Note(note))) => WidgetState::new(
            state.id.clone(),
            Body::Note(doca_ipc::Note { text: text.to_string(), ..note }),
        ),
        _ => state.clone(),
    }
}

/// A tile's state in words, for a screen reader.
///
/// The widget's name first, so a list of tiles read aloud says which is
/// which, then what it is showing.
pub fn spoken(state: &WidgetState) -> String {
    let name = crate::panel::name_of(&state.id);
    match state.body() {
        Ok(Body::Simple(simple)) => match (simple.label.is_empty(), simple.detail.is_empty()) {
            (false, false) => format!("{name}: {}, {}", simple.label, simple.detail),
            (false, true) => format!("{name}: {}", simple.label),
            (true, false) => format!("{name}: {}", simple.detail),
            (true, true) => name,
        },
        Ok(Body::Water(water)) => format!("{name}: {} of {} ml", water.drunk, water.goal),
        Ok(Body::Music(music)) if music.title.is_empty() => format!("{name}: nothing playing"),
        Ok(Body::Music(music)) if music.artist.is_empty() => format!("{name}: {}", music.title),
        Ok(Body::Music(music)) => format!("{name}: {} by {}", music.title, music.artist),
        Ok(Body::Note(note)) => match note.text.lines().map(str::trim).find(|line| !line.is_empty()) {
            Some(line) => format!("{name}: {line}"),
            None => format!("{name}: empty"),
        },
        // The date and what comes first, which is what the tile shows.
        Ok(Body::Calendar(calendar)) if !calendar.missing.is_empty() => {
            format!("{name}: {} {}, {}", calendar.weekday, calendar.day, calendar.missing)
        }
        Ok(Body::Calendar(calendar)) => match calendar.upcoming.first() {
            Some(next) => format!("{name}: {} {}, {} {}", calendar.weekday, calendar.day, next.when, next.title),
            None => format!("{name}: {} {}, nothing ahead", calendar.weekday, calendar.day),
        },
        // The number and the small line under it, which is what the card
        // says when it has the room to say all of it.
        Ok(Body::Claude(claude)) => {
            let under = claude_under(&claude);
            match (claude.label.is_empty(), under.is_empty()) {
                (false, false) => format!("{name}: {}, {under}", claude.label),
                (false, true) => format!("{name}: {}", claude.label),
                (true, false) => format!("{name}: {under}"),
                (true, true) => name,
            }
        }
        Err(_) => name,
    }
}

/// Millilitres as a tile can show them.
///
/// Four digits do not fit a badge the size of one icon, and nobody reads
/// "1750" as a quantity of water anyway — past a litre, litres is the unit
/// people actually speak in. The tenth is dropped when it is zero, so a
/// round two litres is `2L` and not `2.0L`.
fn compact_ml(ml: u32) -> String {
    if ml < 1000 {
        return ml.to_string();
    }
    let litres = ml as f64 / 1000.0;
    if ((litres * 10.0).round() as u32).is_multiple_of(10) {
        format!("{litres:.0}L")
    } else {
        format!("{litres:.1}L")
    }
}

/// The tile the nine widgets without a variant of their own still draw: a
/// line, a quieter line under it, and a bar if there is anything to measure.
fn draw_simple(
    simple: &Simple,
    look: Look,
    area: &gtk::DrawingArea,
    cr: &cairo::Context,
    width: f64,
    height: f64,
) {
    let label = line(area, &simple.label, text_size(look.icon_size, 0.30), true, width);
    let detail = line(area, &simple.detail, text_size(look.icon_size, 0.21), false, width);

    // What the tile gives up first when the bar is crowded: the quiet second
    // line, and then the bar under it. The loud line is the last thing left,
    // because a tile that cannot say its own number is not worth the pixels.
    let shows_detail = look.level == Level::Full && !simple.detail.is_empty();
    let shows_bar = look.level != Level::Minimal && simple.progress != NO_PROGRESS;

    let label_height = label.pixel_extents().1.height() as f64;
    let detail_height = if shows_detail {
        detail.pixel_extents().1.height() as f64
    } else {
        0.0
    };
    let bar = shows_bar.then(|| bar_height(look.icon_size));
    let bar_room = bar.map(|thickness| thickness + GAP).unwrap_or(0.0);

    let stack = label_height + detail_height + bar_room;
    let mut y = ((height - stack) / 2.0).max(0.0);

    paint(cr, &label, 0.0, y, look.palette.label);
    y += label_height;
    if detail_height > 0.0 {
        paint(cr, &detail, 0.0, y, look.palette.detail);
        y += detail_height;
    }
    if let Some(thickness) = bar {
        let fraction = simple.progress.clamp(0.0, 1.0);
        groove(cr, 0.0, y + GAP, width, thickness, fraction, look.palette.track, look.palette.fill);
    }
}

/// What a meter wears: the glass of its card, the colour its share is drawn
/// in on that glass, and the rim it lights when it wants to be seen.
///
/// A meter brings its own colours, the way water does, rather than the
/// theme's. The theme's palette is chosen to stand off the bar, and inside a
/// card the bar is not what the words stand on: the glass is. Colours of its
/// own let every meter be dark glass with white on it, which reads the same
/// under all four themes, and let a meter say something with its colour — a
/// hot processor, a battery running low — that one accent per theme cannot.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Gauge {
    glass: Glass,
    fill: (f64, f64, f64),
    /// A rim drawn over the glass's own, for a state that should be seen
    /// before it is read. The `.active` wash `mark` puts behind the tile is
    /// under the card, where nobody sees it.
    edge: Option<(f64, f64, f64)>,
}

/// The processor at work: a cool green, on glass to match.
const CPU_GAUGE: Gauge = Gauge {
    glass: Glass {
        from: (0.07, 0.18, 0.16),
        to: (0.07, 0.11, 0.17),
        rim: (0.22, 0.06),
        lifted: false,
    },
    fill: (0.33, 0.87, 0.67),
    edge: None,
};

/// The processor working hard enough that the daemon marks the tile: the
/// same card gone the colour of heat.
const CPU_HOT_GAUGE: Gauge = Gauge {
    glass: Glass {
        from: (0.27, 0.10, 0.06),
        to: (0.17, 0.07, 0.10),
        rim: (0.22, 0.06),
        lifted: false,
    },
    fill: (1.0, 0.58, 0.30),
    edge: None,
};

/// A battery being run on: slate glass, and the charge in a spring green.
const BATTERY_GAUGE: Gauge = Gauge {
    glass: Glass {
        from: (0.12, 0.14, 0.18),
        to: (0.08, 0.09, 0.14),
        rim: (0.22, 0.06),
        lifted: false,
    },
    fill: (0.56, 0.90, 0.44),
    edge: None,
};

/// A battery on the charger — what the daemon marks it for: the glass goes
/// green with it.
const BATTERY_CHARGING_GAUGE: Gauge = Gauge {
    glass: Glass {
        from: (0.06, 0.21, 0.12),
        to: (0.06, 0.13, 0.12),
        rim: (0.22, 0.06),
        lifted: false,
    },
    fill: (0.42, 0.95, 0.55),
    edge: None,
};

/// A battery running low and not on the charger: red glass, so it is seen
/// before it is read. The number is still what says how low.
const BATTERY_LOW_GAUGE: Gauge = Gauge {
    glass: Glass {
        from: (0.29, 0.08, 0.09),
        to: (0.17, 0.06, 0.10),
        rim: (0.22, 0.06),
        lifted: false,
    },
    fill: (1.0, 0.45, 0.42),
    edge: None,
};

/// The charge at and under which a battery is low: a fifth, where most
/// desktops start warning.
const LOW_BATTERY: f64 = 0.20;

/// The day, the week or the year going by: dusk violet, for time.
const TIME_GAUGE: Gauge = Gauge {
    glass: Glass {
        from: (0.14, 0.10, 0.25),
        to: (0.08, 0.08, 0.19),
        rim: (0.22, 0.06),
        lifted: false,
    },
    fill: (0.70, 0.60, 1.0),
    edge: None,
};

/// A block of focus: tomato, which is what the technique is named after.
const POMODORO_FOCUS_GAUGE: Gauge = Gauge {
    glass: Glass {
        from: (0.30, 0.09, 0.07),
        to: (0.18, 0.06, 0.08),
        rim: (0.22, 0.06),
        lifted: false,
    },
    fill: (1.0, 0.47, 0.36),
    edge: None,
};

/// A break, short or long: cool sea glass, so a glance tells a break from
/// work without reading the word.
const POMODORO_BREAK_GAUGE: Gauge = Gauge {
    glass: Glass {
        from: (0.06, 0.17, 0.22),
        to: (0.06, 0.11, 0.18),
        rim: (0.22, 0.06),
        lifted: false,
    },
    fill: (0.42, 0.85, 0.93),
    edge: None,
};

/// A timer running down, or paused on the way: honey, warm without being
/// an alarm yet.
const TIMER_GAUGE: Gauge = Gauge {
    glass: Glass {
        from: (0.24, 0.16, 0.05),
        to: (0.15, 0.10, 0.07),
        rim: (0.22, 0.06),
        lifted: false,
    },
    fill: (1.0, 0.76, 0.30),
    edge: None,
};

/// A timer that has rung: the glass goes red and the card lights a hot rim,
/// the way a Claude card nearly spent does, so it is seen across the room.
const TIMER_DONE_GAUGE: Gauge = Gauge {
    glass: Glass {
        from: (0.33, 0.09, 0.06),
        to: (0.20, 0.06, 0.07),
        rim: (0.22, 0.06),
        lifted: false,
    },
    fill: (1.0, 0.50, 0.38),
    edge: Some((1.0, 0.33, 0.24)),
};

/// Whether a timer has rung. The daemon marks a timer that is running as
/// well as one that has rung, so the mark alone cannot tell; one that has
/// rung is marked with all of its time gone.
fn rang(simple: &Simple) -> bool {
    simple.active && simple.progress >= 1.0
}

/// Whether a pomodoro is on a break: the phase is the first thing its
/// detail says, `break` or `long break`, with `· paused` after it when it is.
fn on_a_break(simple: &Simple) -> bool {
    simple.detail.starts_with("break") || simple.detail.starts_with("long break")
}

/// Which meter, if any, this widget is, and what it is wearing now.
///
/// Read off the widget's id, because a meter arrives as a [`Simple`] like
/// every other widget without a body of its own, and off what the state
/// already says — whether it is marked, how full it is — and nothing else.
/// A meter with nothing to measure is no meter: it is two lines, like the
/// battery on a machine without one.
fn gauge_of(id: &str, simple: &Simple) -> Option<Gauge> {
    if simple.progress == NO_PROGRESS {
        return None;
    }
    match id {
        "cpu" if simple.active => Some(CPU_HOT_GAUGE),
        "cpu" => Some(CPU_GAUGE),
        // Marked is charging; low is read off the charge itself, so neither
        // needs the daemon to say anything it does not already.
        "battery" if simple.active => Some(BATTERY_CHARGING_GAUGE),
        "battery" if simple.progress <= LOW_BATTERY => Some(BATTERY_LOW_GAUGE),
        "battery" => Some(BATTERY_GAUGE),
        "time-progress" => Some(TIME_GAUGE),
        // The phase is read off the detail, which already says it; the
        // daemon marks only a running block of focus, so the mark cannot.
        "pomodoro" if on_a_break(simple) => Some(POMODORO_BREAK_GAUGE),
        "pomodoro" => Some(POMODORO_FOCUS_GAUGE),
        "timer" if rang(simple) => Some(TIMER_DONE_GAUGE),
        "timer" => Some(TIMER_GAUGE),
        _ => None,
    }
}

/// The ink on a meter's card, and the quieter ink of its second word.
const ON_A_GAUGE: gdk::RGBA = gdk::RGBA::WHITE;
const QUIET_ON_A_GAUGE: f64 = 0.78;

/// How much of the white the track a share runs in is.
const GAUGE_TRACK: f64 = 0.16;

fn rgb((red, green, blue): (f64, f64, f64)) -> gdk::RGBA {
    gdk::RGBA::new(red, green, blue, 1.0)
}

/// A meter: a share with its number, on a card.
///
/// Wide, the number and what it is a share of sit over a pill as long as the
/// card, filled as far as the share goes. One icon wide there is no room for
/// a pill worth reading, so the share goes round a ring and the number sits
/// in the middle of it — the last two things a meter gives up, because they
/// are the meter.
fn draw_meter(
    simple: &Simple,
    gauge: Gauge,
    look: Look,
    area: &gtk::DrawingArea,
    cr: &cairo::Context,
    width: f64,
    height: f64,
) {
    let icon = look.icon_size as f64;
    let corner = (icon * 0.22).max(3.0).min(height / 2.0);
    let fraction = simple.progress.clamp(0.0, 1.0);
    let track = faded(ON_A_GAUGE, GAUGE_TRACK);
    let fill = rgb(gauge.fill);

    let (x0, y0, w, h) = card(cr, width, height, corner, gauge.glass, composited(area), |cr, width, height| {
        if look.level == Level::Minimal {
            let (cx, cy, radius, thickness) = ring_in(width, height);
            ring(cr, cx, cy, radius, thickness, fraction, track, fill);

            // The number inside the ring, and without its `%` when that is
            // what it takes to fit: the ring already says it is a share. A
            // time has nothing to drop, so it is written smaller instead.
            let inner = ((radius - thickness / 2.0) * 2.0 - 2.0).max(0.0);
            let size = text_size(look.icon_size, 0.22);
            let mut said = simple.label.as_str();
            if line(area, said, size, true, inner).is_ellipsized() {
                said = said.strip_suffix('%').unwrap_or(said);
            }
            let number = fitted(area, said, size, inner);
            let (_, logical) = number.pixel_extents();
            paint(cr, &number, cx - inner / 2.0, cy - logical.height() as f64 / 2.0, ON_A_GAUGE);
            return;
        }

        let pad = (icon * 0.14).max(3.0);
        let room = (width - pad * 2.0).max(0.0);
        let thickness = (icon * 0.08).round().max(2.0);

        let number = line(area, &simple.label, text_size(look.icon_size, 0.30), true, room);
        number.set_alignment(pango::Alignment::Left);
        let (_, logical) = number.pixel_extents();
        let number_height = logical.height() as f64;

        let gap = (icon * 0.08).max(GAP);
        let y = ((height - number_height - gap - thickness) / 2.0).max(0.0);
        paint(cr, &number, pad, y, ON_A_GAUGE);

        // What the number is a share of, on the same line and level with it
        // at the right, in whatever room the number leaves.
        if look.level == Level::Full && !simple.detail.is_empty() {
            let used = logical.width() as f64 + pad * 0.6;
            let of = line(area, &simple.detail, text_size(look.icon_size, 0.19), false, (room - used).max(0.0));
            of.set_alignment(pango::Alignment::Right);
            let lift = (number.baseline() - of.baseline()) as f64 / pango::SCALE as f64;
            paint(cr, &of, pad + used, y + lift, faded(ON_A_GAUGE, QUIET_ON_A_GAUGE));
        }

        groove(cr, pad, y + number_height + gap, room, thickness, fraction, track, fill);
    });

    if let Some(edge) = gauge.edge {
        let _ = cr.save();
        cr.translate(x0, y0);
        rounded(cr, 0.75, 0.75, w - 1.5, h - 1.5, corner);
        cr.set_source_rgb(edge.0, edge.1, edge.2);
        cr.set_line_width(1.5);
        let _ = cr.stroke();
        let _ = cr.restore();
    }
}

/// A number as big as `size` and no bigger than its room: written smaller
/// rather than cut off, which would leave `18:…` of a time. Down to a floor
/// past which it would not be read anyway, and cut off there.
fn fitted(area: &gtk::DrawingArea, text: &str, size: f64, room: f64) -> pango::Layout {
    let floor = (size * 0.7).max(7.0).min(size);
    let mut size = size;
    loop {
        let layout = line(area, text, size, true, room);
        if !layout.is_ellipsized() || size <= floor {
            return layout;
        }
        size = (size * 0.92).max(floor);
    }
}

/// Where a ring sits in a card one icon wide: in the middle, as big as the
/// card leaves room for, and thick enough to read as a share and not a line.
fn ring_in(width: f64, height: f64) -> (f64, f64, f64, f64) {
    let side = width.min(height);
    let thickness = (side * 0.09).max(2.0);
    let margin = (side * 0.10).max(2.0);
    let radius = (side / 2.0 - margin - thickness / 2.0).max(1.0);
    (width / 2.0, height / 2.0, radius, thickness)
}

/// Water: a glass card with the day poured into it.
///
/// The card is the tile, and the water in it rises with the day against the
/// goal — a surface you read at a glance, the way you read a glass on a desk.
/// The amount is written over the water in white, the week stands beside it
/// as seven little bars, and a round `+` says what a press does.
///
/// Like a note, the card brings its own colours rather than the theme's:
/// water is blue on every desktop, and white on deep blue reads under all of
/// them.
fn draw_water(
    water: &Water,
    look: Look,
    area: &gtk::DrawingArea,
    cr: &cairo::Context,
    width: f64,
    height: f64,
) {
    let fraction = if water.goal == 0 {
        0.0
    } else {
        (water.drunk as f64 / water.goal as f64).clamp(0.0, 1.0)
    };
    let done = water.goal > 0 && water.drunk >= water.goal;
    let icon = look.icon_size as f64;
    let corner = (icon * 0.22).max(3.0).min(height / 2.0);

    card(cr, width, height, corner, WATER_GLASS, composited(area), |cr, width, height| {
        if fraction <= 0.0 {
            return;
        }
        // Even a full card keeps a little air over the surface, or the wave
        // would be cut off by the top edge and a full day would read as flat.
        let surface = height - (height * 0.86) * fraction - height * 0.04;
        let swell = (height * 0.045).max(1.0);

        // The wave behind: paler, a step out of phase, so the water has depth.
        wave(cr, width, height, surface - swell * 0.6, swell, 0.9);
        cr.set_source_rgba(0.42, 0.72, 1.0, 0.35);
        let _ = cr.fill();

        // The water itself, lighter at the surface and deeper at the bottom.
        wave(cr, width, height, surface, swell, 0.0);
        let deep = cairo::LinearGradient::new(0.0, surface - swell, 0.0, height);
        deep.add_color_stop_rgb(0.0, 0.36, 0.68, 1.0);
        deep.add_color_stop_rgb(1.0, 0.10, 0.36, 0.86);
        let _ = cr.set_source(&deep);
        let _ = cr.fill();

        // A light along the crest, which is most of what makes it look wet.
        wave_crest(cr, width, surface, swell, 0.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.45);
        cr.set_line_width((icon * 0.025).max(1.0));
        let _ = cr.stroke();

        // A few bubbles, always in the same places, inside whatever water
        // there is.
        for (x, y, r) in [(0.18, 0.80, 0.030), (0.31, 0.62, 0.020), (0.55, 0.86, 0.025), (0.72, 0.70, 0.018), (0.86, 0.90, 0.022)] {
            let (bx, by) = (width * x, height * y);
            if by - r * height > surface + swell {
                cr.new_path();
                cr.arc(bx, by, (r * height).max(1.0), 0.0, std::f64::consts::TAU);
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.30);
                let _ = cr.fill();
            }
        }
    });

    let pad = (icon * 0.12).max(3.0);
    let ink = gdk::RGBA::WHITE;
    let quiet = faded(ink, 0.82);

    // A card one icon wide is a glass and a count: bottles drunk of the
    // bottles in a day, which is a number you can check against your desk.
    if look.level == Level::Minimal {
        let bottles = water.drunk / water.bottle.max(1);
        let of = water.goal.div_ceil(water.bottle.max(1));
        let count = line(area, &format!("{bottles}/{of}"), text_size(look.icon_size, 0.30), true, width);
        let (_, logical) = count.pixel_extents();
        paint(cr, &count, 0.0, (height - logical.height() as f64) / 2.0, ink);
        return;
    }

    // The `+` and the week, right to left, and only when the card is whole.
    let mut right = width - pad;
    if look.level == Level::Full {
        let diameter = (height * 0.40).min(icon * 0.42);
        let (cx, cy) = (right - diameter / 2.0, height / 2.0);
        // A soft shadow under it, so it still stands out of a full card.
        cr.new_path();
        cr.arc(cx, cy + 1.0, diameter / 2.0 + 1.0, 0.0, std::f64::consts::TAU);
        cr.set_source_rgba(0.02, 0.08, 0.25, 0.45);
        let _ = cr.fill();
        cr.new_path();
        cr.arc(cx, cy, diameter / 2.0, 0.0, std::f64::consts::TAU);
        cr.set_source_rgb(0.16, 0.52, 1.0);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.35);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        let arm = diameter * 0.24;
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.set_line_width((diameter * 0.11).max(1.2));
        cr.set_line_cap(cairo::LineCap::Round);
        cr.move_to(cx - arm, cy);
        cr.line_to(cx + arm, cy);
        cr.move_to(cx, cy - arm);
        cr.line_to(cx, cy + arm);
        let _ = cr.stroke();
        cr.set_line_cap(cairo::LineCap::Butt);
        right = cx - diameter / 2.0 - pad * 0.8;

        let days = water.week.len();
        if days > 0 {
            let bar = (icon * 0.045).max(2.0);
            let spacing = bar * 0.9;
            let span = days as f64 * bar + (days - 1) as f64 * spacing;
            let tallest = height * 0.46;
            let floor = height / 2.0 + tallest / 2.0;
            let most = water.week.iter().copied().max().unwrap_or(0).max(water.goal).max(1);
            let left = right - span;
            for (index, ml) in water.week.iter().enumerate() {
                let tall = (tallest * (*ml as f64 / most as f64)).max(bar);
                let x = left + index as f64 * (bar + spacing);
                rounded(cr, x, floor - tall, bar, tall, bar / 2.0);
                let today = index + 1 == days;
                cr.set_source_rgba(1.0, 1.0, 1.0, if today { 0.95 } else { 0.55 });
                let _ = cr.fill();
            }
            right = left - pad * 0.8;
        }
    }

    let room = (right - pad).max(0.0);
    if room <= 0.0 {
        return;
    }

    let amount = if water.drunk < 10_000 {
        format!("{} ml", water.drunk)
    } else {
        compact_ml(water.drunk)
    };
    let amount = line(area, &amount, text_size(look.icon_size, 0.30), true, room);
    amount.set_alignment(pango::Alignment::Left);
    let against = line(
        area,
        &if done {
            "goal met".to_string()
        } else {
            format!("of {}", compact_ml(water.goal))
        },
        text_size(look.icon_size, 0.19),
        true,
        room,
    );
    against.set_alignment(pango::Alignment::Left);

    // The goal over the amount, small, the way the design has its caption.
    let against_height = against.pixel_extents().1.height() as f64;
    let amount_height = amount.pixel_extents().1.height() as f64;
    let y = ((height - amount_height - against_height) / 2.0).max(0.0);
    paint(cr, &against, pad, y, quiet);
    paint(cr, &amount, pad, y + against_height, ink);
}

/// The top of the water as a closed path: a gentle swell across the card,
/// then down its sides and along the bottom.
fn wave(cr: &cairo::Context, width: f64, height: f64, surface: f64, swell: f64, phase: f64) {
    cr.new_path();
    wave_crest(cr, width, surface, swell, phase);
    cr.line_to(width, height);
    cr.line_to(0.0, height);
    cr.close_path();
}

/// Just the crest of [`wave`], left open, for the light along it.
fn wave_crest(cr: &cairo::Context, width: f64, surface: f64, swell: f64, phase: f64) {
    let steps = (width / 2.0).ceil().max(8.0) as usize;
    // About a wave and a half across the card, whatever its width.
    let period = width / 1.5;
    for step in 0..=steps {
        let x = width * step as f64 / steps as f64;
        let y = surface + swell * (std::f64::consts::TAU * x / period + phase * std::f64::consts::PI).sin();
        if step == 0 {
            cr.move_to(x, y);
        } else {
            cr.line_to(x, y);
        }
    }
}

/// Claude: how much of the budget is spent, as a ring or as a card.
///
/// A tile one icon wide is a square of ember with a ring round the share
/// and the share written in it — the shape a crowded bar can still read at a
/// glance. Any wider and it is a card of dark glass: the mark on the left, the
/// share large, what it is a share of small under it, and the pill along the
/// bottom.
///
/// With no budget there is no share, and the card says what was spent instead
/// — `2.4M` — with no ring and no pill. A ring or a pill with nothing to
/// measure would be a percentage the tile made up.
///
/// Like water, the card brings its own colours: orange in the dark reads on
/// every desktop, and the words on it are white under every theme.
fn draw_claude(
    claude: &doca_ipc::Claude,
    look: Look,
    area: &gtk::DrawingArea,
    cr: &cairo::Context,
    width: f64,
    height: f64,
) {
    let room = claude_card(cr, claude, look.level, look.icon_size, width, height, composited(area));
    let (number, under) = claude_says(claude, look.level, room.square);
    let ink = gdk::RGBA::WHITE;

    // The number at the size the shape gives it, made smaller only if it
    // would not otherwise fit — a ring round `270%` must not cut the `%` off.
    let share = if room.square { 0.25 } else { 0.34 };
    let mut size = text_size(look.icon_size, share);
    let mut big = line(area, &number, size, true, room.width);
    let wide = big.pixel_extents().1.width() as f64;
    if wide > room.width && wide > 0.0 {
        size = (size * room.width / wide).max(7.0);
        big = line(area, &number, size, true, room.width);
    }
    let small = under.map(|words| line(area, &words, text_size(look.icon_size, 0.17), false, room.width));
    if !room.centred {
        big.set_alignment(pango::Alignment::Left);
        if let Some(small) = &small {
            small.set_alignment(pango::Alignment::Left);
        }
    }

    let big_height = big.pixel_extents().1.height() as f64;
    let small_height = small.as_ref().map_or(0.0, |small| small.pixel_extents().1.height() as f64);
    let stack = big_height + small_height;
    let y = room.y + ((room.height - stack) / 2.0).max(0.0);
    paint(cr, &big, room.x, y, ink);
    if let Some(small) = &small {
        paint(cr, small, room.x, y + big_height, faded(ink, CLAUDE_QUIET));
    }
}

/// What a Claude card writes: its number, and the small line when there is
/// room for one.
///
/// The small line is the first thing a crowded bar loses — the wide card
/// keeps it only whole, and the square only while it is not the bare value
/// a minimal bar asks for. The number never goes.
fn claude_says(claude: &doca_ipc::Claude, level: Level, square: bool) -> (String, Option<String>) {
    let under = claude_under(claude);
    let keeps = if square {
        level != Level::Minimal
    } else {
        level == Level::Full
    };
    (claude.label.clone(), (keeps && !under.is_empty()).then_some(under))
}

/// The small line: what the number is of, and when the window turns over.
///
/// `turns` is said as the daemon says it, `until 20:30`, and only when the
/// detail has not already said it — a session's detail ends `until 20:30`
/// on its own. A window that rolls has no `turns`, and nothing is made up
/// for it: a countdown is #76's, once the daemon knows when the limit resets.
fn claude_under(claude: &doca_ipc::Claude) -> String {
    let (detail, turns) = (claude.detail.trim(), claude.turns.trim());
    if turns.is_empty() || detail.contains(turns) {
        detail.to_string()
    } else if detail.is_empty() {
        format!("until {turns}")
    } else {
        format!("{detail} \u{b7} until {turns}")
    }
}

/// Where a Claude card has left room for its words, once it is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Words {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    /// In the middle of a square, or flush left beside the mark.
    centred: bool,
    /// Whether the card is the square one, with the ring.
    square: bool,
}

/// Everything on a Claude card but the words: the glass, the ring or the
/// pill, the mark, and the rim it wears near the end of its budget.
///
/// Kept apart from the words because it is all of the card that is shape
/// rather than text, and shape can be checked by counting pixels with no
/// display to lay text out on.
fn claude_card(
    cr: &cairo::Context,
    claude: &doca_ipc::Claude,
    level: Level,
    icon_size: i32,
    width: f64,
    height: f64,
    composited: bool,
) -> Words {
    let icon = icon_size as f64;
    let corner = (icon * 0.22).max(3.0).min(height / 2.0);
    let measured = claude.progress != NO_PROGRESS;
    let fraction = claude.progress.clamp(0.0, 1.0);
    let fill = if claude.near { claude_hot() } else { claude_orange() };
    // A tile one icon wide — which is what a minimal bar gives a wide one —
    // is too narrow for the mark and a number side by side.
    let square = width < height * 1.4;
    let glass = if square { CLAUDE_EMBER } else { CLAUDE_SMOKE };
    let (x0, y0, w, h) = card(cr, width, height, corner, glass, composited, |_, _, _| {});
    let pad = (icon * 0.14).max(3.0);

    let words = if square {
        claude_square(cr, claude, level, icon, (x0, y0, w, h), measured, fraction, fill)
    } else {
        // The pill along the bottom, under the words and clear of the mark.
        let diameter = (h * 0.38).min(icon * 0.40);
        let left = x0 + pad + diameter + pad * 0.8;
        let right = x0 + w - pad;
        // A compact card gives its number the room the mark would take.
        let marked = level == Level::Full || right - left >= icon * 0.8;
        let left = if marked { left } else { x0 + pad };
        if marked {
            spark(cr, x0 + pad + diameter / 2.0, y0 + h / 2.0, diameter / 2.0, fill);
        }
        let mut bottom = y0 + h - pad * 0.5;
        if measured {
            let thickness = bar_height(icon_size);
            let y = y0 + h - pad - thickness;
            groove(cr, left, y, (right - left).max(0.0), thickness, fraction, claude_track(), fill);
            bottom = y - GAP;
        }
        let top = y0 + pad * 0.5;
        Words {
            x: left,
            y: top,
            width: (right - left).max(0.0),
            height: (bottom - top).max(0.0),
            centred: false,
            square: false,
        }
    };

    // Near the end of the budget the card says so in its own rim: the wash
    // `mark` puts behind the tile is under the glass, where nobody sees it.
    if claude.near {
        let _ = cr.save();
        cr.translate(x0, y0);
        rounded(cr, 0.75, 0.75, w - 1.5, h - 1.5, corner);
        let hot = claude_hot();
        cr.set_source_rgba(hot.red(), hot.green(), hot.blue(), 1.0);
        cr.set_line_width(1.5);
        let _ = cr.stroke();
        let _ = cr.restore();
    }
    words
}

/// The square card: a ring round the share, or the mark over the spend when
/// there is no share to ring.
#[allow(clippy::too_many_arguments)]
fn claude_square(
    cr: &cairo::Context,
    claude: &doca_ipc::Claude,
    level: Level,
    icon: f64,
    (x0, y0, w, h): (f64, f64, f64, f64),
    measured: bool,
    fraction: f64,
    fill: gdk::RGBA,
) -> Words {
    let inset = (icon * 0.08).max(2.0);
    // The small line under the ring, when the bar has not asked for the bare
    // value and the square is tall enough to keep the ring readable over it.
    let line_room = text_size(icon as i32, 0.17) * 1.3;
    let under = claude_says(claude, level, true).1.is_some() && h - line_room >= icon * 0.7;
    let lower = if under { line_room } else { 0.0 };
    let (cx, cy) = (x0 + w / 2.0, y0 + (h - lower) / 2.0);

    if !measured {
        let diameter = ((h - lower) * 0.32).min(w * 0.4);
        spark(cr, cx, y0 + inset + diameter / 2.0 + 1.0, diameter / 2.0, fill);
        let top = y0 + inset + diameter + GAP;
        return Words {
            x: x0 + inset,
            y: top,
            width: (w - inset * 2.0).max(0.0),
            height: (y0 + h - inset * 0.5 - top).max(0.0),
            centred: true,
            square: true,
        };
    }

    let thickness = (icon * 0.075).max(2.5);
    let radius = ((w.min(h - lower)) / 2.0 - thickness / 2.0 - inset).max(1.0);
    ring(cr, cx, cy, radius, thickness, fraction, claude_track(), fill);
    // The number goes in the ring; the small line, if any, under it.
    let inner = ((radius - thickness / 2.0) * 2.0 * 0.92).max(0.0);
    let (y, height) = if under {
        (cy - inner / 2.0, (y0 + h - inset * 0.5) - (cy - inner / 2.0))
    } else {
        (cy - inner / 2.0, inner)
    };
    Words {
        x: if under { x0 + inset } else { cx - inner / 2.0 },
        y,
        width: if under { (w - inset * 2.0).max(0.0) } else { inner },
        height: height.max(0.0),
        centred: true,
        square: true,
    }
}

/// Claude's mark: a six-armed asterisk with round ends, drawn as a path.
///
/// Deliberately a generic spark of the kind a typeface already has (✱), and
/// not Anthropic's logo. The logo is theirs, and a tile on somebody's dock is
/// not something they made; where it was in doubt, the issue says to draw a
/// glyph of our own, and this is it.
fn spark(cr: &cairo::Context, cx: f64, cy: f64, radius: f64, colour: gdk::RGBA) {
    let thickness = (radius * 0.42).max(1.5);
    let reach = (radius - thickness / 2.0).max(0.5);
    let _ = cr.save();
    cr.new_path();
    cr.set_line_width(thickness);
    cr.set_line_cap(cairo::LineCap::Round);
    for arm in 0..6 {
        let angle = arm as f64 * std::f64::consts::PI / 3.0 - std::f64::consts::FRAC_PI_2;
        cr.move_to(cx, cy);
        cr.line_to(cx + reach * angle.cos(), cy + reach * angle.sin());
    }
    cr.set_source_rgba(colour.red(), colour.green(), colour.blue(), colour.alpha());
    let _ = cr.stroke();
    let _ = cr.restore();
}

/// The square card's glass: ember, deep orange going to nearly brown.
const CLAUDE_EMBER: Glass = Glass {
    from: (0.42, 0.17, 0.07),
    to: (0.19, 0.07, 0.04),
    rim: (0.30, 0.06),
    lifted: false,
};

/// The wide card's glass: smoke, nearly black, with a rim light enough to
/// stand the card off a dark bar.
const CLAUDE_SMOKE: Glass = Glass {
    from: (0.17, 0.16, 0.16),
    to: (0.08, 0.08, 0.09),
    rim: (0.34, 0.10),
    lifted: false,
};

/// How bright the small line is against the number, which is white.
const CLAUDE_QUIET: f64 = 0.74;

/// The orange the ring, the pill and the mark are drawn in.
fn claude_orange() -> gdk::RGBA {
    gdk::RGBA::new(0.99, 0.60, 0.31, 1.0)
}

/// The orange gone hot, for a budget nearly spent: redder, so it reads as a
/// warning even to an eye that does not compare it with the other.
fn claude_hot() -> gdk::RGBA {
    gdk::RGBA::new(1.0, 0.33, 0.24, 1.0)
}

/// The groove the ring and the pill run in.
fn claude_track() -> gdk::RGBA {
    gdk::RGBA::new(1.0, 1.0, 1.0, 0.16)
}

/// A note, on the paper it was written on.
///
/// The one tile whose colour comes from the note and not from the theme, and
/// the one whose shape is the thing it stands for rather than a decision
/// about room. A post-it is yellow because post-its are, not because the
/// desktop is dark.
///
/// Which means the ink cannot come from the theme either. Writing a note in
/// `midnight`'s pale grey on a yellow square is a blur, so the ink is picked
/// against the paper: every one of these papers is light, so the ink is the
/// one dark that reads on all six.
fn draw_note(
    note: &doca_ipc::Note,
    look: Look,
    area: &gtk::DrawingArea,
    cr: &cairo::Context,
    width: f64,
    height: f64,
) {
    let paper = paper_of(&note.colour);
    let corner = (look.icon_size as f64 * 0.12).max(2.0);
    rounded(cr, 0.0, 0.0, width, height, corner);
    cr.set_source_rgb(paper.0, paper.1, paper.2);
    let _ = cr.fill();

    let text = note.text.trim();
    if text.is_empty() {
        // An empty note says it is empty rather than drawing nothing, which
        // is what `note::split` used to do in the daemon and is still the
        // right answer — a blank square reads as a tile that is broken.
        let layout = line(area, "empty", text_size(look.icon_size, 0.20), false, width);
        let (_, logical) = layout.pixel_extents();
        paint(
            cr,
            &layout,
            0.0,
            (height - logical.height() as f64) / 2.0,
            faded(note_ink(), 0.55),
        );
        return;
    }

    // As many lines as the square has room for, and the rest is the panel's.
    let layout = line(area, text, text_size(look.icon_size, 0.19), false, width - 4.0);
    layout.set_wrap(pango::WrapMode::WordChar);
    layout.set_height((height as i32 - 6) * pango::SCALE);
    layout.set_ellipsize(pango::EllipsizeMode::End);
    layout.set_alignment(pango::Alignment::Left);
    let (_, logical) = layout.pixel_extents();
    let y = ((height - logical.height() as f64) / 2.0).max(3.0);
    paint(cr, &layout, 2.0, y, note_ink());
}

/// The calendar: the date as a leaf off a desk calendar, and what is next
/// beside it.
///
/// The date is the one thing that is always true, so it is what a crowded
/// bar keeps. Then the next thing's time, which is the glance; then the
/// list, which is the reading.
fn draw_calendar(
    calendar: &doca_ipc::Calendar,
    look: Look,
    area: &gtk::DrawingArea,
    cr: &cairo::Context,
    width: f64,
    height: f64,
) {
    let icon = look.icon_size as f64;
    let column = if look.level == Level::Minimal {
        width
    } else {
        (icon * 0.72).min(width * 0.45)
    };

    // The day of the week in the accent, the number under it in the label
    // ink: the leaf of a desk calendar, which is the oldest way there is of
    // saying "today" at a glance.
    let weekday = line(area, &calendar.weekday.to_uppercase(), text_size(look.icon_size, 0.18), true, column);
    let day = line(area, &calendar.day.to_string(), text_size(look.icon_size, 0.40), true, column);
    let weekday_height = weekday.pixel_extents().1.height() as f64;
    let day_height = day.pixel_extents().1.height() as f64;
    let y = ((height - weekday_height - day_height) / 2.0).max(0.0);
    paint(cr, &weekday, 0.0, y, look.palette.accent);
    paint(cr, &day, 0.0, y + weekday_height, look.palette.label);

    if look.level == Level::Minimal {
        return;
    }

    let gap = (icon * 0.10).max(3.0);
    // A hairline between the leaf and the list, in the groove colour.
    let track = look.palette.track;
    cr.set_source_rgba(track.red(), track.green(), track.blue(), track.alpha());
    cr.rectangle(column + gap / 2.0, height * 0.18, 1.0, height * 0.64);
    let _ = cr.fill();

    let left = column + gap;
    let room = (width - left).max(0.0);
    if room <= 0.0 {
        return;
    }
    let size = text_size(look.icon_size, 0.19);

    // Nothing to list is said, in the quieter ink, and so is a calendar that
    // could not be read — in its own words, because "nothing ahead" over a
    // calendar nobody could open would be a lie.
    let quiet = if !calendar.missing.is_empty() {
        Some(calendar.missing.as_str())
    } else if calendar.upcoming.is_empty() {
        Some("nothing ahead")
    } else {
        None
    };
    if let Some(words) = quiet {
        let layout = line(area, words, size, false, room);
        layout.set_alignment(pango::Alignment::Left);
        layout.set_wrap(pango::WrapMode::WordChar);
        layout.set_single_paragraph_mode(false);
        layout.set_height((height as i32) * pango::SCALE);
        let text_height = layout.pixel_extents().1.height() as f64;
        paint(cr, &layout, left, ((height - text_height) / 2.0).max(0.0), look.palette.detail);
        return;
    }

    // Compact keeps the next thing's time and nothing else: at 1.6 icons
    // there is room for "19:00" and not for what happens at it.
    let shown: Vec<pango::Layout> = if look.level == Level::Compact {
        let next = &calendar.upcoming[0];
        let layout = line(area, &next.when, text_size(look.icon_size, 0.24), true, room);
        layout.set_alignment(pango::Alignment::Left);
        vec![layout]
    } else {
        let probe = line(area, "0", size, false, room);
        let line_height = probe.pixel_extents().1.height().max(1) as f64;
        let fits = ((height * 0.9) / line_height).floor().max(1.0) as usize;
        calendar
            .upcoming
            .iter()
            .take(fits.min(UPCOMING_LINES))
            .map(|up| {
                let layout = line(area, "", size, false, room);
                layout.set_alignment(pango::Alignment::Left);
                layout.set_markup(&format!(
                    "<span font_features=\"{TABULAR}\"><span weight=\"semibold\">{}</span> {}</span>",
                    glib::markup_escape_text(&up.when),
                    glib::markup_escape_text(&up.title)
                ));
                layout
            })
            .collect()
    };

    let heights: Vec<f64> = shown.iter().map(|l| l.pixel_extents().1.height() as f64).collect();
    let mut y = ((height - heights.iter().sum::<f64>()) / 2.0).max(0.0);
    for (index, (layout, line_height)) in shown.iter().zip(heights).enumerate() {
        // What is happening now is in the accent; what is still to come in
        // the label ink, the first of it brightest.
        let ink = if calendar.upcoming[index].now {
            look.palette.accent
        } else if index == 0 {
            look.palette.label
        } else {
            look.palette.detail
        };
        paint(cr, layout, left, y, ink);
        y += line_height;
    }
}

/// The most things a calendar tile lists, however tall it is.
const UPCOMING_LINES: usize = 3;

/// The ink a note is written in, on any of its papers.
fn note_ink() -> gdk::RGBA {
    gdk::RGBA::new(0.16, 0.14, 0.10, 1.0)
}

/// The six papers, as the colours they are.
///
/// Light, every one of them, which is what lets a single dark ink serve all
/// six — `a_note_is_readable_on_every_paper_it_offers` holds that rather than
/// trusting it.
fn paper_of(colour: &str) -> (f64, f64, f64) {
    use doca_ipc::note_colour as paper;
    match doca_ipc::note_colour::resolve(colour) {
        paper::PINK => (0.98, 0.78, 0.84),
        paper::BLUE => (0.76, 0.87, 0.97),
        paper::GREEN => (0.79, 0.93, 0.76),
        paper::PURPLE => (0.86, 0.80, 0.95),
        paper::RED => (0.98, 0.76, 0.72),
        _ => (0.99, 0.91, 0.56),
    }
}

/// A rectangle with its corners taken off, as a path.
fn rounded(cr: &cairo::Context, x: f64, y: f64, width: f64, height: f64, radius: f64) {
    let radius = radius.min(width / 2.0).min(height / 2.0);
    let half = std::f64::consts::FRAC_PI_2;
    cr.new_path();
    cr.arc(x + width - radius, y + radius, radius, -half, 0.0);
    cr.arc(x + width - radius, y + height - radius, radius, 0.0, half);
    cr.arc(x + radius, y + height - radius, radius, half, half * 2.0);
    cr.arc(x + radius, y + radius, radius, half * 2.0, half * 3.0);
    cr.close_path();
}

/// What a [`card`] is made of.
///
/// A card brings its own colours, the way a note brings its paper: what is
/// drawn inside it is drawn against the glass and not against the theme, so
/// it reads the same on every desktop.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Glass {
    /// The glass at the top left, and at the bottom right.
    from: (f64, f64, f64),
    to: (f64, f64, f64),
    /// How white the rim is at the top, and at the bottom.
    rim: (f64, f64),
    /// Whether the card stands off the bar on a soft shadow.
    lifted: bool,
}

/// Water's glass: deep teal going to night blue, the way the card in the
/// design does, so the water has something dark to be bright against.
const WATER_GLASS: Glass = Glass {
    from: (0.07, 0.20, 0.22),
    to: (0.09, 0.13, 0.25),
    rim: (0.22, 0.06),
    lifted: false,
};

/// A card of glass: a rounded pane in a gradient, with a hairline rim that is
/// brighter at the top, the way light falls on glass from above.
///
/// `inside` draws into the glass — clipped to it, so a wave or a fill cannot
/// spill over the corners — and is handed the card's own width and height,
/// with the card's top left as its origin. The rim goes on after it, so it is
/// never painted over.
///
/// A lifted card stands on a shadow, and only where something composites the
/// bar: without a compositor there is no alpha to blur into, and a shadow
/// would be a grey smear on the bar. The shadow has to fit in the tile like
/// everything else does, so a lifted card is that much smaller than the tile,
/// and the rectangle it took is returned for whatever is drawn over it.
fn card(
    cr: &cairo::Context,
    width: f64,
    height: f64,
    corner: f64,
    glass: Glass,
    composited: bool,
    inside: impl FnOnce(&cairo::Context, f64, f64),
) -> (f64, f64, f64, f64) {
    let lift = if glass.lifted && composited {
        shadow_of(width, height, corner)
    } else {
        None
    };
    let (x, y, card_width, card_height) = match &lift {
        Some((blur, _)) => (*blur, blur * 0.5, width - blur * 2.0, height - blur * 2.0),
        None => (0.0, 0.0, width, height),
    };
    if let Some((_, shadow)) = lift {
        let _ = cr.save();
        cr.rectangle(0.0, 0.0, width, height);
        cr.clip();
        cr.set_source_rgba(0.0, 0.0, 0.0, SHADOW_ALPHA);
        let _ = cr.mask_surface(&shadow, 0.0, 0.0);
        let _ = cr.restore();
    }

    let _ = cr.save();
    cr.translate(x, y);
    let _ = cr.save();
    rounded(cr, 0.0, 0.0, card_width, card_height, corner);
    let pane = cairo::LinearGradient::new(0.0, 0.0, card_width, card_height);
    pane.add_color_stop_rgb(0.0, glass.from.0, glass.from.1, glass.from.2);
    pane.add_color_stop_rgb(1.0, glass.to.0, glass.to.1, glass.to.2);
    let _ = cr.set_source(&pane);
    let _ = cr.fill_preserve();
    // `clip` consumes the path, so what `inside` draws are its own shapes and
    // not the card's rectangle added to them.
    cr.clip();
    inside(cr, card_width, card_height);
    let _ = cr.restore();

    rounded(cr, 0.5, 0.5, card_width - 1.0, card_height - 1.0, corner);
    let rim = cairo::LinearGradient::new(0.0, 0.0, 0.0, card_height);
    rim.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, glass.rim.0);
    rim.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, glass.rim.1);
    let _ = cr.set_source(&rim);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    let _ = cr.restore();

    (x, y, card_width, card_height)
}

/// Whether the screen this area is on composites, which is what a shadow
/// needs to be a shadow.
fn composited(area: &gtk::DrawingArea) -> bool {
    area.screen().is_some_and(|screen| screen.is_composited())
}

/// How dark a card's shadow is where it is darkest.
const SHADOW_ALPHA: f64 = 0.35;

/// How far a shadow spreads past its card, as a share of the tile's height.
const SHADOW_SPREAD: f64 = 0.08;

thread_local! {
    /// Shadows already blurred, by the size they were blurred for.
    ///
    /// Blurring is the one expensive thing a tile draws, and a tile is
    /// redrawn at every frame of an animation at a size that does not change.
    static SHADOWS: RefCell<HashMap<(i32, i32, i32), cairo::ImageSurface>> = RefCell::new(HashMap::new());
}

/// The shadow under a lifted card the size of this tile, and how far it
/// spreads — the margin the card has to leave for it.
///
/// Cairo has no blur, so this is one: the card's shape in solid alpha,
/// dropped a little, then three passes of a box blur, which is as near a
/// gaussian as the eye can tell. Painted once per size and kept.
fn shadow_of(width: f64, height: f64, corner: f64) -> Option<(f64, cairo::ImageSurface)> {
    let blur = (height * SHADOW_SPREAD).round().max(2.0);
    let (across, down) = (width.ceil() as i32, height.ceil() as i32);
    if across <= blur as i32 * 2 || down <= blur as i32 * 2 {
        return None;
    }
    let key = (across, down, (corner * 4.0).round() as i32);
    if let Some(shadow) = SHADOWS.with(|shadows| shadows.borrow().get(&key).cloned()) {
        return Some((blur, shadow));
    }

    let mut surface = cairo::ImageSurface::create(cairo::Format::A8, across, down).ok()?;
    {
        let cr = cairo::Context::new(&surface).ok()?;
        rounded(&cr, blur, blur, width - blur * 2.0, height - blur * 2.0, corner);
        cr.set_source_rgba(0.0, 0.0, 0.0, 1.0);
        let _ = cr.fill();
    }
    surface.flush();
    {
        let stride = surface.stride() as usize;
        let mut data = surface.data().ok()?;
        let radius = (blur / 3.0).round().max(1.0) as usize;
        for _ in 0..3 {
            box_blur(&mut data, stride, across as usize, down as usize, radius);
        }
    }
    surface.mark_dirty();

    SHADOWS.with(|shadows| {
        let mut shadows = shadows.borrow_mut();
        // A bar has a handful of tile sizes; anything past this is sizes
        // gone by, from an icon size the bar no longer has.
        if shadows.len() >= 32 {
            shadows.clear();
        }
        shadows.insert(key, surface.clone());
    });
    Some((blur, surface))
}

/// One pass of a box blur over a single channel, across and then down.
fn box_blur(data: &mut [u8], stride: usize, width: usize, height: usize, radius: usize) {
    let span = (radius * 2 + 1) as u32;
    let mut line = vec![0u8; width.max(height)];
    let blur_line = |read: &dyn Fn(usize) -> u8, length: usize, out: &mut [u8]| {
        let mut sum: u32 = 0;
        let at = |i: isize| -> u32 {
            if i < 0 || i as usize >= length {
                0
            } else {
                read(i as usize) as u32
            }
        };
        for i in -(radius as isize)..=(radius as isize) {
            sum += at(i);
        }
        for (i, slot) in out.iter_mut().enumerate().take(length) {
            *slot = (sum / span) as u8;
            sum += at(i as isize + radius as isize + 1);
            sum -= at(i as isize - radius as isize);
        }
    };
    for row in 0..height {
        let start = row * stride;
        let source = data[start..start + width].to_vec();
        blur_line(&|i| source[i], width, &mut line);
        data[start..start + width].copy_from_slice(&line[..width]);
    }
    for column in 0..width {
        let source: Vec<u8> = (0..height).map(|row| data[row * stride + column]).collect();
        blur_line(&|i| source[i], height, &mut line);
        for row in 0..height {
            data[row * stride + column] = line[row];
        }
    }
}

/// A ring round a point: the whole circle faintly, the share of it brightly,
/// from twelve o'clock round the way a clock goes.
///
/// The ends are round, like a pill's, so a little is a dot and not a sliver.
/// Nothing at all is no arc, not a dot at twelve — an empty ring says it is
/// empty — and a whole one is a closed circle, so its two round ends do not
/// overlap into a bump at the top.
#[allow(clippy::too_many_arguments)]
fn ring(
    cr: &cairo::Context,
    cx: f64,
    cy: f64,
    radius: f64,
    thickness: f64,
    fraction: f64,
    track: gdk::RGBA,
    fill: gdk::RGBA,
) {
    use std::f64::consts::{FRAC_PI_2, TAU};

    let _ = cr.save();
    cr.set_line_width(thickness);
    cr.new_path();
    cr.arc(cx, cy, radius, 0.0, TAU);
    cr.set_source_rgba(track.red(), track.green(), track.blue(), track.alpha());
    let _ = cr.stroke();

    let fraction = fraction.clamp(0.0, 1.0);
    if fraction > 0.0 {
        cr.new_path();
        let start = -FRAC_PI_2;
        cr.arc(cx, cy, radius, start, start + TAU * fraction);
        if fraction < 1.0 {
            cr.set_line_cap(cairo::LineCap::Round);
        }
        cr.set_source_rgba(fill.red(), fill.green(), fill.blue(), fill.alpha());
        let _ = cr.stroke();
    }
    let _ = cr.restore();
}

/// Music: the cover, and the track written over it.
///
/// The cover is the tile. Everything else here exists to keep the words on
/// top of it readable, which is the hard part — a cover is somebody else's
/// picture and can be white, black or a saturated orange, and the text has to
/// hold against all three.
///
/// The answer is a veil, not a colour chosen per cover. A gradient from
/// nothing at the top to nearly opaque at the bottom, with the words in the
/// bottom third, means the pixel behind any letter is dark whatever the cover
/// is — and dark by an amount this file decides rather than by an amount the
/// album decided. `a_cover_of_any_colour_keeps_its_words_readable` measures
/// exactly that, against a white cover and a black one.
///
/// So the words are white here even under `paper`, which is the one place in
/// the bar that ignores the theme's own ink. A photo with a scrim over it is
/// a photo with a scrim over it on every desktop.
fn draw_music(
    music: &doca_ipc::Music,
    look: Look,
    area: &gtk::DrawingArea,
    cr: &cairo::Context,
    width: f64,
    height: f64,
) {
    let density = area.scale_factor();
    let cover = (!music.art.is_empty())
        .then(|| {
            let dense = density as f64;
            crate::dock::scaled_to_fill(&music.art, width * dense, height * dense)
        })
        .flatten();

    if let Some(cover) = &cover {
        let _ = cr.save();
        // Centred and cropped rather than squashed: a square cover in a wide
        // tile has to lose its edges, not its proportions.
        let x = (width - (cover.width() / density) as f64) / 2.0;
        let y = (height - (cover.height() / density) as f64) / 2.0;
        cr.rectangle(0.0, 0.0, width, height);
        cr.clip();
        crate::dock::paint_dense(cr, cover, (x, y), density, 1.0);
        let _ = cr.restore();
        veil(cr, width, height);
    }

    // Written by the bar and not by the daemon, because the bar is the one
    // that knows how much room there is to say it in.
    let (title, under) = if music.title.is_empty() && music.artist.is_empty() {
        ("\u{2014}".to_string(), "nothing playing".to_string())
    } else {
        (
            if music.title.is_empty() {
                "unknown".to_string()
            } else {
                music.title.clone()
            },
            if music.artist.is_empty() {
                music.player.clone()
            } else {
                music.artist.clone()
            },
        )
    };

    let (ink, quiet) = if cover.is_some() {
        (ON_A_COVER, faded(ON_A_COVER, 0.78))
    } else {
        (look.palette.label, look.palette.detail)
    };

    let name = line(area, &title, text_size(look.icon_size, 0.28), true, width);
    let who = line(area, &under, text_size(look.icon_size, 0.21), false, width);
    let name_height = name.pixel_extents().1.height() as f64;
    let who_height = who.pixel_extents().1.height() as f64;

    // Sat at the bottom when there is a cover, because that is where the veil
    // is; centred when there is not, because then there is nothing to hide
    // behind and the tile is just two lines.
    let mut y = if cover.is_some() {
        height - name_height - who_height - GAP
    } else {
        ((height - name_height - who_height) / 2.0).max(0.0)
    };
    paint(cr, &name, 0.0, y, ink);
    y += name_height;
    paint(cr, &who, 0.0, y, quiet);
}

/// The ink the words take when they are sitting on somebody else's picture.
const ON_A_COVER: gdk::RGBA = gdk::RGBA::WHITE;

fn faded(colour: gdk::RGBA, alpha: f64) -> gdk::RGBA {
    gdk::RGBA::new(colour.red(), colour.green(), colour.blue(), alpha)
}

/// The scrim that makes a cover safe to write on.
///
/// Nothing at the top, so the picture is still a picture, and nearly opaque
/// where the words are. The whole of the contrast guarantee is here: not that
/// the text is bright enough, but that whatever is under it has been made
/// dark enough first.
fn veil(cr: &cairo::Context, width: f64, height: f64) {
    let shade = cairo::LinearGradient::new(0.0, 0.0, 0.0, height);
    shade.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.0);
    shade.add_color_stop_rgba(0.45, 0.0, 0.0, 0.0, 0.38);
    shade.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, 0.88);
    let _ = cr.set_source(&shade);
    cr.rectangle(0.0, 0.0, width, height);
    let _ = cr.fill();
}

/// The gap between two things in a tile, and the one tunable in here.
const GAP: f64 = 2.0;

/// How tall a progress bar is: the three pixels the stylesheet drew, grown
/// with the icons so it does not vanish at 96px.
fn bar_height(icon_size: i32) -> f64 {
    (icon_size as f64 * 0.0625).round().max(2.0)
}

/// A font size in pixels, as a share of the icon size.
///
/// In pixels rather than points because everything else about a tile is: a
/// point size would follow the desktop's font scaling and leave the text the
/// wrong size for the tile it is in.
fn text_size(icon_size: i32, share: f64) -> f64 {
    (icon_size as f64 * share).max(7.0)
}

/// One line of text, laid out to the width it has and no wider.
fn line(
    area: &gtk::DrawingArea,
    text: &str,
    size: f64,
    bold: bool,
    width: f64,
) -> pango::Layout {
    let layout = area.create_pango_layout(Some(text));
    let mut font = pango::FontDescription::new();
    font.set_absolute_size(size * pango::SCALE as f64);
    if bold {
        font.set_weight(pango::Weight::Semibold);
    }
    layout.set_font_description(Some(&font));
    layout.set_width(width.max(0.0) as i32 * pango::SCALE);
    layout.set_alignment(pango::Alignment::Center);
    // A track title is longer than any tile, and a tile that grew to fit one
    // would shove its neighbours along the bar.
    layout.set_ellipsize(pango::EllipsizeMode::End);
    layout.set_single_paragraph_mode(true);
    // Every digit the same width, so a number that changes in place — a
    // clock, a countdown, a percentage — changes its digits and not its
    // width. Letters are not affected.
    let attributes = pango::AttrList::new();
    attributes.insert(pango::AttrFontFeatures::new(TABULAR));
    layout.set_attributes(Some(&attributes));
    layout
}

/// The OpenType feature for digits of one width.
///
/// Markup set on a [`line`] replaces its attributes, so markup has to ask
/// for it again in a span of its own.
const TABULAR: &str = "tnum";

fn paint(cr: &cairo::Context, layout: &pango::Layout, x: f64, y: f64, colour: gdk::RGBA) {
    cr.set_source_rgba(colour.red(), colour.green(), colour.blue(), colour.alpha());
    cr.move_to(x, y);
    pangocairo::functions::show_layout(cr, layout);
}

/// A bar in its groove: the whole width faintly, the share of it brightly.
///
/// Both are pills, rounded at the ends by half their thickness. The share is
/// never shorter than it is thick, so that a little is a dot and not a
/// sliver, and is clipped to the groove so its round end sits inside it.
///
/// The two colours rather than a [`Palette`], because a card brings its own:
/// Claude's pill is orange on its glass whatever the theme's bar is.
#[allow(clippy::too_many_arguments)]
fn groove(
    cr: &cairo::Context,
    x: f64,
    y: f64,
    width: f64,
    thickness: f64,
    fraction: f64,
    track: gdk::RGBA,
    fill: gdk::RGBA,
) {
    let radius = thickness / 2.0;
    cr.set_source_rgba(track.red(), track.green(), track.blue(), track.alpha());
    rounded(cr, x, y, width, thickness, radius);
    let _ = cr.fill_preserve();

    if fraction <= 0.0 {
        cr.new_path();
        return;
    }
    let _ = cr.save();
    cr.clip();
    cr.set_source_rgba(fill.red(), fill.green(), fill.blue(), fill.alpha());
    rounded(cr, x, y, (width * fraction).max(thickness), thickness, radius);
    let _ = cr.fill();
    let _ = cr.restore();
}

/// What one [`Shelf::show`] did: tiles reused, tiles built, tiles dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Reconciled {
    pub kept: usize,
    pub made: usize,
    pub gone: usize,
}

/// Every widget tile on the bar, and the divider that stands in front of them.
///
/// The bar is rebuilt whenever a window opens, closes or takes focus, and now
/// on every change to the config as well — which means on every frame of a
/// slider being dragged in the preferences window. Tearing the tiles down and
/// building them again each time is a visible flicker at that rate, and it
/// throws away the one thing a tile is: a `GtkWidget` with a style class, a
/// drawing area and an X window of its own.
///
/// So this reconciles by id, the way [`crate::row::Row::fill`] does for the
/// icons. Drawing the tiles instead of assembling them changed what is inside
/// one and nothing about that: a tile that comes back is still the same tile,
/// told its new state and its new size rather than remade at them.
#[derive(Default)]
pub struct Shelf {
    /// Built once and kept, so that taking it off the bar when no widget is
    /// shown does not destroy it.
    divider: RefCell<Option<gtk::Separator>>,
    /// The tiles on the bar, in the order they are shown.
    tiles: RefCell<Vec<(String, WidgetTile)>>,
    /// The tiles on their way off it, each still holding a shrinking share
    /// of its place until it has gone.
    leaving: RefCell<Vec<(String, WidgetTile)>>,
    /// Whether the bar has been shown before. The first time is the dock
    /// starting, and tiles that were there all along do not *arrive*.
    shown_before: Cell<bool>,
    /// Whoever moves tiles, handed to every tile the shelf makes.
    grip: RefCell<Option<Grip>>,
    /// The order the tiles stood in when one was picked up, so putting it
    /// back where it was is nothing to write.
    lifted: RefCell<Option<Vec<String>>>,
    /// The room a tile in the hand just left, closing as the room where it
    /// went opens: the neighbour it passed glides across rather than jumps.
    gap: RefCell<Option<Gap>>,
}

/// The room a held tile left when it moved slot, closing on its own clock.
struct Gap {
    filler: gtk::DrawingArea,
}

impl Shelf {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether this widget is one of ours, and must survive a rebuild.
    ///
    /// The rebuild sweeps the bar clear of everything it is about to make
    /// again. What the shelf owns is not made again, so it has to be asked —
    /// and a tile on its way out is still the shelf's until it has gone.
    pub fn owns(&self, child: &gtk::Widget) -> bool {
        if self
            .divider
            .borrow()
            .as_ref()
            .is_some_and(|divider| divider.clone().upcast::<gtk::Widget>() == *child)
        {
            return true;
        }
        // The room a tile in the hand just left, still closing.
        if self
            .gap
            .borrow()
            .as_ref()
            .is_some_and(|gap| gap.filler.clone().upcast::<gtk::Widget>() == *child)
        {
            return true;
        }
        self.tiles
            .borrow()
            .iter()
            .chain(self.leaving.borrow().iter())
            .any(|(_, tile)| tile.root == *child)
    }

    /// Put these widgets on the bar, keeping the tiles already there.
    ///
    /// A tile whose id comes back is the same tile, moved into place and told
    /// its new state rather than remade. One that does not come back shrinks
    /// off the bar where it stood and is dropped once it has gone — which is
    /// when its widget is finally destroyed, because until then this held
    /// the only other reference to it. One that is new grows into the room
    /// that opens for it. Both on the curve the icons use, so that a dock
    /// switched for another is one movement and not a bar emptied and
    /// refilled.
    ///
    /// Order is settled by moving each child to the end in turn, never by
    /// unparenting: taking a widget out of a container and putting it back is
    /// the destroy-and-rebuild this exists to avoid.
    ///
    /// Answers with what it did, because reconciliation is invisible when it
    /// works — a shelf that quietly went back to rebuilding everything looks
    /// exactly like one that did not, and the counts are the only thing that
    /// tells them apart.
    pub fn show(
        &self,
        items: &gtk::Box,
        widgets: &[WidgetState],
        look: Look,
        expand: &Expand,
    ) -> Reconciled {
        self.settle();
        let announcing = self.shown_before.replace(true);
        // A rebuild that lands while a tile is in the hand brings the order
        // the config still has; the hand has not let go yet, so the tiles
        // stay in the order it has made so far.
        let carried;
        let widgets = if self.lifting() {
            carried = in_the_order_of(widgets, &self.order());
            carried.as_slice()
        } else {
            widgets
        };

        // The order the tiles stand in now, leaving ones included: where a
        // tile that is going goes from is worked out from its neighbours.
        let children = items.children();
        let standing = |tile: &WidgetTile| children.iter().position(|child| *child == tile.root);
        let mut before: Vec<(String, bool, WidgetTile)> = self
            .tiles
            .borrow_mut()
            .drain(..)
            .map(|(id, tile)| (id, false, tile))
            .chain(
                self.leaving
                    .borrow_mut()
                    .drain(..)
                    .map(|(id, tile)| (id, true, tile)),
            )
            .collect();
        before.sort_by_key(|(_, _, tile)| standing(tile));
        let previous: Vec<String> = before.iter().map(|(id, _, _)| id.clone()).collect();
        let mut known: HashMap<String, (bool, WidgetTile)> = before
            .into_iter()
            .map(|(id, leaving, tile)| (id, (leaving, tile)))
            .collect();

        let mut did = Reconciled::default();
        let shown: Vec<(String, WidgetTile)> = widgets
            .iter()
            .map(|state| {
                let tile = match known.remove(&state.id) {
                    Some((_, tile)) => {
                        did.kept += 1;
                        tile.relook(look);
                        tile.update(state);
                        tile.stay();
                        tile
                    }
                    None => {
                        did.made += 1;
                        let tile = WidgetTile::new(state, look, expand.clone());
                        tile.grip_with(self.grip.borrow().clone());
                        items.add(&tile.root);
                        if announcing {
                            tile.arrive();
                        }
                        tile
                    }
                };
                (state.id.clone(), tile)
            })
            .collect();

        // Whatever no dock asks for any more goes, from where it stood:
        // after as many of its old neighbours as are still on the bar.
        let staying: Vec<&String> = shown.iter().map(|(id, _)| id).collect();
        let mut leaving: Vec<(usize, usize, String, WidgetTile)> = known
            .into_iter()
            .map(|(id, (was_leaving, tile))| {
                if !was_leaving {
                    did.gone += 1;
                }
                tile.leave();
                let at = previous
                    .iter()
                    .position(|seen| *seen == id)
                    .unwrap_or(previous.len());
                let anchor = previous[..at]
                    .iter()
                    .filter(|seen| staying.contains(seen))
                    .count();
                (anchor, at, id, tile)
            })
            .collect();
        leaving.sort_by_key(|(anchor, at, _, _)| (*anchor, *at));

        // Read back by scripts/check-live.sh, which has no other way to tell
        // a bar that kept its tiles from one that made them again.
        tracing::debug!(
            kept = did.kept,
            made = did.made,
            gone = did.gone,
            "the widget shelf"
        );

        let mut order: Vec<&WidgetTile> = Vec::with_capacity(shown.len() + leaving.len());
        let mut going = leaving.iter().peekable();
        for (index, (_, tile)) in shown.iter().enumerate() {
            while going.peek().is_some_and(|(anchor, ..)| *anchor <= index) {
                order.push(&going.next().expect("peeked").3);
            }
            order.push(tile);
        }
        order.extend(going.map(|(.., tile)| tile));

        if order.is_empty() {
            // Nothing to divide from. A separator with no widgets behind it
            // is a stray line at the end of the bar.
            self.take_divider_down();
        } else {
            let divider = self
                .divider
                .borrow_mut()
                .get_or_insert_with(crate::dock::divider)
                .clone();
            if divider.parent().is_none() {
                items.add(&divider);
            }
            items.reorder_child(&divider, -1);
            for tile in order {
                items.reorder_child(&tile.root, -1);
            }
        }

        *self.tiles.borrow_mut() = shown;
        *self.leaving.borrow_mut() = leaving
            .into_iter()
            .map(|(_, _, id, tile)| (id, tile))
            .collect();
        did
    }

    /// Let go of the tiles that have finished leaving, and of the divider if
    /// nothing is left for it to divide.
    ///
    /// A tile takes itself off the bar on the last frame of its leaving, but
    /// only if there is a frame to take it on: this is for whoever cannot
    /// wait for one — the next `show`, and the bar measuring itself once a
    /// dock switch is over.
    pub fn settle(&self) {
        self.leaving.borrow_mut().retain(|(_, tile)| {
            if tile.life.get().gone() {
                tile.unshelve();
                return false;
            }
            true
        });
        for (_, tile) in self.tiles.borrow().iter().chain(self.leaving.borrow().iter()) {
            tile.pass();
        }
        if self.tiles.borrow().is_empty() && self.leaving.borrow().is_empty() {
            self.take_divider_down();
        }
    }

    fn take_divider_down(&self) {
        if let Some(divider) = self.divider.borrow().as_ref() {
            if let Some(parent) = divider
                .parent()
                .and_then(|parent| parent.downcast::<gtk::Container>().ok())
            {
                parent.remove(divider);
            }
        }
    }

    /// Whether any tile is still arriving or leaving.
    pub fn changing(&self) -> bool {
        self.tiles
            .borrow()
            .iter()
            .chain(self.leaving.borrow().iter())
            .any(|(_, tile)| tile.life.get().moving())
    }

    /// How much wider the shelf would be with every tile at its full size —
    /// the arriving ones arrived and the leaving ones not yet gone. The same
    /// question `Row::unfolding` asks of the icons, and for the same reason.
    pub fn unfolding(&self) -> i32 {
        self.tiles
            .borrow()
            .iter()
            .chain(self.leaving.borrow().iter())
            .map(|(_, tile)| tile.unfolding())
            .sum()
    }

    /// The tiles on the bar and staying, in the order they stand — where the
    /// keyboard can go once it is past the icons. A tile on its way off is
    /// not somewhere to go.
    pub fn on_the_bar(&self) -> Vec<(String, WidgetTile)> {
        self.tiles.borrow().clone()
    }

    /// Say again what every tile is showing, after the bar's `show_all`.
    pub fn refresh(&self, widgets: &[WidgetState]) {
        for state in widgets {
            self.update(state);
        }
    }

    /// One widget's new state, if it has a tile on the bar.
    ///
    /// `false` for a widget the bar is not showing — which is the signal that
    /// a rebuild is needed, rather than something to report as an error.
    pub fn update(&self, state: &WidgetState) -> bool {
        match self
            .tiles
            .borrow()
            .iter()
            .find(|(id, _)| *id == state.id)
        {
            Some((_, tile)) => {
                tile.update(state);
                true
            }
            None => false,
        }
    }

    /// What a widget's panel says while it is being typed into.
    pub fn typing(&self, id: &str, text: Option<&str>) {
        if let Some((_, tile)) = self.tiles.borrow().iter().find(|(tile, _)| *tile == id) {
            tile.typing(text);
        }
    }

    /// Who moves tiles. Every tile on the shelf is told, and every one it
    /// makes from now on.
    pub fn grips(&self, grip: Grip) {
        for (_, tile) in self.tiles.borrow().iter() {
            tile.grip_with(Some(grip.clone()));
        }
        self.grip.replace(Some(grip));
    }

    fn order(&self) -> Vec<String> {
        self.tiles
            .borrow()
            .iter()
            .map(|(id, _)| id.clone())
            .collect()
    }

    fn tile(&self, id: &str) -> Option<WidgetTile> {
        self.tiles
            .borrow()
            .iter()
            .find(|(seen, _)| seen == id)
            .map(|(_, tile)| tile.clone())
    }

    /// Pick a tile up: a picture of it to carry, and a hole where it stood.
    pub fn lift(&self, id: &str) -> Option<gtk::gdk_pixbuf::Pixbuf> {
        let tile = self.tile(id)?;
        let picture = tile.snapshot();
        self.lifted.replace(Some(self.order()));
        tile.hold(true);
        picture
    }

    /// Whether a tile is in the hand.
    pub fn lifting(&self) -> bool {
        self.lifted.borrow().is_some()
    }

    /// Carry the held tile to `x`, along the screen, and move its hole to the
    /// slot it is over: before the first tile whose middle is past `x`.
    ///
    /// The hole moves in one frame and the room follows over `SHUFFLE`: it
    /// grows in its new place as a filler the same width shrinks in the old
    /// one, so the total never changes and the tile passed glides across.
    /// One glide at a time — a second move before the first has finished
    /// finishes it at once — which a hand only ever sees on a very quick
    /// sweep across several tiles. Answers whether the hole moved.
    pub fn carry_to(&self, items: &gtk::Box, id: &str, x: f64) -> bool {
        let Some(held) = self.tile(id) else {
            return false;
        };
        let to = self
            .tiles
            .borrow()
            .iter()
            .filter(|(seen, _)| seen != id)
            .filter_map(|(_, tile)| {
                let (_, left, _) = tile.root.window()?.origin();
                Some(left as f64 + tile.root.allocated_width() as f64 / 2.0)
            })
            .filter(|middle| *middle < x)
            .count();
        let now = self.order().iter().position(|seen| seen == id).unwrap_or(0);
        if to == now {
            return false;
        }

        self.close_gap();
        // The room the tile is leaving, as a filler of the same width where
        // it stood.
        let width = held.area.size_request().0;
        let filler = gtk::DrawingArea::new();
        filler.set_size_request(width, held.area.size_request().1);
        items.add(&filler);
        if let Some(at) = place_of(items, &held.root) {
            items.reorder_child(&filler, at as i32);
        }
        filler.show();

        // And the tile itself, next to the neighbour it now stands beside.
        {
            let mut tiles = self.tiles.borrow_mut();
            let entry = tiles.remove(now);
            tiles.insert(to, entry);
        }
        let (before, after) = {
            let tiles = self.tiles.borrow();
            let root = |at: usize| tiles.get(at).map(|(_, tile)| tile.root.clone());
            (root(to + 1), to.checked_sub(1).and_then(root))
        };
        let mine = place_of(items, &held.root);
        let position = match (before, after, mine) {
            (Some(next), _, Some(mine)) => {
                place_of(items, &next).map(|at| if mine < at { at - 1 } else { at })
            }
            (None, Some(previous), Some(mine)) => {
                place_of(items, &previous).map(|at| if mine < at { at } else { at + 1 })
            }
            _ => None,
        };
        if let Some(position) = position {
            items.reorder_child(&held.root, position as i32);
        }

        held.room.set(0.0);
        held.resize();
        let since = std::time::Instant::now();
        self.gap.replace(Some(Gap {
            filler: filler.clone(),
        }));
        let tile = held.clone();
        filler.add_tick_callback(move |filler, _| {
            let left = motion::glide_left(since.elapsed());
            tile.room.set(1.0 - left);
            tile.resize();
            let width = (tile.declared() as f64 * left).round() as i32;
            filler.set_size_request(width, filler.size_request().1);
            if left > 0.0 {
                return glib::ControlFlow::Continue;
            }
            unpack(filler.upcast_ref());
            glib::ControlFlow::Break
        });
        true
    }

    /// Finish the glide in progress at once: the filler goes and every tile
    /// takes all of its room.
    fn close_gap(&self) {
        if let Some(gap) = self.gap.replace(None) {
            unpack(gap.filler.upcast_ref());
        }
        for (_, tile) in self.tiles.borrow().iter() {
            if tile.room.get() < 1.0 {
                tile.room.set(1.0);
                tile.resize();
            }
        }
    }

    /// Put the held tile down where its hole is. The new order, when it is
    /// not the order it was picked up from.
    pub fn put_down(&self, id: &str) -> Option<Vec<String>> {
        let before = self.lifted.replace(None)?;
        if let Some(tile) = self.tile(id) {
            tile.hold(false);
        }
        let after = self.order();
        (after != before).then_some(after)
    }
}

/// These widgets, the ones in `order` first and in that order, then the rest
/// as they came.
fn in_the_order_of(widgets: &[WidgetState], order: &[String]) -> Vec<WidgetState> {
    let mut placed: Vec<WidgetState> = order
        .iter()
        .filter_map(|id| widgets.iter().find(|state| &state.id == id).cloned())
        .collect();
    placed.extend(
        widgets
            .iter()
            .filter(|state| !order.contains(&state.id))
            .cloned(),
    );
    placed
}

/// Where a child stands in the box, counting every child.
fn place_of(items: &gtk::Box, child: &gtk::Widget) -> Option<usize> {
    items.children().iter().position(|seen| seen == child)
}

/// Off whatever it is packed in, if anything.
fn unpack(widget: &gtk::Widget) {
    if let Some(parent) = widget
        .parent()
        .and_then(|parent| parent.downcast::<gtk::Container>().ok())
    {
        parent.remove(widget);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn every_kind_of_tile_is_said_in_words() {
        let clock = WidgetState::new(
            "time-progress",
            Body::Simple(Simple { label: "63%".to_string(), detail: "of today".to_string(), progress: 0.63, active: false }),
        );
        assert_eq!(spoken(&clock), "Time progress: 63%, of today");
        assert_eq!(spoken(&water(750, 2000)), "Water: 750 of 2000 ml");
        let music = |title: &str, artist: &str| {
            WidgetState::new(
                "music",
                Body::Music(doca_ipc::Music {
                    title: title.to_string(),
                    artist: artist.to_string(),
                    player: "rhythmbox".to_string(),
                    playing: true,
                    art: String::new(),
                }),
            )
        };
        assert_eq!(spoken(&music("Girl from Ipanema", "João Gilberto")), "Music: Girl from Ipanema by João Gilberto");
        assert_eq!(spoken(&music("", "")), "Music: nothing playing");
        let note = |text: &str| {
            WidgetState::new(
                "note",
                Body::Note(doca_ipc::Note { text: text.to_string(), colour: "yellow".to_string() }),
            )
        };
        assert_eq!(spoken(&note("\n  milk\nbread")), "Note: milk", "the first line that says something");
        assert_eq!(spoken(&note("")), "Note: empty");
        let calendar = |upcoming: Vec<doca_ipc::Upcoming>, missing: &str| {
            WidgetState::new(
                "calendar",
                Body::Calendar(doca_ipc::Calendar {
                    weekday: "Sat".to_string(),
                    day: 17,
                    upcoming,
                    missing: missing.to_string(),
                }),
            )
        };
        let dentist = doca_ipc::Upcoming { when: "19:00".to_string(), title: "Dentist".to_string(), now: false };
        let named = crate::panel::name_of("calendar");
        assert_eq!(spoken(&calendar(vec![dentist], "")), format!("{named}: Sat 17, 19:00 Dentist"));
        assert_eq!(spoken(&calendar(Vec::new(), "")), format!("{named}: Sat 17, nothing ahead"));
        assert_eq!(
            spoken(&calendar(Vec::new(), "no calendar service")),
            format!("{named}: Sat 17, no calendar service"),
            "what is missing, never an empty day"
        );
        let claude = |label: &str, detail: &str, progress, turns: &str| {
            WidgetState::new("claude", Body::Claude(usage(label, detail, progress, false, turns)))
        };
        let named = crate::panel::name_of("claude");
        assert_eq!(
            spoken(&claude("72%", "1.8M of 2.5M \u{b7} today", 0.72, "midnight")),
            format!("{named}: 72%, 1.8M of 2.5M \u{b7} today \u{b7} until midnight"),
            "the small line the card writes, turning time and all"
        );
        assert_eq!(
            spoken(&claude("2.4M", "session \u{b7} until 20:30", NO_PROGRESS, "20:30")),
            format!("{named}: 2.4M, session \u{b7} until 20:30"),
            "a time the detail already says is not said twice"
        );
        assert_eq!(spoken(&claude("2.7M", "last 7 days", NO_PROGRESS, "")), format!("{named}: 2.7M, last 7 days"));
    }

    pub fn simple(id: &str) -> WidgetState {
        WidgetState::new(
            id,
            Body::Simple(Simple {
                label: id.to_string(),
                detail: String::new(),
                progress: NO_PROGRESS,
                active: false,
            }),
        )
    }

    fn water(drunk: u32, goal: u32) -> WidgetState {
        WidgetState::new(
            "water",
            Body::Water(Water { drunk, goal, bottle: 500, week: vec![0; 7] }),
        )
    }

    const ICON: i32 = 48;

    #[test]
    fn millilitres_are_shown_in_the_unit_people_speak_in() {
        assert_eq!(compact_ml(0), "0");
        assert_eq!(compact_ml(750), "750");
        assert_eq!(compact_ml(999), "999");
        // Past a litre nobody reads the digits as a quantity, and four of
        // them do not fit a badge one icon wide.
        assert_eq!(compact_ml(1000), "1L");
        assert_eq!(compact_ml(1500), "1.5L");
        assert_eq!(compact_ml(1750), "1.8L");
        assert_eq!(compact_ml(2000), "2L");
        assert_eq!(compact_ml(6000), "6L");
    }

    #[test]
    fn no_amount_of_water_is_ever_shown_in_more_than_four_characters() {
        for ml in (0..=doca_ipc::MAX_WATER_GOAL).step_by(10) {
            let shown = compact_ml(ml);
            assert!(
                shown.chars().count() <= 4,
                "{ml}ml came out as {shown:?}, which is wider than the badge"
            );
        }
    }

    fn look(icon_size: i32) -> Look {
        Look {
            icon_size,
            palette: crate::theme::palette(crate::theme::DEFAULT),
            level: Level::Full,
        }
    }

    /// A shelf and the bar it draws on, with something standing where the row
    /// of icons stands — the thing the shelf must never move.
    fn bar() -> (gtk::Box, gtk::Widget, Shelf, Expand) {
        let items = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let row = gtk::Label::new(Some("the icons")).upcast::<gtk::Widget>();
        items.add(&row);
        (
            items,
            row,
            Shelf::new(),
            Rc::new(|_: &str, _: &gtk::Widget| {}),
        )
    }

    /// Let GTK draw and lay out whatever is due, for about this long.
    fn pumping(span: std::time::Duration) {
        let until = std::time::Instant::now() + span;
        while std::time::Instant::now() < until {
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Two tiles on a bar that is on screen, so that each tile has a window
    /// with a place on the screen to be measured by.
    fn a_bar_on_screen() -> (gtk::Window, gtk::Box, Shelf, Expand) {
        let (items, _, shelf, expand) = bar();
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.add(&items);
        shelf.show(
            &items,
            &[simple("clock"), simple("cpu")],
            look(ICON),
            &expand,
        );
        window.show_all();
        pumping(std::time::Duration::from_millis(60));
        (window, items, shelf, expand)
    }

    fn middle_of(shelf: &Shelf, id: &str) -> f64 {
        let tile = shelf.tile(id).expect("a tile with that id");
        let (_, left, _) = tile.root.window().expect("a tile on screen").origin();
        left as f64 + tile.root.allocated_width() as f64 / 2.0
    }

    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_tile_dragged_past_its_neighbour_takes_its_place() {
        let (window, items, shelf, _) = a_bar_on_screen();
        let past_cpu = middle_of(&shelf, "cpu") + 1.0;

        assert!(shelf.lift("clock").is_some(), "nothing to carry");
        assert_eq!(
            shelf.tile("clock").unwrap().root.opacity(),
            0.0,
            "the tile in the hand is still drawn on the bar"
        );
        assert!(
            shelf.carry_to(&items, "clock", past_cpu),
            "the hole did not move"
        );
        assert_eq!(shelf.order(), ["cpu", "clock"]);
        let at = |id: &str| place_of(&items, &shelf.tile(id).unwrap().root);
        assert!(
            at("cpu") < at("clock"),
            "the bar's own children were not reordered"
        );

        // The neighbour glides: the room the held tile left is a filler
        // that closes over the glide, while its new room opens.
        assert!(shelf.owns(&shelf.gap.borrow().as_ref().unwrap().filler.clone().upcast()));
        pumping(motion::SHUFFLE + std::time::Duration::from_millis(60));
        let filler = shelf.gap.borrow().as_ref().map(|gap| gap.filler.clone());
        assert!(
            filler.is_none_or(|filler| filler.parent().is_none()),
            "the room the tile left never closed"
        );
        assert_eq!(
            shelf.tile("clock").unwrap().room.get(),
            1.0,
            "the room it went to never opened"
        );

        assert_eq!(
            shelf.put_down("clock"),
            Some(vec!["cpu".to_string(), "clock".to_string()])
        );
        assert_eq!(
            shelf.tile("clock").unwrap().root.opacity(),
            1.0,
            "put down and still a hole"
        );
        window.close();
    }

    /// Picked up and put back where it was is nothing to write.
    pub fn a_tile_put_back_where_it_was_asks_for_nothing() {
        let (window, items, shelf, _) = a_bar_on_screen();
        let past_cpu = middle_of(&shelf, "cpu") + 1.0;
        shelf.lift("clock");
        shelf.carry_to(&items, "clock", past_cpu);
        let before_cpu = middle_of(&shelf, "cpu") - 200.0;
        shelf.carry_to(&items, "clock", before_cpu);
        assert_eq!(shelf.order(), ["clock", "cpu"]);
        assert_eq!(shelf.put_down("clock"), None);
        window.close();
    }

    /// A rebuild while a tile is in the hand brings the old order; the tiles
    /// stay in the order the hand has made.
    pub fn a_rebuild_mid_drag_keeps_the_order_the_hand_made() {
        let (window, items, shelf, expand) = a_bar_on_screen();
        let past_cpu = middle_of(&shelf, "cpu") + 1.0;
        shelf.lift("clock");
        shelf.carry_to(&items, "clock", past_cpu);

        shelf.show(
            &items,
            &[simple("clock"), simple("cpu")],
            look(ICON),
            &expand,
        );

        assert_eq!(
            shelf.order(),
            ["cpu", "clock"],
            "the rebuild took the tile back"
        );
        assert!(shelf.lifting(), "the rebuild let go of the tile");
        assert!(shelf.put_down("clock").is_some());
        window.close();
    }

    /// A press that does not travel is still a click, and opens the panel —
    /// on the release, so the same press could have picked the tile up.
    pub fn a_left_click_on_a_tile_opens_it_when_the_button_comes_up() {
        let (items, _, shelf, _) = bar();
        let opened = Rc::new(Cell::new(0));
        let counting = opened.clone();
        let expand: Expand =
            Rc::new(move |_: &str, _: &gtk::Widget| counting.set(counting.get() + 1));
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.add(&items);
        shelf.show(&items, &[simple("clock")], look(ICON), &expand);
        window.show_all();
        pumping(std::time::Duration::from_millis(40));
        let tile = shelf.tile("clock").unwrap().root;

        tile.event(&a_button(&tile, gdk::EventType::ButtonPress, 1));
        assert_eq!(opened.get(), 0, "the panel opened on the press");
        tile.event(&a_button(&tile, gdk::EventType::ButtonRelease, 1));
        assert_eq!(opened.get(), 1, "the click never opened the panel");

        tile.event(&a_button(&tile, gdk::EventType::ButtonPress, 3));
        assert_eq!(
            opened.get(),
            2,
            "the right button stopped opening the panel on the press"
        );
        window.close();
    }

    /// A button event on this widget's window.
    fn a_button(widget: &gtk::Widget, kind: gdk::EventType, button: u32) -> gdk::Event {
        use glib::translate::{ToGlibPtr, ToGlibPtrMut};
        let window = widget.window().expect("a widget on screen has a window");
        let mut event = gdk::Event::new(kind);
        // SAFETY: a button event is a GdkEventButton; the window reference
        // handed over is the one `gdk_event_free` gives back.
        unsafe {
            let raw: *mut gdk::ffi::GdkEvent = event.to_glib_none_mut().0;
            let raw = raw as *mut gdk::ffi::GdkEventButton;
            (*raw).window = window.to_glib_full();
            (*raw).send_event = 1;
            (*raw).button = button;
            (*raw).x = 10.0;
            (*raw).y = 10.0;
        }
        event
    }

    fn roots(shelf: &Shelf) -> Vec<gtk::Widget> {
        shelf
            .tiles
            .borrow()
            .iter()
            .map(|(_, tile)| tile.root.clone())
            .collect()
    }

    fn widths(shelf: &Shelf) -> Vec<i32> {
        shelf
            .tiles
            .borrow()
            .iter()
            .map(|(_, tile)| tile.area.size_request().0)
            .collect()
    }

    /// A note is readable on every paper it offers, in every theme.
    ///
    /// The issue asks for this in as many words — a yellow post-it under
    /// `midnight` must not be a blur. It is the one tile whose colours come
    /// from the note instead of the theme, which is exactly why it needs
    /// saying: nothing else in the bar would have noticed.
    ///
    /// Contrast is measured as the plain difference in luminance between the
    /// ink and the paper. The threshold is not a standard, it is the number
    /// below which these six stop being legible, and it is here so that a
    /// seventh paper added later has to clear the same bar.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_note_is_readable_on_every_paper_it_offers() {
        let ink = note_ink();
        let ink_luma = 0.2126 * ink.red() + 0.7152 * ink.green() + 0.0722 * ink.blue();

        for paper in doca_ipc::note_colour::ALL {
            let (red, green, blue) = paper_of(paper);
            let paper_luma = 0.2126 * red + 0.7152 * green + 0.0722 * blue;

            assert!(
                paper_luma > ink_luma,
                "{paper} paper is darker than the ink, so the note is light on light"
            );
            assert!(
                paper_luma - ink_luma > 0.45,
                "{paper} paper and the ink are {:.2} apart, which is not enough to read",
                paper_luma - ink_luma
            );
        }
    }

    /// The ground a theme puts under its tiles, with how see-through it is.
    /// The same figures as `#ground` in the theme's sheet.
    fn ground_of(theme: &str, area: &gtk::DrawingArea) -> gdk::RGBA {
        match theme {
            "midnight" => gdk::RGBA::new(0.051, 0.059, 0.078, 1.0),
            "paper" => gdk::RGBA::new(0.980, 0.973, 0.953, 0.95),
            "system" => {
                let bg = area.style_context().lookup_color("theme_bg_color").unwrap();
                gdk::RGBA::new(bg.red(), bg.green(), bg.blue(), 0.82)
            }
            _ => gdk::RGBA::new(0.110, 0.110, 0.118, 0.82),
        }
    }

    /// The wash behind a tile marking itself, where the theme has one of its
    /// own: `#widget.active`, the same figures as the sheet.
    fn wash_of(theme: &str) -> Option<gdk::RGBA> {
        match theme {
            "midnight" => Some(gdk::RGBA::new(0.086, 0.137, 0.239, 1.0)),
            "paper" => Some(gdk::RGBA::new(0.776, 0.486, 0.306, 0.16)),
            "native" => Some(gdk::RGBA::new(0.298, 0.553, 1.0, 0.18)),
            _ => None,
        }
    }

    /// How much ink lands on the tile, and how much outside it.
    fn ink_of(state: &WidgetState, look: Look) -> (u32, u32) {
        const MARGIN: i32 = 24;
        let (width, height) = (declared_width(state, look), height_of(look.icon_size));
        let (across, down) = (width + MARGIN * 2, height + MARGIN * 2);
        let mut surface =
            gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, across, down)
                .expect("no surface");
        {
            let cr = gtk::cairo::Context::new(&surface).expect("no context");
            cr.translate(MARGIN as f64, MARGIN as f64);
            let area = gtk::DrawingArea::new();
            draw_in(state, look, &area, &cr, width as f64, height as f64);
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let data = surface.data().expect("the surface is still borrowed");
        let (mut inside, mut outside) = (0, 0);
        for y in 0..down {
            for x in 0..across {
                if data[y as usize * stride + x as usize * 4 + 3] == 0 {
                    continue;
                }
                let on_tile = (MARGIN..MARGIN + width).contains(&x)
                    && (MARGIN..MARGIN + height).contains(&y);
                if on_tile {
                    inside += 1;
                } else {
                    outside += 1;
                }
            }
        }
        (inside, outside)
    }

    fn calendars() -> Vec<WidgetState> {
        let up = |when: &str, title: &str, now| doca_ipc::Upcoming {
            when: when.to_string(),
            title: title.to_string(),
            now,
        };
        let calendar = |upcoming, missing: &str| {
            WidgetState::new(
                "calendar",
                Body::Calendar(doca_ipc::Calendar {
                    weekday: "Wed".to_string(),
                    day: 30,
                    upcoming,
                    missing: missing.to_string(),
                }),
            )
        };
        vec![
            calendar(
                vec![
                    up("now", "Standup with the whole team", true),
                    up("19:00", "Dentist", false),
                    up("Fri 09:30", "Renew the car insurance", false),
                ],
                "",
            ),
            calendar(Vec::new(), ""),
            calendar(Vec::new(), "no calendar service"),
        ]
    }

    /// A Claude body as the daemon sends one.
    fn usage(label: &str, detail: &str, progress: f64, near: bool, turns: &str) -> doca_ipc::Claude {
        doca_ipc::Claude {
            mode: doca_ipc::claude_mode::ALL[0].to_string(),
            label: label.to_string(),
            detail: detail.to_string(),
            progress,
            near,
            input: 0,
            output: 0,
            cache_write: 0,
            cache_read: 0,
            turns: turns.to_string(),
        }
    }

    fn usages() -> Vec<WidgetState> {
        let claude = |label: &str, detail: &str, progress, near, turns: &str| {
            WidgetState::new("claude", Body::Claude(usage(label, detail, progress, near, turns)))
        };
        vec![
            claude("2.4M", "of 2.5M this week", 0.96, true, ""),
            claude("2.4M", "last 5 hours", NO_PROGRESS, false, ""),
            claude("?", "Claude Code not found", NO_PROGRESS, false, ""),
            claude("Claude", "usage format not understood", NO_PROGRESS, false, ""),
            claude("72%", "1.8M of 2.5M \u{b7} session \u{b7} until 20:30", 0.72, false, "20:30"),
            claude("270%", "6.8M of 2.5M \u{b7} today", 1.0, true, "midnight"),
            claude("0%", "0 of 2.5M \u{b7} last 7 days", 0.0, false, ""),
        ]
    }

    /// Every word these tiles write stands 4.5:1 off the bar, WCAG's figure
    /// for small text and these are as small as text gets. `system` is held to
    /// 3:1: its colours are the GTK theme's, and the most the bar can do is
    /// refuse an accent that does not read.
    ///
    /// Against the bar as it really is: two of the grounds are see-through,
    /// so each is taken over the lightest and the darkest wallpaper there is.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn every_word_on_these_tiles_stands_off_the_bar_on_every_theme() {
        let area = gtk::DrawingArea::new();
        use crate::theme::{contrast, over, wallpapers};
        for theme in crate::theme::names() {
            let palette = crate::theme::palette_for(theme, &area.style_context());
            let least = if theme == "system" { 3.0 } else { 4.5 };
            let ground = ground_of(theme, &area);
            for wallpaper in wallpapers() {
                let bar = over(ground, wallpaper);
                let washed = wash_of(theme).map_or(bar, |wash| over(wash, bar));
                let inks = [("label", palette.label), ("detail", palette.detail), ("accent", palette.accent)];
                for (name, ink) in inks {
                    for under in [bar, washed] {
                        let ratio = contrast(over(ink, under), under);
                        assert!(ratio >= least, "{theme}: {name} stands {ratio:.2} off the bar");
                    }
                }
            }
        }
    }

    /// Nothing is cut off or spills onto a neighbour, at any size, in any
    /// state of giving way, with what these tiles say at their longest.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn these_tiles_stay_inside_themselves_at_every_size() {
        let area = gtk::DrawingArea::new();
        for theme in crate::theme::names() {
            let palette = crate::theme::palette_for(theme, &area.style_context());
            for level in [Level::Full, Level::Compact, Level::Minimal] {
                for icon_size in doca_ipc::MIN_ICON_SIZE..=doca_ipc::MAX_ICON_SIZE {
                    let look = Look { icon_size, palette, level };
                    for state in calendars().iter().chain(usages().iter()).chain(meters().iter()) {
                        let (inside, outside) = ink_of(state, look);
                        let said = spoken(state);
                        assert!(inside > 0, "{theme} {level:?} {icon_size}: {said} drew nothing");
                        assert_eq!(outside, 0, "{theme} {level:?} {icon_size}: {said} spilled");
                    }
                }
            }
        }
    }

    /// A Claude card brings its own colours, so the theme has no say in it:
    /// the same card under every theme comes out the same pixels, at every
    /// level the bar can ask for.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_claude_card_looks_the_same_whatever_the_desktop_is_wearing() {
        let area = gtk::DrawingArea::new();
        for state in usages() {
            let Ok(Body::Claude(claude)) = state.body() else { unreachable!() };
            for level in [Level::Full, Level::Compact, Level::Minimal] {
                let (width, height) = (level.width(Tile::Wide, ICON), ICON);
                let mut first: Option<Vec<u8>> = None;
                for theme in crate::theme::names() {
                    let look = Look { icon_size: ICON, palette: crate::theme::palette(theme), level };
                    let (pixels, _) = pixels(width, height, |cr| {
                        draw_claude(&claude, look, &area, cr, width as f64, height as f64)
                    });
                    match &first {
                        None => first = Some(pixels),
                        Some(before) => assert!(
                            *before == pixels,
                            "{level:?}: {} came out differently under {theme}",
                            spoken(&state)
                        ),
                    }
                }
            }
        }
    }

    /// The number is written at every level, the bare square included, and
    /// written in white: a crowded bar can lose the small line and the mark,
    /// never what the tile is for.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_claude_card_writes_its_number_at_every_level() {
        let area = gtk::DrawingArea::new();
        for level in [Level::Full, Level::Compact, Level::Minimal] {
            let (width, height) = (level.width(Tile::Wide, ICON), ICON);
            let look = Look { level, ..look(ICON) };
            let white = |claude: &doca_ipc::Claude| {
                let (data, _) = pixels(width, height, |cr| {
                    draw_claude(claude, look, &area, cr, width as f64, height as f64)
                });
                data.chunks(4).filter(|p| p[0] > 230 && p[1] > 230 && p[2] > 230).count()
            };
            let written = white(&usage("72%", "1.8M of 2.5M", 0.72, false, "20:30"));
            let blank = white(&usage("", "", 0.72, false, ""));
            assert!(written > blank, "{level:?}: the number is not on the card");
        }
    }

    /// And the theme really has no say in it, which is the part a luminance
    /// sum cannot check: the same note drawn under all four themes comes out
    /// the same pixels.
    pub fn a_note_looks_the_same_whatever_the_desktop_is_wearing() {
        let (width, height) = (48, 48);
        let mut first: Option<Vec<u8>> = None;
        for theme in crate::theme::names() {
            let mut surface =
                gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, width, height)
                    .expect("no surface");
            {
                let cr = gtk::cairo::Context::new(&surface).expect("no context");
                let area = gtk::DrawingArea::new();
                draw_note(
                    &doca_ipc::Note {
                        text: "call the dentist".into(),
                        colour: "pink".into(),
                    },
                    Look {
                        icon_size: ICON,
                        palette: crate::theme::palette(theme),
                        level: Level::Full,
                    },
                    &area,
                    &cr,
                    width as f64,
                    height as f64,
                );
            }
            surface.flush();
            let pixels = surface.data().expect("the surface is still borrowed").to_vec();
            match &first {
                None => first = Some(pixels),
                Some(before) => assert_eq!(
                    *before, pixels,
                    "the note came out differently under {theme}, so the theme reached it"
                ),
            }
        }
    }

    /// A cover of any colour is made dark enough to write on.
    ///
    /// The issue calls this a contrast requirement and not a matter of taste,
    /// so it is measured. A cover is somebody else's picture: it can be white,
    /// black or a saturated orange, and the words go on all three. What is
    /// asserted is not that the text is bright enough — it is white, it cannot
    /// be brighter — but that whatever ends up *under* it has been made dark
    /// enough first. That is the veil's whole job and the only part of the
    /// contrast this code controls.
    ///
    /// The veil is measured on its own rather than through `draw_music`,
    /// because a tile with words in it measures the words: the first version
    /// of this test reported 216 on a white cover and I took it for a veil
    /// that was too thin. It was the letters.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_cover_of_any_colour_is_made_dark_enough_to_write_on() {
        let (width, height) = (120.0, 48.0);
        for (name, shade) in [("white", 1.0), ("black", 0.0), ("hot orange", 0.78)] {
            let mut surface = gtk::cairo::ImageSurface::create(
                gtk::cairo::Format::ARgb32,
                width as i32,
                height as i32,
            )
            .expect("no surface");
            {
                let cr = gtk::cairo::Context::new(&surface).expect("no context");
                cr.set_source_rgb(shade, shade * 0.55, 0.0);
                cr.rectangle(0.0, 0.0, width, height);
                let _ = cr.fill();
                veil(&cr, width, height);
            }
            surface.flush();
            let stride = surface.stride() as usize;
            let data = surface.data().expect("the surface is still borrowed");

            // The band the words sit in: the bottom third.
            let mut brightest = 0u8;
            for row in (height as usize * 2 / 3)..height as usize {
                for column in 0..width as usize {
                    let at = row * stride + column * 4;
                    brightest = brightest
                        .max(data[at])
                        .max(data[at + 1])
                        .max(data[at + 2]);
                }
            }

            assert!(
                brightest < 160,
                "under a {name} cover the brightest thing the words sit on is \
                 {brightest}, which white text does not survive"
            );
        }
    }

    /// And the words really are the white that measurement assumes, which the
    /// veil alone cannot say. Under `paper` too: a photo with a scrim over it
    /// is a photo with a scrim over it on every desktop.
    pub fn words_on_a_cover_are_white_whatever_the_theme_is() {
        let cover = painted_cover(255);
        let (width, height) = (120, 48);
        for theme in crate::theme::names() {
            let mut surface =
                gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, width, height)
                    .expect("no surface");
            {
                let cr = gtk::cairo::Context::new(&surface).expect("no context");
                let area = gtk::DrawingArea::new();
                draw_music(
                    &doca_ipc::Music {
                        title: "IIIIIIII".into(),
                        artist: "IIIIIIII".into(),
                        player: "rhythmbox".into(),
                        playing: true,
                        art: cover.clone(),
                    },
                    Look {
                        icon_size: ICON,
                        palette: crate::theme::palette(theme),
                        level: Level::Full,
                    },
                    &area,
                    &cr,
                    width as f64,
                    height as f64,
                );
            }
            surface.flush();
            let stride = surface.stride() as usize;
            let data = surface.data().expect("the surface is still borrowed");
            let mut brightest = 0u8;
            for row in (height as usize * 2 / 3)..height as usize {
                for column in 0..width as usize {
                    let at = row * stride + column * 4;
                    brightest = brightest.max(data[at].min(data[at + 1]).min(data[at + 2]));
                }
            }
            assert!(
                brightest > 200,
                "under {theme} the words on a cover came out at {brightest}, not white"
            );
        }
        std::fs::remove_file(&cover).ok();
    }

    /// A one-colour cover written to a file, because the drawing loads from a
    /// path and not from a pixbuf.
    fn painted_cover(shade: u8) -> String {
        let pixbuf = gdk::gdk_pixbuf::Pixbuf::new(
            gdk::gdk_pixbuf::Colorspace::Rgb,
            false,
            8,
            64,
            64,
        )
        .expect("no pixbuf");
        pixbuf.fill(u32::from_be_bytes([shade, shade, shade, 255]));
        let path = std::env::temp_dir().join(format!(
            "doca-cover-{}-{shade}.png",
            std::process::id()
        ));
        pixbuf.savev(&path, "png", &[]).expect("the cover saves");
        path.display().to_string()
    }

    /// The card holds as much water as the day has in it.
    ///
    /// Drawing is the one part of a tile no assertion about state can reach,
    /// and it is where the mistakes are: the first bottle written here was
    /// full at every level, because the outline was still the current path
    /// when the water was filled and Cairo painted the pair. Nothing but
    /// counting pixels would have said so.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn the_card_holds_as_much_as_the_day_has_in_it() {
        let poured = |drunk: u32| {
            let (width, height) = (120, 48);
            let mut surface = gtk::cairo::ImageSurface::create(
                gtk::cairo::Format::ARgb32,
                width,
                height,
            )
            .expect("no surface");
            {
                let cr = gtk::cairo::Context::new(&surface).expect("no context");
                let area = gtk::DrawingArea::new();
                area.set_size_request(width, height);
                draw_water(
                    &Water { drunk, goal: 2000, bottle: 500, week: vec![0; 7] },
                    look(ICON),
                    &area,
                    &cr,
                    width as f64,
                    height as f64,
                );
            }
            surface.flush();
            let stride = surface.stride() as usize;
            let data = surface.data().expect("the surface is still borrowed");
            // Only the left third, clear of the `+` and the week, and only
            // pixels that are water: blue well over red. The glass behind is
            // dark and the words over it are white, so neither counts.
            //
            // Cairo's ARGB32 is native-endian, so on this machine a pixel is
            // blue, green, red, alpha in memory.
            let mut painted = 0usize;
            for row in 0..height as usize {
                for column in 0..(width as usize / 3) {
                    let at = row * stride + column * 4;
                    let (blue, red) = (data[at] as i32, data[at + 2] as i32);
                    if blue > 150 && blue - red > 80 {
                        painted += 1;
                    }
                }
            }
            painted
        };

        let (empty, quarter, half, full) = (poured(0), poured(500), poured(1000), poured(2000));

        assert_eq!(empty, 0, "an empty day already has water in it");
        assert!(
            quarter > empty && half > quarter && full > half,
            "the card does not track the day: {empty} < {quarter} < {half} < {full}"
        );
        assert!(
            full > quarter * 2,
            "a full card is barely fuller than a quarter one: {quarter} against {full}"
        );
    }

    /// A meter as the bar would draw it: at its declared size for this level,
    /// under the default theme, and its pixels.
    fn metered(state: &WidgetState, level: Level) -> (Vec<u8>, usize, usize, usize) {
        let look = Look { level, ..look(ICON) };
        let (width, height) = (declared_width(state, look), height_of(ICON));
        let (data, stride) = pixels(width, height, |cr| {
            let area = gtk::DrawingArea::new();
            draw_in(state, look, &area, cr, width as f64, height as f64);
        });
        (data, stride, width as usize, height as usize)
    }

    /// Whether a pixel is the share: near the colour it is drawn in, which is
    /// none of the glass, the white or the track. Blue, green, red in memory.
    fn is_share(pixel: &[u8], fill: (f64, f64, f64)) -> bool {
        let near = |byte: u8, channel: f64| (byte as f64 - channel * 255.0).abs() < 40.0;
        near(pixel[2], fill.0) && near(pixel[1], fill.1) && near(pixel[0], fill.2)
    }

    /// The share's pixels, by quarter of the tile: top right, bottom right,
    /// bottom left, top left.
    fn share_quarters(state: &WidgetState, level: Level) -> [usize; 4] {
        let fill = gauge_for(state).expect("not a meter").fill;
        let (data, stride, width, height) = metered(state, level);
        let mut quarters = [0; 4];
        for y in 0..height {
            for x in 0..width {
                if is_share(&data[y * stride + x * 4..], fill) {
                    let quarter = match (x >= width / 2, y >= height / 2) {
                        (true, false) => 0,
                        (true, true) => 1,
                        (false, true) => 2,
                        (false, false) => 3,
                    };
                    quarters[quarter] += 1;
                }
            }
        }
        quarters
    }

    /// How many columns of the tile the share reaches across.
    fn share_columns(state: &WidgetState, level: Level) -> usize {
        let fill = gauge_for(state).expect("not a meter").fill;
        let (data, stride, width, height) = metered(state, level);
        (0..width)
            .filter(|x| (0..height).any(|y| is_share(&data[y * stride + x * 4..], fill)))
            .count()
    }

    /// The meters and the counts at their longest and their emptiest, for
    /// the checks that go through every tile.
    fn meters() -> Vec<WidgetState> {
        vec![
            reading("cpu", "63%", "CPU", 0.63, false),
            reading("cpu", "100%", "CPU", 1.0, true),
            reading("cpu", "0%", "CPU", 0.0, false),
            reading("battery", "100%", "full", 1.0, false),
            reading("battery", "18%", "25m", 0.18, false),
            reading("battery", "64%", "11h 59 to full", 0.64, true),
            reading("battery", "—", "no battery", NO_PROGRESS, false),
            reading("time-progress", "100%", "of this year", 1.0, false),
            reading("time-progress", "0%", "of this week", 0.0, false),
            reading("pomodoro", "25:00", "focus \u{b7} paused", 0.0, false),
            reading("pomodoro", "18:42", "focus", 0.25, true),
            reading("pomodoro", "14:59", "long break \u{b7} paused", 0.0, false),
            reading("timer", "04:10", "running", 0.70, true),
            reading("timer", "1440:00", "paused", 0.0, false),
            reading("timer", "00:00", "done", 1.0, true),
        ]
    }

    /// One icon wide, a meter's share goes round a ring, as far round as the
    /// share is, from twelve o'clock.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_meter_one_icon_wide_rings_as_much_as_its_share() {
        for id in METERS {
            let at = |share: f64| reading(id, &format!("{}%", (share * 100.0).round()), "", share, false);
            let total = |share: f64| share_quarters(&at(share), Level::Minimal).iter().sum::<usize>();

            let [top_right, bottom_right, bottom_left, top_left] = share_quarters(&at(0.25), Level::Minimal);
            assert!(
                top_right > (bottom_right + bottom_left + top_left) * 3,
                "{id}: a quarter is not the top right: {top_right} against {}",
                bottom_right + bottom_left + top_left
            );
            assert_eq!(total(0.0), 0, "{id}: nothing measured, and a share drawn");
            let ratio = total(0.5) as f64 / total(1.0) as f64;
            assert!((0.42..0.58).contains(&ratio), "{id}: half a ring is {ratio:.2} of a whole one");
        }
    }

    /// Wide, the share is a pill as long as the share, at both the widths a
    /// wide tile has; one icon wide it is a ring and no pill. The number is
    /// there at every level, and what it is a share of only when the tile is
    /// whole.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_meter_keeps_its_share_and_its_number_as_it_gives_way() {
        for id in METERS {
            let at = |share: f64, detail: &str| reading(id, "63%", detail, share, false);
            for level in [Level::Full, Level::Compact] {
                let ratio = share_columns(&at(0.5, ""), level) as f64 / share_columns(&at(1.0, ""), level) as f64;
                assert!((0.42..0.58).contains(&ratio), "{id} {level:?}: half a pill is {ratio:.2} of a whole one");
                let [top_right, bottom_right, bottom_left, top_left] = share_quarters(&at(1.0, ""), level);
                assert!(
                    bottom_right + bottom_left > (top_right + top_left) * 3,
                    "{id} {level:?}: the pill is not under the number"
                );
            }
            let [top_right, bottom_right, bottom_left, top_left] = share_quarters(&at(1.0, ""), Level::Minimal);
            assert!(
                [top_right, bottom_right, bottom_left, top_left].iter().all(|quarter| *quarter > 0),
                "{id}: a whole ring is not all the way round"
            );

            // The words: white, and only the words are.
            let words = |state: &WidgetState, level| {
                let (data, _, _, _) = metered(state, level);
                data.chunks(4).filter(|pixel| pixel[0] > 200 && pixel[1] > 200 && pixel[2] > 200).count()
            };
            for level in [Level::Full, Level::Compact, Level::Minimal] {
                assert!(words(&at(0.5, ""), level) > 0, "{id} {level:?}: the number is gone");
            }
            assert!(
                words(&at(0.5, "CPU"), Level::Full) > words(&at(0.5, ""), Level::Full),
                "{id}: a whole tile does not say what its number is a share of"
            );
            for level in [Level::Compact, Level::Minimal] {
                assert_eq!(
                    metered(&at(0.5, "CPU"), level).0,
                    metered(&at(0.5, ""), level).0,
                    "{id} {level:?}: a tile giving way still says what its number is a share of"
                );
            }
        }
    }

    /// A timer that has rung says so in a hot rim round its card, at every
    /// level, and a timer still running has no such rim.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_timer_that_has_rung_is_plain_on_its_card() {
        let hot = TIMER_DONE_GAUGE.edge.expect("a rung timer has a rim");
        let rim = |state: &WidgetState, level| {
            let (data, stride, width, height) = metered(state, level);
            let mut lit = 0;
            for y in 0..height {
                for x in 0..width {
                    let on_the_edge = x < 2 || y < 2 || x >= width - 2 || y >= height - 2;
                    if on_the_edge && is_share(&data[y * stride + x * 4..], hot) {
                        lit += 1;
                    }
                }
            }
            lit
        };
        for level in [Level::Full, Level::Compact, Level::Minimal] {
            let done = rim(&reading("timer", "00:00", "done", 1.0, true), level);
            let running = rim(&reading("timer", "04:10", "running", 0.70, true), level);
            assert!(done > 0, "{level:?}: a timer that has rung has no rim");
            assert_eq!(running, 0, "{level:?}: a running timer lights a rim");
        }
    }

    /// The theme has no say in a meter: the same reading under all four
    /// themes is the same pixels, at every level.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_meter_looks_the_same_whatever_the_desktop_is_wearing() {
        let area = gtk::DrawingArea::new();
        // A meter with nothing to measure is two lines in the theme's ink.
        for state in meters().into_iter().filter(|state| gauge_for(state).is_some()) {
            for level in [Level::Full, Level::Compact, Level::Minimal] {
                let mut first: Option<Vec<u8>> = None;
                for theme in crate::theme::names() {
                    let look = Look { icon_size: ICON, palette: crate::theme::palette_for(theme, &area.style_context()), level };
                    let (width, height) = (declared_width(&state, look), height_of(ICON));
                    let (data, _) = pixels(width, height, |cr| {
                        draw_in(&state, look, &area, cr, width as f64, height as f64);
                    });
                    match &first {
                        None => first = Some(data),
                        Some(before) => assert!(
                            *before == data,
                            "{} {level:?} came out differently under {theme}, so the theme reached it",
                            spoken(&state)
                        ),
                    }
                }
            }
        }
    }

    /// The calendar draws its date always, its list only when there is room,
    /// and what is happening now in the accent.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_calendar_tile_keeps_its_date_and_gives_up_its_list() {
        let up = |when: &str, title: &str, now: bool| doca_ipc::Upcoming {
            when: when.to_string(),
            title: title.to_string(),
            now,
        };
        let calendar = |upcoming: Vec<doca_ipc::Upcoming>| doca_ipc::Calendar {
            weekday: "Sat".to_string(),
            day: 17,
            upcoming,
            missing: String::new(),
        };
        // Every pixel's alpha, and how many right of the leaf are painted and
        // how many of those in the accent — at the width the bar really gives
        // a calendar at that level.
        let drawn = |calendar: &doca_ipc::Calendar, level: Level| {
            let height = ICON;
            let width = level.width(Tile::Wide, ICON);
            let mut surface =
                gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, width, height).expect("no surface");
            let mut tile_look = look(ICON);
            tile_look.level = level;
            {
                let cr = gtk::cairo::Context::new(&surface).expect("no context");
                let area = gtk::DrawingArea::new();
                area.set_size_request(width, height);
                draw_calendar(calendar, tile_look, &area, &cr, width as f64, height as f64);
            }
            surface.flush();
            let stride = surface.stride() as usize;
            let data = surface.data().expect("the surface is still borrowed");
            let fill = tile_look.palette.accent;
            let leaf = (ICON as f64 * 0.72) as usize + 2;
            let (mut alphas, mut list, mut accent) = (Vec::new(), 0usize, 0usize);
            for row in 0..height as usize {
                for column in 0..width as usize {
                    let at = row * stride + column * 4;
                    alphas.push(data[at + 3]);
                    if data[at + 3] == 0 || column < leaf {
                        continue;
                    }
                    list += 1;
                    // ARGB32 is premultiplied BGRA in memory. Text over
                    // nothing keeps its ink's colour at every coverage,
                    // so un-premultiplied it is the accent or it is not.
                    let alpha = data[at + 3] as f64;
                    let channel = |offset: usize| data[at + offset] as f64 * 255.0 / alpha;
                    if (channel(2) - fill.red() * 255.0).abs() < 12.0
                        && (channel(1) - fill.green() * 255.0).abs() < 12.0
                        && (channel(0) - fill.blue() * 255.0).abs() < 12.0
                    {
                        accent += 1;
                    }
                }
            }
            (alphas, list, accent)
        };

        let busy = calendar(vec![up("19:00", "Dinner", false), up("Sun 09:00", "Mass", false)]);
        let (full, list, _) = drawn(&busy, Level::Full);
        assert!(full.iter().any(|a| *a > 0), "nothing was drawn");
        assert!(list > 0, "the list was not drawn");

        let (minimal, _, _) = drawn(&busy, Level::Minimal);
        let (minimal_empty, _, _) = drawn(&calendar(vec![]), Level::Minimal);
        assert!(minimal.iter().any(|a| *a > 0), "a crowded bar lost the date, which is the one thing it keeps");
        assert_eq!(minimal, minimal_empty, "the narrowest tile still drew part of a list it has no room for");

        let (_, compact_list, _) = drawn(&busy, Level::Compact);
        assert!(compact_list > 0 && compact_list < list, "compact keeps the next time and only that");

        let (_, _, calm) = drawn(&calendar(vec![up("19:00", "Dinner", false)]), Level::Full);
        let (_, _, lit) = drawn(&calendar(vec![up("now", "Dinner", true)]), Level::Full);
        assert!(lit > calm, "something under way is not drawn apart: {calm} accent pixels against {lit}");

        let (_, nothing, _) = drawn(&calendar(vec![]), Level::Full);
        assert!(nothing > 0, "nothing ahead drew nothing, which reads as a broken tile");
    }

    /// The tile says what the panel says, the daemon's echo of a moment ago
    /// does not take the last letters back off it, and the paper stays the
    /// note's. With the panel down the daemon has the last word again.
    pub fn a_note_being_typed_is_on_the_tile_before_the_daemon_hears() {
        let note = |text: &str| {
            WidgetState::new(
                "note",
                Body::Note(doca_ipc::Note {
                    text: text.to_string(),
                    colour: "pink".to_string(),
                }),
            )
        };
        let (items, _, shelf, expand) = bar();
        shelf.show(&items, &[note("mi")], look(ICON), &expand);
        let shown = || shelf.tiles.borrow()[0].1.shown.borrow().clone();

        shelf.typing("note", Some("milk"));
        assert_eq!(shown(), note("milk"), "the tile fell behind the typing");

        shelf.update(&note("mil"));
        assert_eq!(shown(), note("milk"), "an old echo took letters off the tile");

        shelf.typing("note", None);
        shelf.update(&note("milk and bread"));
        assert_eq!(shown(), note("milk and bread"), "the tile stayed on the panel's word");
    }

    /// The one that matters. Everything else here is about this holding under
    /// a list that moved.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn showing_the_same_widgets_again_keeps_the_very_same_tiles() {
        let (items, _, shelf, expand) = bar();
        let first = shelf.show(&items, &[simple("clock"), simple("cpu")], look(ICON), &expand);
        let before = roots(&shelf);

        let again = shelf.show(&items, &[simple("clock"), simple("cpu")], look(ICON), &expand);

        assert_eq!(first, Reconciled { kept: 0, made: 2, gone: 0 });
        assert_eq!(
            again,
            Reconciled { kept: 2, made: 0, gone: 0 },
            "a rebuild that changed nothing made tiles anyway"
        );
        assert_eq!(before, roots(&shelf), "the tiles were built again");
    }

    pub fn a_widget_that_went_takes_its_tile_off_the_bar() {
        let (items, _, shelf, expand) = bar();
        shelf.show(&items, &[simple("clock"), simple("cpu")], look(ICON), &expand);
        let clock = roots(&shelf)[0].clone();
        let cpu = roots(&shelf)[1].clone();

        let did = shelf.show(&items, &[simple("clock")], look(ICON), &expand);

        assert_eq!(did, Reconciled { kept: 1, made: 0, gone: 1 });
        assert_eq!(roots(&shelf), vec![clock.clone()], "the wrong tile stayed");
        // Going, not gone: it shrinks off the bar where it stood, the way an
        // icon does, rather than leaving a tile-sized hole in one frame.
        assert!(
            items.children().contains(&cpu),
            "the tile was snatched off the bar instead of leaving it"
        );

        std::thread::sleep(motion::ENTRY);
        shelf.settle();

        assert!(
            !items.children().contains(&cpu),
            "a widget nobody asks for is still on the bar"
        );
        assert!(items.children().contains(&clock), "the survivor went too");
    }

    /// The other half of a dock switch: the tiles of the dock that came in.
    /// They grow into the room that opens for them rather than appearing in
    /// it at full size — but only when something is switched, never when the
    /// dock starts with them already on it.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_widget_that_joined_grows_onto_the_bar_rather_than_appearing() {
        let (items, _, shelf, expand) = bar();
        shelf.show(&items, &[simple("clock")], look(ICON), &expand);
        assert_eq!(
            widths(&shelf),
            vec![Tile::Wide.width(ICON)],
            "a tile the dock started with was announced as if it had just come"
        );

        shelf.show(&items, &[simple("clock"), water(2, 8)], look(ICON), &expand);
        let joined = widths(&shelf)[1];
        assert!(
            joined < Tile::Wide.width(ICON) / 2,
            "the new tile took {joined}px the instant it was asked for"
        );
        assert!(shelf.changing());

        std::thread::sleep(motion::ENTRY);
        shelf.settle();

        assert_eq!(
            widths(&shelf),
            vec![Tile::Wide.width(ICON), Tile::Wide.width(ICON)],
            "the new tile never finished arriving"
        );
        assert!(!shelf.changing());
        assert_eq!(shelf.unfolding(), 0);
    }

    /// A tile on its way out goes from where it stood, between the same two
    /// neighbours, not from the end of the shelf it was moved to.
    pub fn a_tile_that_is_leaving_keeps_its_place_until_it_has_gone() {
        let (items, row, shelf, expand) = bar();
        shelf.show(
            &items,
            &[simple("clock"), simple("cpu"), simple("disk")],
            look(ICON),
            &expand,
        );
        let standing = roots(&shelf);

        shelf.show(&items, &[simple("clock"), simple("disk")], look(ICON), &expand);

        let divider = shelf.divider.borrow().clone().expect("a divider").upcast();
        let mut expected = vec![row, divider];
        expected.extend(standing);
        assert_eq!(
            items.children(),
            expected,
            "the leaving tile was moved before it went"
        );
    }

    /// Two dock switches in a row, faster than a tile can leave: the tile
    /// asked for again is the same tile, turned round where it stood.
    pub fn a_tile_asked_for_again_while_leaving_turns_round() {
        let (items, _, shelf, expand) = bar();
        shelf.show(&items, &[simple("clock"), simple("cpu")], look(ICON), &expand);
        let cpu = roots(&shelf)[1].clone();

        shelf.show(&items, &[simple("clock")], look(ICON), &expand);
        let did = shelf.show(&items, &[simple("clock"), simple("cpu")], look(ICON), &expand);

        assert_eq!(
            did,
            Reconciled { kept: 2, made: 0, gone: 0 },
            "a tile that had not finished leaving was made again"
        );
        assert_eq!(roots(&shelf)[1], cpu);
        let tiles = shelf.tiles.borrow();
        assert!(!tiles[1].1.life.get().leaving(), "the tile that came back is still going");
    }

    /// Adding a widget to one dock must not reset the stopwatch in another,
    /// and a tile rebuilt is a tile reset.
    pub fn a_widget_that_joined_leaves_the_others_alone() {
        let (items, _, shelf, expand) = bar();
        shelf.show(&items, &[simple("clock")], look(ICON), &expand);
        let clock = roots(&shelf)[0].clone();

        let did = shelf.show(&items, &[simple("clock"), water(0, 8)], look(ICON), &expand);

        assert_eq!(did, Reconciled { kept: 1, made: 1, gone: 0 });
        assert_eq!(roots(&shelf)[0], clock, "the tile that stayed was remade");
        assert_eq!(roots(&shelf).len(), 2);
    }

    /// Reordering is the case where unparenting would be easiest and worst:
    /// the tiles all survive and only their places change.
    pub fn a_reorder_moves_the_tiles_rather_than_remaking_them() {
        let (items, row, shelf, expand) = bar();
        shelf.show(&items, &[simple("clock"), simple("cpu")], look(ICON), &expand);
        let clock = roots(&shelf)[0].clone();
        let cpu = roots(&shelf)[1].clone();

        let did = shelf.show(&items, &[simple("cpu"), simple("clock")], look(ICON), &expand);

        assert_eq!(
            did,
            Reconciled { kept: 2, made: 0, gone: 0 },
            "reordering remade a tile"
        );
        assert_eq!(roots(&shelf), vec![cpu.clone(), clock.clone()]);
        let on_the_bar = items.children();
        let divider = shelf.divider.borrow().clone().expect("a divider").upcast();
        assert_eq!(
            on_the_bar,
            vec![row, divider, cpu, clock],
            "the bar is not in the order the daemon gave"
        );
    }

    pub fn the_divider_only_stands_where_there_is_something_to_divide() {
        let (items, row, shelf, expand) = bar();
        shelf.show(&items, &[simple("clock")], look(ICON), &expand);
        assert_eq!(items.children().len(), 3, "row, divider, tile");

        shelf.show(&items, &[], look(ICON), &expand);
        assert_eq!(
            items.children().len(),
            3,
            "the divider went before the tile it divides had finished going"
        );
        std::thread::sleep(motion::ENTRY);
        shelf.settle();

        assert_eq!(
            items.children(),
            vec![row],
            "a divider with nothing behind it is a line at the end of the bar"
        );
    }

    /// The divider is kept rather than remade, so that the empty case is not
    /// itself a rebuild.
    pub fn the_divider_that_comes_back_is_the_one_that_left() {
        let (items, _, shelf, expand) = bar();
        shelf.show(&items, &[simple("clock")], look(ICON), &expand);
        let divider = shelf.divider.borrow().clone().expect("a divider");

        shelf.show(&items, &[], look(ICON), &expand);
        shelf.show(&items, &[simple("clock")], look(ICON), &expand);

        assert_eq!(shelf.divider.borrow().clone().expect("a divider"), divider);
    }

    pub fn the_shelf_knows_what_is_its_own() {
        let (items, row, shelf, expand) = bar();
        shelf.show(&items, &[simple("clock")], look(ICON), &expand);

        assert!(!shelf.owns(&row), "the row of icons is not the shelf's");
        for mine in items.children().into_iter().filter(|child| *child != row) {
            assert!(shelf.owns(&mine), "the shelf disowned {mine:?}");
        }
    }

    pub fn a_state_for_a_widget_with_no_tile_is_not_claimed() {
        let (items, _, shelf, expand) = bar();
        shelf.show(&items, &[simple("clock")], look(ICON), &expand);

        assert!(shelf.update(&simple("clock")), "the clock has a tile");
        assert!(
            !shelf.update(&water(1, 8)),
            "a widget with no tile cannot have been updated"
        );
    }

    /// Each widget's declared size, on the bar: a square tile is one icon and
    /// a wide one is two and a half, side by side.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    /// A tile says what it shows to a screen reader, and says it again when
    /// it changes — it is a drawing, and otherwise says nothing at all.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_tile_tells_a_screen_reader_what_it_shows() {
        use atk::prelude::*;
        let tile = WidgetTile::new(&water(500, 2000), look(ICON), Rc::new(|_: &str, _: &gtk::Widget| {}));
        let accessible = tile.root.accessible().expect("a widget has an accessible");
        assert_eq!(accessible.name().as_deref(), Some("Water: 500 of 2000 ml"));
        assert_eq!(accessible.role(), atk::Role::PushButton, "choosing it opens a panel");

        tile.update(&water(750, 2000));
        assert_eq!(accessible.name().as_deref(), Some("Water: 750 of 2000 ml"), "it said the old amount");
    }

    pub fn a_tile_is_the_size_the_widget_declared() {
        let (items, _, shelf, expand) = bar();

        shelf.show(&items, &[simple("clock"), water(2, 8)], look(ICON), &expand);

        assert_eq!(
            widths(&shelf),
            vec![Tile::Wide.width(ICON), Tile::Wide.width(ICON)],
            "a tile is not the width its own variant asked for"
        );
    }

    /// The acceptance that a tile does not stay the same size when the icons
    /// go from 24 to 96 — and that the tiles already on the bar take the new
    /// size without being remade.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_tile_follows_the_icon_size_without_being_rebuilt() {
        let (items, _, shelf, expand) = bar();
        shelf.show(&items, &[simple("clock"), water(2, 8)], look(24), &expand);
        let before = roots(&shelf);
        let small = widths(&shelf);

        let did = shelf.show(&items, &[simple("clock"), water(2, 8)], look(96), &expand);

        assert_eq!(did, Reconciled { kept: 2, made: 0, gone: 0 });
        assert_eq!(before, roots(&shelf), "resizing the icons remade the tiles");
        assert_eq!(small, vec![Tile::Wide.width(24), Tile::Wide.width(24)]);
        assert_eq!(
            widths(&shelf),
            vec![Tile::Wide.width(96), Tile::Wide.width(96)],
            "the tiles are the same size at 96px icons as at 24px"
        );
    }

    /// A tile that joins or leaves must not move its neighbours, which is
    /// what a declared width buys: every tile's size comes from its own
    /// variant, so nothing about it depends on what else is on the bar.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_tile_coming_or_going_does_not_resize_its_neighbours() {
        let (items, _, shelf, expand) = bar();
        shelf.show(&items, &[simple("clock")], look(ICON), &expand);
        let alone = widths(&shelf);

        shelf.show(
            &items,
            &[simple("clock"), water(2, 8), simple("cpu")],
            look(ICON),
            &expand,
        );
        let crowded = widths(&shelf);
        shelf.show(&items, &[simple("clock")], look(ICON), &expand);

        assert_eq!(crowded[0], alone[0], "a tile joining squeezed the one beside it");
        assert_eq!(widths(&shelf), alone, "a tile leaving stretched the survivor");
    }

    /// That nothing in the drawing path can reach the daemon.
    ///
    /// The strongest form of it is structural — a tile holds a state and a
    /// closure that opens a panel, and no proxy at all — but the closure is
    /// the one thing that could be called from a draw, so it is handed a
    /// closure that would fail the test if it ever were.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn drawing_a_tile_asks_the_daemon_for_nothing() {
        let asked = Rc::new(Cell::new(false));
        let telling = asked.clone();
        let expand: Expand = Rc::new(move |_: &str, _: &gtk::Widget| telling.set(true));

        let items = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let shelf = Shelf::new();
        shelf.show(
            &items,
            &[
                simple("clock"),
                WidgetState::new(
                    "cpu",
                    Body::Simple(Simple {
                        label: "42%".to_string(),
                        detail: "CPU".to_string(),
                        progress: 0.42,
                        active: true,
                    }),
                ),
                water(3, 8),
            ],
            look(ICON),
            &expand,
        );

        for (_, tile) in shelf.tiles.borrow().iter() {
            let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 256, 256)
                .expect("a surface to draw on");
            let cr = cairo::Context::new(&surface).expect("a context");
            tile.area.set_size_request(
                tile.shown.borrow().tile().unwrap().width(ICON),
                height_of(ICON),
            );
            // `draw` is the whole of the path, called the way the signal
            // calls it.
            draw(&tile.shown.borrow(), tile.look.get(), &tile.area, &cr);
        }

        assert!(!asked.get(), "drawing a tile went to the daemon");
    }

    /// The panel hands the keyboard back to the tile that opened it, which
    /// needs the tile to be able to hold it.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_tile_can_be_given_the_keyboard_back() {
        let (items, _, shelf, expand) = bar();
        shelf.show(&items, &[simple("clock")], look(ICON), &expand);

        for (_, tile) in shelf.tiles.borrow().iter() {
            assert!(
                tile.root.can_focus(),
                "a tile cannot take focus, so handing it back after a panel \
                 closes does nothing"
            );
        }
    }

    /// A body from a build that knew a variant this one does not still leaves
    /// a tile on the bar, of a declared size, that draws something.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_widget_this_bar_cannot_read_still_gets_a_tile() {
        let (items, _, shelf, expand) = bar();
        // What a newer daemon's `calendar` would look like here: the shelf
        // cannot read the body, and must not drop the widget for it.
        let unreadable = unreadable_state("calendar");

        let did = shelf.show(&items, &[unreadable.clone()], look(ICON), &expand);

        assert_eq!(did, Reconciled { kept: 0, made: 1, gone: 0 });
        assert_eq!(widths(&shelf), vec![Tile::Wide.width(ICON)]);

        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 256, 256)
            .expect("a surface to draw on");
        let cr = cairo::Context::new(&surface).expect("a context");
        let tile = &shelf.tiles.borrow()[0].1;
        draw(&unreadable, look(ICON), &tile.area, &cr);
    }

    /// A state whose body this build has no drawer for.
    ///
    /// Built by taking a real one and renaming its body, which is the only
    /// way to get one — `WidgetState::new` cannot make a name and a payload
    /// disagree, which is the point of it.
    fn unreadable_state(kind: &str) -> WidgetState {
        use zbus::zvariant::{serialized::Context, to_bytes, Endian};

        // The wire shape is `(s s v)`: id, kind, payload. Rewriting the kind
        // in a serialised state is how a reader older than its writer sees
        // one, and there is no other way to build it from here.
        let real = simple("calendar");
        let context = Context::new_dbus(Endian::Little, 0);
        let bytes = to_bytes(context, &(real.id.clone(), kind.to_string(), {
            let (_, body) = split(&real);
            body
        }))
        .expect("a forged state serialises");
        let (forged, _): (WidgetState, _) =
            bytes.deserialize().expect("a forged state deserialises");
        assert!(forged.body().is_err(), "the forged body was readable after all");
        forged
    }

    /// The payload of a state, by going back through the wire.
    fn split(state: &WidgetState) -> (String, zbus::zvariant::OwnedValue) {
        use zbus::zvariant::{serialized::Context, to_bytes, Endian};

        let context = Context::new_dbus(Endian::Little, 0);
        let bytes = to_bytes(context, state).expect("a state serialises");
        let ((_, kind, body), _): ((String, String, zbus::zvariant::OwnedValue), _) =
            bytes.deserialize().expect("a state is an id, a name and a payload");
        (kind, body)
    }

    /// A progress bar that grows with the icons, and never to nothing.
    /// Draws into a fresh surface and hands back its pixels, four bytes each:
    /// blue, green, red and alpha in memory, Cairo's ARGB32 being
    /// native-endian on this machine.
    fn pixels(width: i32, height: i32, draw: impl FnOnce(&cairo::Context)) -> (Vec<u8>, usize) {
        let mut surface =
            cairo::ImageSurface::create(cairo::Format::ARgb32, width, height).expect("no surface");
        {
            let cr = cairo::Context::new(&surface).expect("no context");
            draw(&cr);
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let data = surface.data().expect("the surface is still borrowed").to_vec();
        (data, stride)
    }

    /// A ring in pure red over a track nobody can see, and where the red is,
    /// by quarter of the clock: top right, bottom right, bottom left, top
    /// left.
    fn ring_quarters(fraction: f64) -> [usize; 4] {
        const SIDE: i32 = 100;
        let clear = gdk::RGBA::new(0.0, 0.0, 0.0, 0.0);
        let red = gdk::RGBA::new(1.0, 0.0, 0.0, 1.0);
        let (data, stride) = pixels(SIDE, SIDE, |cr| ring(cr, 50.0, 50.0, 40.0, 8.0, fraction, clear, red));
        let mut quarters = [0; 4];
        for y in 0..SIDE as usize {
            for x in 0..SIDE as usize {
                let at = y * stride + x * 4;
                if data[at + 2] > 128 && data[at + 3] > 128 {
                    let quarter = match (x >= 50, y >= 50) {
                        (true, false) => 0,
                        (true, true) => 1,
                        (false, true) => 2,
                        (false, false) => 3,
                    };
                    quarters[quarter] += 1;
                }
            }
        }
        quarters
    }

    #[test]
    fn a_ring_starts_at_twelve_and_goes_the_way_a_clock_does() {
        let [top_right, bottom_right, bottom_left, top_left] = ring_quarters(0.25);
        // The round ends reach half their thickness past twelve and three:
        // two half discs of radius 4, give or take their soft edges, and
        // nothing else.
        let spill = bottom_right + bottom_left + top_left;
        let caps = (std::f64::consts::PI * 4.0 * 4.0 * 1.25).ceil() as usize;
        assert!(
            spill <= caps && top_right > spill * 5,
            "a quarter ring is not the top right quarter: {top_right} there, {spill} elsewhere"
        );
    }

    #[test]
    fn a_ring_fills_as_much_of_itself_as_its_share() {
        let total = |fraction| ring_quarters(fraction).iter().sum::<usize>();
        let (empty, half, whole) = (total(0.0), total(0.5), total(1.0));

        assert_eq!(empty, 0, "an empty ring has a dot at twelve");
        let ratio = half as f64 / whole as f64;
        assert!(
            (0.45..0.56).contains(&ratio),
            "half a ring is {ratio:.2} of a whole one"
        );
        assert_eq!(total(1.7), whole, "a ring past full is fuller than full");
    }

    #[test]
    fn an_empty_ring_still_shows_its_track() {
        let track = gdk::RGBA::new(1.0, 1.0, 1.0, 0.2);
        let fill = gdk::RGBA::new(1.0, 0.0, 0.0, 1.0);
        let (data, _) = pixels(100, 100, |cr| ring(cr, 50.0, 50.0, 40.0, 8.0, 0.0, track, fill));
        let inked = data.chunks(4).filter(|pixel| pixel[3] > 0).count();
        assert!(inked > 0, "an empty ring is not there at all");
    }

    /// The widgets that are meters, and the counts that have an end to
    /// measure against.
    const METERS: [&str; 5] = ["cpu", "battery", "time-progress", "pomodoro", "timer"];

    /// Every glass a meter can wear.
    const GAUGES: [(&str, Gauge); 10] = [
        ("cpu", CPU_GAUGE),
        ("hot cpu", CPU_HOT_GAUGE),
        ("battery", BATTERY_GAUGE),
        ("charging battery", BATTERY_CHARGING_GAUGE),
        ("low battery", BATTERY_LOW_GAUGE),
        ("time", TIME_GAUGE),
        ("pomodoro focus", POMODORO_FOCUS_GAUGE),
        ("pomodoro break", POMODORO_BREAK_GAUGE),
        ("timer", TIMER_GAUGE),
        ("timer done", TIMER_DONE_GAUGE),
    ];

    /// A meter's words are white on its glass, and its share stands off the
    /// track it runs in, whatever the glass: 4.5:1 for the words, WCAG's
    /// figure for small text, and 3:1 for the share, its figure for a shape
    /// that has to be seen. Across the whole gradient, since a word can sit
    /// anywhere on it. The theme is not in this at all, which is the point of
    /// a meter bringing its own colours.
    #[test]
    fn a_meter_is_readable_on_every_glass_it_wears() {
        use crate::theme::{contrast, over};
        for (name, gauge) in GAUGES {
            let Glass { from, to, .. } = gauge.glass;
            for step in 0..=4 {
                let t = step as f64 / 4.0;
                let mix = |a: f64, b: f64| a + (b - a) * t;
                let glass = gdk::RGBA::new(mix(from.0, to.0), mix(from.1, to.1), mix(from.2, to.2), 1.0);
                for (what, ink) in [("number", ON_A_GAUGE), ("detail", faded(ON_A_GAUGE, QUIET_ON_A_GAUGE))] {
                    let ratio = contrast(over(ink, glass), glass);
                    assert!(ratio >= 4.5, "{name}: the {what} stands {ratio:.2} off its glass");
                }
                let track = over(faded(ON_A_GAUGE, GAUGE_TRACK), glass);
                let ratio = contrast(rgb(gauge.fill), track);
                assert!(ratio >= 3.0, "{name}: the share stands {ratio:.2} off its track");
            }
        }
    }

    fn reading(id: &str, label: &str, detail: &str, progress: f64, active: bool) -> WidgetState {
        WidgetState::new(
            id,
            Body::Simple(Simple { label: label.to_string(), detail: detail.to_string(), progress, active }),
        )
    }

    fn gauge_for(state: &WidgetState) -> Option<Gauge> {
        match state.body() {
            Ok(Body::Simple(simple)) => gauge_of(&state.id, &simple),
            _ => None,
        }
    }

    #[test]
    fn only_a_widget_that_measures_a_share_is_a_meter() {
        assert_eq!(gauge_for(&reading("cpu", "63%", "CPU", 0.63, false)), Some(CPU_GAUGE));
        assert_eq!(
            gauge_for(&reading("cpu", "93%", "CPU", 0.93, true)),
            Some(CPU_HOT_GAUGE),
            "a processor the daemon marks is hot"
        );
        assert_eq!(gauge_for(&reading("cpu", "—", "CPU", NO_PROGRESS, false)), None, "nothing to measure");

        assert_eq!(gauge_for(&reading("battery", "64%", "3h 10", 0.64, false)), Some(BATTERY_GAUGE));
        assert_eq!(
            gauge_for(&reading("battery", "18%", "25m", 0.18, false)),
            Some(BATTERY_LOW_GAUGE),
            "a fifth or less and not charging is low"
        );
        assert_eq!(gauge_for(&reading("battery", "20%", "", 0.20, false)), Some(BATTERY_LOW_GAUGE));
        assert_eq!(gauge_for(&reading("battery", "21%", "", 0.21, false)), Some(BATTERY_GAUGE));
        assert_eq!(
            gauge_for(&reading("battery", "18%", "1h 20 to full", 0.18, true)),
            Some(BATTERY_CHARGING_GAUGE),
            "a low battery on the charger is charging, not low"
        );
        assert_eq!(
            gauge_for(&reading("battery", "—", "no battery", NO_PROGRESS, false)),
            None,
            "a machine with no battery has nothing to measure"
        );
        assert_eq!(gauge_for(&reading("time-progress", "63%", "of today", 0.63, false)), Some(TIME_GAUGE));
        assert_eq!(gauge_for(&reading("clock", "14:05", "Sun 8 Nov", NO_PROGRESS, false)), None);
    }

    #[test]
    fn a_pomodoro_wears_one_glass_for_focus_and_another_for_a_break() {
        for (detail, active) in [("focus", true), ("focus \u{b7} paused", false)] {
            assert_eq!(
                gauge_for(&reading("pomodoro", "18:42", detail, 0.25, active)),
                Some(POMODORO_FOCUS_GAUGE),
                "{detail}"
            );
        }
        for detail in ["break", "break \u{b7} paused", "long break", "long break \u{b7} paused"] {
            assert_eq!(
                gauge_for(&reading("pomodoro", "03:10", detail, 0.37, false)),
                Some(POMODORO_BREAK_GAUGE),
                "{detail}"
            );
        }
        assert_ne!(POMODORO_FOCUS_GAUGE.glass, POMODORO_BREAK_GAUGE.glass);
        assert_ne!(POMODORO_FOCUS_GAUGE.fill, POMODORO_BREAK_GAUGE.fill);
    }

    #[test]
    fn only_a_timer_that_has_rung_lights_its_rim() {
        assert_eq!(gauge_for(&reading("timer", "00:00", "done", 1.0, true)), Some(TIMER_DONE_GAUGE));
        assert_eq!(
            gauge_for(&reading("timer", "04:10", "running", 0.70, true)),
            Some(TIMER_GAUGE),
            "a running timer is marked too, and has not rung"
        );
        assert_eq!(gauge_for(&reading("timer", "10:00", "paused", 0.33, false)), Some(TIMER_GAUGE));
        assert_eq!(gauge_for(&reading("timer", "15:00", "paused", 0.0, false)), Some(TIMER_GAUGE));
        assert!(TIMER_DONE_GAUGE.edge.is_some());
        for (name, gauge) in GAUGES.iter().filter(|(name, _)| *name != "timer done") {
            assert_eq!(gauge.edge, None, "{name} lights a rim and has nothing to warn of");
        }
    }

    #[test]
    fn a_ring_fits_the_card_it_is_on_at_every_size() {
        for icon_size in doca_ipc::MIN_ICON_SIZE..=doca_ipc::MAX_ICON_SIZE {
            let side = Level::Minimal.width(Tile::Wide, icon_size) as f64;
            let height = height_of(icon_size) as f64;
            let (cx, cy, radius, thickness) = ring_in(side, height);
            let reach = radius + thickness / 2.0;
            assert!(
                cx - reach >= 1.0 && cx + reach <= side - 1.0 && cy - reach >= 1.0 && cy + reach <= height - 1.0,
                "{icon_size}: a ring of {reach:.1} round ({cx:.1}, {cy:.1}) leaves a {side}x{height} card"
            );
            assert!(radius - thickness / 2.0 > thickness, "{icon_size}: a ring with no middle to write in");
        }
    }

    /// Where a card puts ink, drawn at the middle of a surface with room
    /// round it: inside the card, in the tile around the card, and outside
    /// the tile altogether.
    fn card_ink(glass: Glass, composited: bool) -> (usize, usize, usize) {
        const MARGIN: i32 = 16;
        let (width, height) = (120, 48);
        let mut placed = (0.0, 0.0, 0.0, 0.0);
        let (data, stride) = pixels(width + MARGIN * 2, height + MARGIN * 2, |cr| {
            cr.translate(MARGIN as f64, MARGIN as f64);
            placed = card(cr, width as f64, height as f64, 10.0, glass, composited, |_, _, _| {});
        });
        let (x, y, card_width, card_height) = placed;
        let (mut on_card, mut round_card, mut off_tile) = (0, 0, 0);
        for row in 0..(height + MARGIN * 2) {
            for column in 0..(width + MARGIN * 2) {
                if data[row as usize * stride + column as usize * 4 + 3] == 0 {
                    continue;
                }
                let (tx, ty) = ((column - MARGIN) as f64, (row - MARGIN) as f64);
                let in_tile = (0.0..width as f64).contains(&tx) && (0.0..height as f64).contains(&ty);
                let in_card = (x..x + card_width).contains(&tx) && (y..y + card_height).contains(&ty);
                if !in_tile {
                    off_tile += 1;
                } else if in_card {
                    on_card += 1;
                } else {
                    round_card += 1;
                }
            }
        }
        (on_card, round_card, off_tile)
    }

    #[test]
    fn a_lifted_card_has_a_shadow_only_where_something_composites() {
        let lifted = Glass { lifted: true, ..WATER_GLASS };

        let (on_card, round_card, off_tile) = card_ink(lifted, true);
        assert!(on_card > 0, "the card was not drawn");
        assert!(round_card > 0, "a lifted card on a compositor has no shadow");
        assert_eq!(off_tile, 0, "the shadow spills out of the tile");

        let (on_card, round_card, off_tile) = card_ink(lifted, false);
        assert!(on_card > 0, "the card was not drawn");
        assert_eq!(round_card + off_tile, 0, "a shadow with no compositor to blend it");
    }

    #[test]
    fn a_card_that_is_not_lifted_is_the_whole_tile() {
        let mut placed = (0.0, 0.0, 0.0, 0.0);
        let _ = pixels(120, 48, |cr| {
            placed = card(cr, 120.0, 48.0, 10.0, WATER_GLASS, true, |_, _, _| {});
        });
        assert_eq!(placed, (0.0, 0.0, 120.0, 48.0));
        let (on_card, round_card, off_tile) = card_ink(WATER_GLASS, true);
        assert!(on_card > 0);
        assert_eq!(round_card + off_tile, 0, "a card with no shadow drew one");
    }

    /// A Claude card's shape, with no words on it, at the width the bar
    /// gives it at this level.
    fn claude_pixels(claude: &doca_ipc::Claude, level: Level) -> (Vec<u8>, usize, i32, i32) {
        let (width, height) = (level.width(Tile::Wide, ICON), ICON);
        let (data, stride) = pixels(width, height, |cr| {
            claude_card(cr, claude, level, ICON, width as f64, height as f64, false);
        });
        (data, stride, width, height)
    }

    /// How many pixels in this part of a card are the orange, and how many
    /// the orange gone hot. The card is opaque, so no unpremultiplying.
    fn oranges(
        (data, stride, width, height): &(Vec<u8>, usize, i32, i32),
        keep: impl Fn(i32, i32) -> bool,
    ) -> (usize, usize) {
        let (mut orange, mut hot) = (0, 0);
        for y in 0..*height {
            for x in 0..*width {
                if !keep(x, y) {
                    continue;
                }
                let at = y as usize * stride + x as usize * 4;
                let (blue, green, red) = (data[at] as i32, data[at + 1] as i32, data[at + 2] as i32);
                if red > 200 && (110..200).contains(&green) && blue < 130 {
                    orange += 1;
                } else if red > 220 && green < 110 && blue < 100 {
                    hot += 1;
                }
            }
        }
        (orange, hot)
    }

    #[test]
    fn a_share_is_ringed_on_the_square_and_a_pill_on_the_card() {
        let shared = usage("72%", "1.8M of 2.5M", 0.72, false, "");
        let spent = usage("2.4M", "last 5 hours", NO_PROGRESS, false, "");

        // The square: the ring goes all round, so its lower half has orange
        // in it; the mark of a square with no share stays in the top half.
        let lower = |_: i32, y: i32| y > ICON / 2 + 2;
        let (ring, _) = oranges(&claude_pixels(&shared, Level::Minimal), lower);
        let (none, _) = oranges(&claude_pixels(&spent, Level::Minimal), lower);
        assert!(ring > 0, "a share on the square has no ring");
        assert_eq!(none, 0, "a square with no share drew a ring round nothing");

        // The card: right of the mark there is only the pill to be orange.
        let clear_of_the_mark = |x: i32, _: i32| x > ICON * 3 / 4;
        for level in [Level::Full, Level::Compact] {
            let (pill, _) = oranges(&claude_pixels(&shared, level), clear_of_the_mark);
            let (none, _) = oranges(&claude_pixels(&spent, level), clear_of_the_mark);
            assert!(pill > 0, "{level:?}: a share on the card has no pill");
            assert_eq!(none, 0, "{level:?}: a card with no share drew a pill for nothing");
        }
    }

    #[test]
    fn a_budget_nearly_spent_is_plain_on_the_card_itself() {
        let calm = usage("72%", "1.8M of 2.5M", 0.72, false, "");
        let near = usage("91%", "2.3M of 2.5M", 0.91, true, "");
        for level in [Level::Full, Level::Compact, Level::Minimal] {
            let (_, cool) = oranges(&claude_pixels(&calm, level), |_, _| true);
            let (_, lit) = oranges(&claude_pixels(&near, level), |_, _| true);
            assert_eq!(cool, 0, "{level:?}: a card nowhere near its budget looks alarmed");
            // The rim at least: all round the card, so more than its height.
            assert!(lit > ICON as usize * 2, "{level:?}: near the end of the budget the card looks the same");
        }
    }

    #[test]
    fn a_crowded_bar_takes_the_small_line_first_and_never_the_number() {
        let claude = usage("72%", "1.8M of 2.5M", 0.72, false, "20:30");
        let says = |level, square| claude_says(&claude, level, square);
        let line = Some("1.8M of 2.5M \u{b7} until 20:30".to_string());
        assert_eq!(says(Level::Full, false), ("72%".to_string(), line.clone()));
        assert_eq!(says(Level::Compact, false), ("72%".to_string(), None));
        assert_eq!(says(Level::Minimal, true), ("72%".to_string(), None), "the bare value is the number");
        assert_eq!(says(Level::Full, true), ("72%".to_string(), line), "a square with room keeps its line");

        let quiet = usage("2.7M", "", NO_PROGRESS, false, "");
        assert_eq!(claude_says(&quiet, Level::Full, false), ("2.7M".to_string(), None), "nothing to say is no line");
    }

    #[test]
    fn the_small_line_says_when_the_window_turns_and_only_once() {
        let under = |detail: &str, turns: &str| claude_under(&usage("", detail, NO_PROGRESS, false, turns));
        assert_eq!(under("today", "midnight"), "today \u{b7} until midnight");
        assert_eq!(under("session \u{b7} until 20:30", "20:30"), "session \u{b7} until 20:30");
        assert_eq!(under("last 7 days", ""), "last 7 days", "a window that rolls is given no time");
        assert_eq!(under("", "20:30"), "until 20:30");
    }

    /// The words on a Claude card are white, or white let down a little for
    /// the small line, and both stand 4.5:1 off every part of both glasses —
    /// which is the whole of their contrast, because the theme never reaches
    /// the card.
    #[test]
    fn the_words_on_a_claude_card_stand_off_its_glass() {
        use crate::theme::{contrast, over};
        let white = gdk::RGBA::WHITE;
        for glass in [CLAUDE_EMBER, CLAUDE_SMOKE] {
            for (r, g, b) in [glass.from, glass.to] {
                let pane = gdk::RGBA::new(r, g, b, 1.0);
                for (name, ink) in [("number", white), ("small line", faded(white, CLAUDE_QUIET))] {
                    let ratio = contrast(over(ink, pane), pane);
                    assert!(ratio >= 4.5, "the {name} stands {ratio:.2} off {glass:?}");
                }
            }
        }
    }

    #[test]
    fn a_bar_is_thick_enough_to_see_at_every_icon_size() {
        assert!(bar_height(24) >= 2.0);
        assert!(bar_height(96) > bar_height(24));
    }

    #[test]
    fn text_never_shrinks_to_the_point_of_being_invisible() {
        assert!(text_size(24, 0.21) >= 7.0);
        assert!(text_size(96, 0.30) > text_size(24, 0.30));
    }
}

