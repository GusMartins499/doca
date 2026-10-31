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
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, outer_w, outer_h).ok()?;
        {
            let cr = cairo::Context::new(&surface).ok()?;
            let card = gtk::DrawingArea::new();
            card.set_widget_name("ground");
            let context = card.style_context();
            let (w, h) = (outer_w as f64, outer_h as f64);
            gtk::render_background(&context, &cr, 0.0, 0.0, w, h);
            gtk::render_frame(&context, &cr, 0.0, 0.0, w, h);
            cr.translate(CARRY_PAD as f64, CARRY_PAD as f64);
            self.root.draw(&cr);
        }
        gdk::pixbuf_get_from_surface(&surface, 0, 0, outer_w, outer_h)
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
        Ok(Body::Simple(simple)) => draw_simple(&simple, look, area, cr, width, height),
        Ok(Body::Water(water)) => draw_water(&water, look, area, cr, width, height),
        Ok(Body::Music(music)) => draw_music(&music, look, area, cr, width, height),
        Ok(Body::Note(note)) => draw_note(&note, look, area, cr, width, height),
        Ok(Body::Calendar(calendar)) => draw_calendar(&calendar, look, area, cr, width, height),
        // Two lines and a bar: the simple tile, which is all the usage tile
        // is. The breakdown it carries is the panel's.
        Ok(Body::Claude(claude)) => draw_simple(
            &Simple { label: claude.label, detail: claude.detail, progress: claude.progress, active: claude.near },
            look,
            area,
            cr,
            width,
            height,
        ),
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
        Ok(Body::Claude(claude)) => match (claude.label.is_empty(), claude.detail.is_empty()) {
            (false, false) => format!("{name}: {}, {}", claude.label, claude.detail),
            (false, true) => format!("{name}: {}", claude.label),
            (true, false) => format!("{name}: {}", claude.detail),
            (true, true) => name,
        },
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
        groove(cr, 0.0, y + GAP, width, thickness, fraction, look.palette);
    }
}

/// Water: a bottle that fills, and what is in it.
///
/// A bottle and not a ring, because a ring is a share and water is a
/// quantity: you look at a bottle on a desk and know how much is left
/// without reading a number off it. The fill is the day against the goal and
/// the number beside it is the day in millilitres, so the glance and the
/// reading answer the same question at two different speeds.
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

    // The bottle keeps its own proportions whatever the tile is doing: it is
    // a picture of a bottle, and a bottle stretched to fill a box stops being
    // one.
    let bottle_height = (height * 0.88).max(1.0);
    let bottle_width = (bottle_height * 0.46).min(width);
    let gap = (look.icon_size as f64 * 0.14).max(3.0);
    let top = (height - bottle_height) / 2.0;

    // The water first and the glass over it, so the outline stays crisp
    // however full the bottle is. Stroked first, the inner half of the line
    // is painted over and a full bottle loses its edge.
    if fraction > 0.0 {
        let _ = cr.save();
        bottle(cr, 0.0, top, bottle_width, bottle_height);
        // `clip` and not `clip_preserve`: the bottle has to stop being the
        // current path once it is the clip, or the fill below adds the water
        // to it and paints the pair — which is a bottle full at every level,
        // and was.
        cr.clip();
        let fill = look.palette.fill;
        cr.set_source_rgba(fill.red(), fill.green(), fill.blue(), fill.alpha());
        let surface = top + bottle_height * (1.0 - fraction);
        cr.rectangle(0.0, surface, bottle_width, top + bottle_height - surface);
        let _ = cr.fill();
        let _ = cr.restore();
    }

    bottle(cr, 0.0, top, bottle_width, bottle_height);
    let track = look.palette.track;
    cr.set_source_rgba(track.red(), track.green(), track.blue(), track.alpha());
    cr.set_line_width((look.icon_size as f64 * 0.045).max(1.0));
    let _ = cr.stroke();

    // At the narrowest the bar ever squeezes a tile, the bottle is the whole
    // of it: the fill still says how the day is going, and a number in the
    // room left would be two characters of nothing.
    if look.level == Level::Minimal {
        return;
    }

    let text_left = bottle_width + gap;
    let room = (width - text_left).max(0.0);
    if room <= 0.0 {
        return;
    }

    let amount = line(
        area,
        &compact_ml(water.drunk),
        text_size(look.icon_size, 0.34),
        true,
        room,
    );
    // The goal under the amount, and only when the tile is drawing whole —
    // it is the one thing here the fill already says.
    let against = (look.level == Level::Full).then(|| {
        line(
            area,
            &if done {
                "done".to_string()
            } else {
                format!("of {}", compact_ml(water.goal))
            },
            text_size(look.icon_size, 0.21),
            false,
            room,
        )
    });

    let amount_height = amount.pixel_extents().1.height() as f64;
    let against_height = against
        .as_ref()
        .map(|layout| layout.pixel_extents().1.height() as f64)
        .unwrap_or(0.0);
    let mut y = ((height - amount_height - against_height) / 2.0).max(0.0);

    // A day that reached its goal says so in the colour the water is drawn
    // in, and says it by standing still. A tile that blinks to be noticed is
    // a tile you end up covering up.
    let ink = if done { look.palette.fill } else { look.palette.label };
    paint(cr, &amount, text_left, y, ink);
    y += amount_height;
    if let Some(against) = against {
        paint(cr, &against, text_left, y, look.palette.detail);
    }
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
    paint(cr, &weekday, 0.0, y, look.palette.fill);
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
                    "<b>{}</b> {}",
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
            look.palette.fill
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
    let cover = (!music.art.is_empty())
        .then(|| crate::dock::scaled_to_fill(&music.art, width, height))
        .flatten();

    if let Some(cover) = &cover {
        let _ = cr.save();
        // Centred and cropped rather than squashed: a square cover in a wide
        // tile has to lose its edges, not its proportions.
        let x = (width - cover.width() as f64) / 2.0;
        let y = (height - cover.height() as f64) / 2.0;
        cr.rectangle(0.0, 0.0, width, height);
        cr.clip();
        cr.set_source_pixbuf(cover, x, y);
        let _ = cr.paint();
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

/// The outline of a bottle, left on `cr` as the current path.
///
/// Left rather than stroked so the caller can both draw it and pour into it
/// without describing the shape twice.
fn bottle(cr: &cairo::Context, x: f64, y: f64, width: f64, height: f64) {
    let neck_width = width * 0.44;
    let neck_height = height * 0.20;
    let shoulder = height * 0.14;
    let corner = width * 0.26;
    let (left, right) = (x, x + width);
    let neck_left = x + (width - neck_width) / 2.0;
    let neck_right = x + (width + neck_width) / 2.0;
    let shoulder_top = y + neck_height;
    let body_top = shoulder_top + shoulder;
    let bottom = y + height;

    cr.new_path();
    cr.move_to(neck_left, y);
    cr.line_to(neck_right, y);
    cr.line_to(neck_right, shoulder_top);
    cr.curve_to(neck_right, body_top, right, shoulder_top, right, body_top);
    cr.line_to(right, bottom - corner);
    cr.curve_to(right, bottom, right, bottom, right - corner, bottom);
    cr.line_to(left + corner, bottom);
    cr.curve_to(left, bottom, left, bottom, left, bottom - corner);
    cr.line_to(left, body_top);
    cr.curve_to(left, shoulder_top, neck_left, body_top, neck_left, shoulder_top);
    cr.close_path();
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
        font.set_weight(pango::Weight::Bold);
    }
    layout.set_font_description(Some(&font));
    layout.set_width(width.max(0.0) as i32 * pango::SCALE);
    layout.set_alignment(pango::Alignment::Center);
    // A track title is longer than any tile, and a tile that grew to fit one
    // would shove its neighbours along the bar.
    layout.set_ellipsize(pango::EllipsizeMode::End);
    layout.set_single_paragraph_mode(true);
    layout
}

fn paint(cr: &cairo::Context, layout: &pango::Layout, x: f64, y: f64, colour: gdk::RGBA) {
    cr.set_source_rgba(colour.red(), colour.green(), colour.blue(), colour.alpha());
    cr.move_to(x, y);
    pangocairo::functions::show_layout(cr, layout);
}

/// A bar in its groove: the whole width faintly, the share of it brightly.
fn groove(
    cr: &cairo::Context,
    x: f64,
    y: f64,
    width: f64,
    thickness: f64,
    fraction: f64,
    palette: Palette,
) {
    let track = palette.track;
    cr.set_source_rgba(track.red(), track.green(), track.blue(), track.alpha());
    cr.rectangle(x, y, width, thickness);
    let _ = cr.fill();

    if fraction <= 0.0 {
        return;
    }
    let fill = palette.fill;
    cr.set_source_rgba(fill.red(), fill.green(), fill.blue(), fill.alpha());
    cr.rectangle(x, y, width * fraction, thickness);
    let _ = cr.fill();
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

    /// The bottle holds as much water as the day has in it.
    ///
    /// Drawing is the one part of a tile no assertion about state can reach,
    /// and it is where the mistakes are: the first bottle written here was
    /// full at every level, because the outline was still the current path
    /// when the water was filled and Cairo painted the pair. Nothing but
    /// counting pixels would have said so.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn the_bottle_holds_as_much_as_the_day_has_in_it() {
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
            // Only the left third, where the bottle is: the amount beside it
            // is text, and text that got longer would read as more water.
            //
            // Any alpha at all, not a threshold: the outline is drawn in the
            // groove colour, which is a fourteen-percent white — asking for
            // half-opaque pixels finds the water and calls the glass nothing.
            let mut painted = 0usize;
            for row in 0..height as usize {
                for column in 0..(width as usize / 3) {
                    if data[row * stride + column * 4 + 3] > 0 {
                        painted += 1;
                    }
                }
            }
            painted
        };

        let (empty, quarter, half, full) = (poured(0), poured(500), poured(1000), poured(2000));

        assert!(empty > 0, "nothing was drawn at all");
        assert!(
            quarter > empty && half > quarter && full > half,
            "the bottle does not track the day: {empty} < {quarter} < {half} < {full}"
        );
        assert!(
            full > empty * 2,
            "a full bottle is barely fuller than an empty one: {empty} against {full}"
        );
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
            let fill = tile_look.palette.fill;
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
