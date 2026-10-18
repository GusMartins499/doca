use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use gtk::cairo;
use gtk::gdk_pixbuf;
use gtk::prelude::*;
use doca_ipc::{DockItem, DocaProxy};

use crate::magnify::{self, Placement};
use crate::motion::Life;

/// Room under the icons for the running-app dot.
pub const INDICATOR: i32 = 9;
const DOT_WIDTH: f64 = 6.0;
const DOT_HEIGHT: f64 = 3.0;

/// One icon, as the row needs it.
pub struct Entry {
    pub item: DockItem,
    source: Option<gdk_pixbuf::Pixbuf>,
    scaled: RefCell<HashMap<i32, gdk_pixbuf::Pixbuf>>,
    launched: Cell<Option<Instant>>,
    life: Cell<Life>,
}

impl Entry {
    fn from(item: &DockItem, largest: i32, arriving: bool) -> Self {
        Self {
            item: item.clone(),
            source: crate::dock::load_pixbuf(&item.icon, largest),
            scaled: RefCell::new(HashMap::new()),
            launched: Cell::new(None),
            life: Cell::new(if arriving { Life::arriving() } else { Life::settled() }),
        }
    }

    /// The same app again, with whatever changed about it — and the icon it
    /// already had.
    fn again(self, item: &DockItem) -> Self {
        Self {
            item: item.clone(),
            ..self
        }
    }

    fn at(&self, size: i32) -> Option<gdk_pixbuf::Pixbuf> {
        if let Some(kept) = self.scaled.borrow().get(&size) {
            return Some(kept.clone());
        }
        let made = self.source.as_ref()?.scale_simple(
            size,
            size,
            gdk_pixbuf::InterpType::Bilinear,
        )?;
        self.scaled.borrow_mut().insert(size, made.clone());
        Some(made)
    }
}

/// Something the row runs each frame, once the icons have been placed.
type Follower = Rc<RefCell<Option<Box<dyn Fn()>>>>;

/// One icon as it is drawn this instant.
///
/// Where the lens put it, the size it came out, and how far through arriving
/// or leaving it is. Everything the drawing needs — and everything the bar
/// needs, since the bar is painted around the icons as they are drawn. One
/// answer to where an icon is, rather than two that have to agree.
#[derive(Clone, Copy)]
struct Drawn {
    placed: Placement,
    presence: f64,
}

impl Drawn {
    /// Where an icon goes: the lens's answer, swollen by whatever bounce is
    /// in progress.
    ///
    /// `bounce` is a multiple of the size and `presence` how far through
    /// arriving the icon is — both 1.0 for an icon that is simply sitting
    /// there, which is every icon almost all of the time.
    fn of(
        centre: f64,
        pointer: Option<f64>,
        base: f64,
        scale: f64,
        presence: f64,
        bounce: f64,
    ) -> Self {
        let mut placed = magnify::place(centre, pointer, base, scale);
        placed.size *= bounce;
        Self { placed, presence }
    }

    /// The size the icon is painted at: the lens's answer, settled onto a
    /// step so the scaled picture is one worth keeping.
    fn size(&self, base: f64) -> i32 {
        magnify::settled(self.placed.size.round() as i32, base as i32)
    }

    /// How much of its own size an arriving or leaving icon is drawn at, if
    /// it is one. `None` is an icon that is simply there.
    fn growth(&self) -> Option<f64> {
        (self.presence < 1.0).then(|| crate::motion::entry_scale(self.presence))
    }

    /// The left and right the icon covers on the row's surface.
    fn edges(&self, base: f64) -> (f64, f64) {
        let half = self.size(base) as f64 * self.growth().unwrap_or(1.0) / 2.0;
        (self.placed.centre - half, self.placed.centre + half)
    }
}

/// Where the row sits when the pointer is elsewhere.
#[derive(Clone, Copy, Default)]
pub struct Rest {
    pub icon: f64,
    pub slot: f64,
    pub spacing: f64,
    pub margin: f64,
    pub scale: f64,
}

impl Rest {
    pub fn new(icon: i32, spacing: i32, padding: i32, scale: f64) -> Self {
        Self {
            icon: icon as f64,
            slot: (icon + padding * 2) as f64,
            spacing: spacing as f64,
            margin: magnify::edge_room(icon as f64, scale) as f64,
            scale,
        }
    }

    pub fn centre(&self, index: usize) -> f64 {
        magnify::base_centre(index, self.slot, self.spacing, self.margin)
    }

    /// The run the icons take at rest: the first icon's left edge to the last
    /// one's right, and nothing else.
    ///
    /// This is the place the row holds in the bar — the width the layout is
    /// given, and so the width of the bar at rest. It is not what the bar is
    /// *painted* around: that is wherever the icons are drawn this frame,
    /// which is this run while the pointer is away and wider than it while
    /// the lens is open. See `ground::around`.
    pub fn run_width(&self, count: usize) -> i32 {
        self.run(count as f64).ceil() as i32
    }

    /// The run a number of slots takes, fractions of one included — which
    /// is what a row with something arriving or leaving is made of.
    pub fn run(&self, slots: f64) -> f64 {
        magnify::run(slots, self.slot, self.spacing)
    }

    /// The surface the row is drawn on: the run, plus the room at either end
    /// that an icon pushed outwards by the lens is drawn in.
    ///
    /// Wider than the run, and on purpose — this is the room, and the only
    /// room, the lens has to push an end icon into. It stays: the bar is
    /// *painted* into it when the lens asks (`ground::around`), but nothing
    /// is *laid out* in it, because sizing the bar to it is what made the
    /// dock as wide as the screen with the icons adrift in the middle of it.
    /// Plank splits the two the same way — `DockWidth` is the whole monitor
    /// and `DockBackgroundWidth` the part that gets painted.
    #[cfg(test)]
    pub fn width(&self, count: usize) -> i32 {
        self.around(self.run_width(count))
    }

    /// The surface around a run of any width, a part-filled one included.
    pub fn around(&self, run: i32) -> i32 {
        if run <= 0 {
            return 0;
        }
        run + (self.margin * 2.0).ceil() as i32
    }

    /// How tall the row is when nothing is magnified — the height the bar has.
    pub fn resting_height(&self) -> i32 {
        self.icon.ceil() as i32 + INDICATOR
    }

    /// How tall the row needs to be to draw a magnified icon.
    pub fn height(&self) -> i32 {
        (self.icon * self.scale).ceil() as i32 + INDICATOR
    }

    /// The room above the bar that a magnified icon rises into.
    pub fn overhead(&self) -> i32 {
        self.height() - self.resting_height()
    }
}

/// The icon row: one widget that draws every icon itself.
///
/// A widget per icon cannot do this. Changing an icon's size changes what it
/// asks of its parent, so every frame of the lens renegotiated the layout of
/// the whole bar — which caps the animation at around thirty frames a second
/// with thirty icons, and makes the neighbours shuffle as the sizes settle.
/// Here the sizes are a drawing, not a layout: the row's own size never
/// changes while the lens is open.
#[derive(Clone)]
pub struct Row {
    pub area: gtk::DrawingArea,
    /// Holds the row's place in the bar, at the height the bar should be.
    ///
    /// The drawing itself lives above the bar, where it has room to grow, so
    /// something has to stand in the bar's layout and say how much width the
    /// icons take and how tall the bar needs to be for them.
    pub perch: gtk::Box,
    entries: Rc<RefCell<Vec<Entry>>>,
    rest: Rc<Cell<Rest>>,
    /// How many icons are staying — everything in `entries` but those on
    /// their way out. The room the bar ends up holding, once nothing moves.
    settled: Rc<Cell<usize>>,
    /// Where the pointer is: what the lens is travelling towards.
    pointer: Rc<Cell<Option<f64>>>,
    /// Where the lens is aimed, which is where the pointer is except when the
    /// pointer did not travel to get there. Advanced once a frame.
    lens: Rc<Cell<Option<f64>>>,
    lens_at: Rc<Cell<Instant>>,
    largest: Rc<Cell<i32>>,
    /// When the pointer arrived or left, and which of the two it was.
    hovering: Rc<Cell<bool>>,
    hover_since: Rc<Cell<Instant>>,
    ticking: Rc<Cell<bool>>,
    /// Something to run each frame, besides redrawing the icons.
    ///
    /// The bar's background is painted around what the row draws, and it is
    /// painted on the window rather than on the row's own surface — so when
    /// the lens moves an icon at an end of the row, the window has to be told
    /// that the place its background goes has moved. This is how it is told:
    /// one call a frame, off the clock the icons are already on.
    following: Follower,
    dots: Rc<RefCell<(gdk::RGBA, gdk::RGBA)>>,
    /// The icon the keyboard is on, by id, while the dock has the keyboard.
    ///
    /// By id and not by place, because the row is filled again whenever a
    /// window opens or closes: an app that opened to the left of the one
    /// being pointed at must not move the ring onto its neighbour.
    ringed: Rc<RefCell<Option<String>>>,
    ring: Rc<Cell<gdk::RGBA>>,
    proxy: Rc<RefCell<Option<Rc<DocaProxy<'static>>>>>,
}

impl Row {
    /// Put the drawing above the bar, lined up with the place it holds there.
    pub fn stage_on(&self, stage: &gtk::Overlay) {
        self.area.set_halign(gtk::Align::Start);
        self.area.set_valign(gtk::Align::Start);
        stage.add_overlay(&self.area);

        let row = self.clone();
        stage.connect_get_child_position(move |_, child| {
            if child != &row.area.clone().upcast::<gtk::Widget>() {
                return None;
            }
            let perch = row.perch.allocation();
            let rest = row.rest.get();
            // The perch holds the run of icons and nothing more, so the room
            // the lens needs hangs off both of its ends — out of the bar and
            // into the window, which is wider than the bar by exactly that.
            let margin = rest.margin as i32;
            Some(gdk::Rectangle::new(
                perch.x() - margin,
                perch.y() - rest.overhead(),
                perch.width() + margin * 2,
                rest.height(),
            ))
        });
    }

    pub fn new() -> Self {
        let area = gtk::DrawingArea::new();
        area.set_widget_name("row");
        area.add_events(
            gdk::EventMask::POINTER_MOTION_MASK
                | gdk::EventMask::BUTTON_PRESS_MASK
                | gdk::EventMask::LEAVE_NOTIFY_MASK
                | gdk::EventMask::ENTER_NOTIFY_MASK,
        );

        // So that the keyboard can stand on it: the row is one widget for all
        // the icons, and it holds GTK's focus whenever an icon has the
        // keyboard — which is also what tells a screen reader that the icon
        // it is reading out is the one that is focused. A click does not
        // take focus with it: a drawing area never asks for focus on its own.
        area.set_can_focus(true);

        let perch = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        perch.set_widget_name("perch");

        let row = Self {
            area,
            perch,
            entries: Rc::new(RefCell::new(Vec::new())),
            rest: Rc::new(Cell::new(Rest::default())),
            settled: Rc::new(Cell::new(0)),
            pointer: Rc::new(Cell::new(None)),
            lens: Rc::new(Cell::new(None)),
            lens_at: Rc::new(Cell::new(Instant::now())),
            largest: Rc::new(Cell::new(0)),
            hovering: Rc::new(Cell::new(false)),
            hover_since: Rc::new(Cell::new(Instant::now() - crate::motion::ZOOM)),
            ticking: Rc::new(Cell::new(false)),
            following: Rc::new(RefCell::new(None)),
            ringed: Rc::new(RefCell::new(None)),
            ring: Rc::new(Cell::new(crate::theme::ring(crate::theme::DEFAULT))),
            proxy: Rc::new(RefCell::new(None)),
            dots: Rc::new(RefCell::new((
                gdk::RGBA::new(1.0, 1.0, 1.0, 0.45),
                gdk::RGBA::new(0.298, 0.553, 1.0, 1.0),
            ))),
        };

        let drawing = row.clone();
        row.area.connect_draw(move |area, cr| {
            drawing.draw(area, cr);
            glib::Propagation::Proceed
        });

        row
    }

    /// Take a new list of items, keeping what the old one already knew.
    ///
    /// The dock is rebuilt whenever a window opens, closes or takes focus —
    /// several times a minute in ordinary use. Starting from scratch each time
    /// meant reloading every icon from the theme, and forgetting where the
    /// pointer was: the lens collapsed mid-hover every time something, three
    /// windows away, took focus. What changed is which apps are running, not
    /// where the pointer is or what an icon looks like.
    pub fn fill(&self, items: &[DockItem], rest: Rest) {
        let largest = (rest.icon * rest.scale).ceil() as i32;
        let resized = largest != self.largest.replace(largest);
        // A row that had nothing on it is not a row something arrived at.
        // Without this every icon fades in together whenever the dock starts
        // or the icon theme changes — an animation that announces nothing.
        //
        // A row whose icons all changed size is not one either, but it is
        // not thrown away to say so: the apps that stay keep their place and
        // their life, and only their pictures are loaded again. Throwing it
        // away is what made switching to a dock with more apps on it — which
        // can take the icons down a size to fit — a bar emptied and refilled
        // in one frame, rather than the same switch any other dock gets.
        let announcing = !self.entries.borrow().is_empty();

        let mut old: Vec<Option<Entry>> =
            self.entries.borrow_mut().drain(..).map(Some).collect();
        let mut was_at: HashMap<String, usize> = old
            .iter()
            .enumerate()
            .filter_map(|(at, entry)| entry.as_ref().map(|e| (e.item.id.clone(), at)))
            .collect();

        // In the order given, taking what each id already had.
        let mut staying: Vec<Entry> = Vec::with_capacity(items.len());
        let mut survived = vec![false; old.len()];
        for item in items {
            let Some(at) = was_at.remove(&item.id) else {
                staying.push(Entry::from(item, largest, announcing));
                continue;
            };
            let entry = old[at].take().expect("one id, one entry");
            survived[at] = true;
            let entry = if resized || entry.item.icon != item.icon {
                // The same app wearing a different picture, or the same
                // picture at a different size: a load, but not an arrival —
                // it never left the bar, and it goes on from where it was.
                let loaded = Entry::from(item, largest, false);
                loaded.life.set(entry.life.get());
                loaded
            } else {
                entry.again(item)
            };
            if entry.life.get().leaving() {
                // Closed and reopened before its icon finished going: it
                // comes back from the size it had shrunk to.
                entry.life.set(entry.life.get().reversed());
            }
            staying.push(entry);
        }

        // Whatever the new list did not claim is on its way out. It holds its
        // slot and its place in the row while it shrinks, and lets go of the
        // space only once it is gone — which is `step`'s job, a frame at a
        // time, not this one's.
        let departing: Vec<(usize, Entry)> = old
            .iter_mut()
            .enumerate()
            .filter_map(|(at, entry)| {
                let entry = entry.take()?;
                if entry.life.get().gone() {
                    return None;
                }
                if !entry.life.get().leaving() {
                    entry.life.set(entry.life.get().reversed());
                }
                // Back between the icons it was between: after as many of its
                // old neighbours as are still on the row.
                Some((survived[..at].iter().filter(|kept| **kept).count(), entry))
            })
            .collect();

        let mut row = Vec::with_capacity(staying.len() + departing.len());
        let mut departing = departing.into_iter().peekable();
        for (index, entry) in staying.into_iter().enumerate() {
            while departing.peek().is_some_and(|(anchor, _)| *anchor <= index) {
                row.push(departing.next().expect("peeked").1);
            }
            row.push(entry);
        }
        row.extend(departing.map(|(_, entry)| entry));
        *self.entries.borrow_mut() = row;

        self.rest.set(rest);
        self.settled.set(items.len());
        self.hold_room();
        self.speak();
        self.area.queue_draw();
        self.animate();
    }

    /// Tell a screen reader what the row is saying now.
    ///
    /// The row is one drawing, so it is one accessible object. At rest it is
    /// "Dock", described by what is on it in order, the way the keys count.
    /// While the keyboard is on an icon the object takes that icon's name and
    /// says where it is — the object a reader announces when focus lands, and
    /// again whenever its name changes, which is every arrow. An accessible
    /// child per icon would say the same with more machinery; this is what
    /// the one widget the row is can say on its own.
    fn speak(&self) {
        let Some(accessible) = self.area.accessible() else {
            return;
        };
        use atk::prelude::*;
        let items = self.items();
        let ringed = self.ringed.borrow().clone();
        match ringed.and_then(|id| items.iter().position(|item| item.id == id)) {
            Some(index) => {
                accessible.set_name(&spoken(&items[index..=index]));
                accessible.set_description(&format!("Dock, {} of {}", index + 1, items.len()));
            }
            None => {
                accessible.set_name("Dock");
                accessible.set_description(&spoken(&items));
            }
        }
    }

    /// What is on the row and staying, in the order it is shown — the icons
    /// the keyboard can stand on. One on its way out is a picture, not a
    /// place to go.
    pub fn items(&self) -> Vec<DockItem> {
        self.entries
            .borrow()
            .iter()
            .filter(|entry| !entry.life.get().leaving())
            .map(|entry| entry.item.clone())
            .collect()
    }

    /// Put the keyboard's ring on this icon, or take it off the row.
    pub fn ring(&self, id: Option<&str>) {
        let id = id.map(str::to_string);
        if *self.ringed.borrow() == id {
            return;
        }
        *self.ringed.borrow_mut() = id;
        self.speak();
        self.area.queue_draw();
    }

    /// The icon the ring is on, if it is on one.
    #[cfg(test)]
    pub fn ringed(&self) -> Option<String> {
        self.ringed.borrow().clone()
    }

    pub fn ring_colour(&self, colour: gdk::RGBA) {
        self.ring.set(colour);
    }

    /// Give the bar the room the icons take this instant, and no more.
    ///
    /// It used to be handed the room for the icons that were staying, all at
    /// once, with a departing icon drawn in the margin the row keeps for the
    /// lens and the run centred so the icons did not see the jump. The icons
    /// did not; everything else on the bar did. The widget tiles are laid out
    /// after the row, and a bar that lost a slot in one frame moved them half
    /// a slot in that frame — on a dock switch, where several icons go and
    /// come at once, the tiles leapt sideways while the icons glided.
    ///
    /// So the room follows the icons, a fraction of a slot at a time, and is
    /// let go of only as the icon that held it shrinks: the bar narrows as
    /// one movement with the row, and what sits beside the row slides with
    /// it. It is a layout a frame, but only for the frames something is
    /// arriving or leaving — the lens, which is open for as long as the
    /// pointer is on the bar, still never touches the layout.
    fn hold_room(&self) {
        let rest = self.rest.get();
        let present: f64 = self
            .entries
            .borrow()
            .iter()
            .map(|entry| entry.life.get().presence())
            .sum();
        let run = rest.run(present).ceil() as i32;
        if self.perch.size_request() == (run, rest.resting_height()) {
            return;
        }
        self.perch.set_size_request(run, rest.resting_height());
        self.area.set_size_request(rest.around(run), rest.height());
    }

    /// Whether anything on the row is still arriving or leaving.
    pub fn changing(&self) -> bool {
        self.entries
            .borrow()
            .iter()
            .any(|entry| entry.life.get().moving())
    }

    /// Let go of what has finished leaving, and hand back the room it held.
    ///
    /// The clock does this a frame at a time; this is for whoever has to know
    /// the row is done *now* — the bar, measuring itself once a dock switch
    /// is over, cannot wait for a frame that a hidden window may never draw.
    pub fn settle(&self) {
        self.sweep();
        self.hold_room();
    }

    /// How much wider the row would be with everything arriving already
    /// there and everything leaving not yet gone.
    ///
    /// The widest the row can get in the middle of a change: an icon arriving
    /// eases out and one leaving eases in, so halfway through a swap the two
    /// together take more than either end of it. The window has to be that
    /// wide for the whole of it, or the bar would outgrow the window and be
    /// pushed off centre.
    pub fn unfolding(&self) -> i32 {
        let rest = self.rest.get();
        let all = self.entries.borrow().len();
        (rest.run_width(all) - self.perch.size_request().0).max(0)
    }

    /// Forget every icon, so the next fill loads them again.
    ///
    /// `fill` keeps an entry whose icon *name* has not changed, which is what
    /// stops the row reloading thirty icons every time a window takes focus.
    /// A new icon theme changes none of those names and all of those pictures,
    /// so it is the one case where the cache has to be thrown away.
    pub fn reload_icons(&self) {
        self.entries.borrow_mut().clear();
    }

    pub fn serve(&self, proxy: Rc<DocaProxy<'static>>) {
        *self.proxy.borrow_mut() = Some(proxy);
    }

    pub fn proxy(&self) -> Rc<DocaProxy<'static>> {
        self.proxy
            .borrow()
            .clone()
            .expect("the row is wired before it is drawn on")
    }

    pub fn dot_colours(&self, running: gdk::RGBA, active: gdk::RGBA) {
        *self.dots.borrow_mut() = (running, active);
    }

    /// Where every icon sits at rest this instant, arrivals and departures
    /// included.
    ///
    /// `Rest::centre` is the settled answer and the common one; this is what
    /// the row actually draws and is clicked on, which differ only while
    /// something is joining or leaving.
    fn centres(&self) -> Vec<f64> {
        let rest = self.rest.get();
        let entries = self.entries.borrow();
        let presence: Vec<f64> = entries.iter().map(|entry| entry.life.get().presence()).collect();
        // The row is in this state every frame but the few after something
        // opened or closed, and the answer there is exactly where the icons
        // rest — taken from the same place the rest of the row takes it,
        // rather than arrived at again by a different route.
        if entries.len() == self.settled.get() && presence.iter().all(|part| *part >= 1.0) {
            return (0..entries.len()).map(|index| rest.centre(index)).collect();
        }
        // The room is what `hold_room` gave the bar, which is the icons as
        // they are this instant — so the run fills it, and the bar moves with
        // the row rather than the row moving inside a bar that jumped.
        let room: f64 = presence.iter().sum();
        magnify::centres(&presence, rest.slot, rest.spacing, rest.margin, room)
    }

    /// Note where the pointer is. Redrawing waits for the next frame.
    ///
    /// Leaving keeps the last position: the lens closes over the icon it was
    /// open on, rather than snapping shut the moment the pointer is gone.
    pub fn aim(&self, pointer: Option<f64>) {
        match pointer {
            Some(x) => {
                // A shut lens aims wherever it likes: there is nothing on
                // screen to jump, and travelling to the pointer while closed
                // would only mean arriving late as it opens. An open one
                // travels, and `step` is what carries it.
                if self.scale_now() <= 1.0 {
                    self.lens.set(Some(x));
                    self.lens_at.set(Instant::now());
                }
                self.pointer.set(Some(x));
                self.hover(true);
            }
            None => self.hover(false),
        }
        self.animate();
    }

    /// Where the lens is aimed this instant.
    ///
    /// Everything the row draws, and everything it is clicked on, asks this
    /// rather than the pointer — the same reason `scale_now` exists. The two
    /// agree except in the frames after a pointer appeared somewhere it had
    /// not travelled to, and in those frames what you hit has to be what you
    /// see.
    fn lens(&self) -> Option<f64> {
        self.lens.get().or_else(|| self.pointer.get())
    }

    /// Carry the lens one frame closer to the pointer.
    ///
    /// Called from the frame clock and nowhere else: reading the lens must
    /// not move it, or the two or three places that ask per frame would each
    /// advance it and the travel would depend on how often it was looked at.
    fn step(&self, now: Instant) {
        let Some(pointer) = self.pointer.get() else {
            return;
        };
        let since = now.saturating_duration_since(self.lens_at.replace(now));
        let lens = self.lens.get().unwrap_or(pointer);
        self.lens.set(Some(crate::motion::aimed(lens, pointer, since)));
    }

    /// Let go of the icons whose leaving is over, and of the bounces that
    /// have finished.
    ///
    /// Until this runs a departing icon is still in the row, holding the
    /// fraction of a slot its shrinking left it — which is the whole of what
    /// "and only then releases the space" means.
    ///
    /// Finished bounces are let go here, on the clock, rather than wherever
    /// they happened to be noticed: the row is asked where its icons are
    /// twice a frame now — once to draw them and once to paint the bar around
    /// them — and an answer that clears state as a side effect is an answer
    /// that depends on who asked first.
    fn sweep(&self) {
        let mut entries = self.entries.borrow_mut();
        entries.retain(|entry| !entry.life.get().gone());
        for entry in entries.iter() {
            if entry
                .launched
                .get()
                .is_some_and(|started| !crate::motion::is_launching_since(started))
            {
                entry.launched.set(None);
            }
        }
    }

    fn hover(&self, hovering: bool) {
        if self.hovering.replace(hovering) == hovering {
            return;
        }
        let elapsed = self.hover_since.get().elapsed();
        self.hover_since.set(
            Instant::now() - crate::motion::reversed_start(elapsed, crate::motion::ZOOM),
        );
    }

    /// The magnification in force this instant, part-way through opening or
    /// closing. Everything the row draws and everything it is clicked on goes
    /// through here, so what you hit is what you see.
    fn scale_now(&self) -> f64 {
        let rest = self.rest.get();
        let progress =
            crate::motion::zoom_progress(self.hover_since.get().elapsed(), self.hovering.get());
        1.0 + (rest.scale - 1.0) * progress
    }

    /// The item at this place in the row, the way `index_of` counts.
    pub fn item(&self, index: usize) -> Option<DockItem> {
        self.entries.borrow().get(index).map(|entry| entry.item.clone())
    }

    /// Where this item is in the row, if it is in it.
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.entries.borrow().iter().position(|entry| entry.item.id == id)
    }

    pub fn launch(&self, index: usize) {
        if let Some(entry) = self.entries.borrow().get(index) {
            entry.launched.set(Some(Instant::now()));
        }
        self.animate();
    }

    /// Redraw on the compositor's own clock, and stop when there is nothing
    /// left moving — a dock that wakes sixty times a second while it sits
    /// still is paid for in battery.
    fn animate(&self) {
        if self.ticking.replace(true) {
            return;
        }
        let row = self.clone();
        self.area.add_tick_callback(move |area, _| {
            row.step(Instant::now());
            row.sweep();
            row.hold_room();
            area.queue_draw();
            if let Some(follow) = row.following.borrow().as_ref() {
                follow();
            }
            if row.resting() {
                row.ticking.set(false);
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    fn resting(&self) -> bool {
        let entries = self.entries.borrow();
        let launching = entries
            .iter()
            .any(|entry| entry.launched.get().is_some_and(crate::motion::is_launching_since));
        let joining = entries.iter().any(|entry| entry.life.get().moving());
        !launching && !joining && !self.hovering.get() && self.scale_now() <= 1.0
    }

    /// Which icon is under `x`, by what is drawn rather than by what rests there.
    pub fn at(&self, x: f64) -> Option<usize> {
        let rest = self.rest.get();
        let scale = self.scale_now();
        // Aimed where the lens is, asked about where the click was. The two
        // are the same once the lens has caught up, and while it has not, the
        // icons are where the lens put them and not where the pointer is.
        let pointer = self.lens().or(Some(x));
        let centres = self.centres();
        let entries = self.entries.borrow();
        (0..entries.len()).find(|index| {
            // An icon on its way out is a picture, not a target: clicking
            // where an app used to be must not launch it again.
            if entries[*index].life.get().leaving() {
                return false;
            }
            let placed = magnify::place(centres[*index], pointer, rest.icon, scale);
            (x - placed.centre).abs() <= (placed.size / 2.0).max(rest.slot / 2.0)
        })
    }

    /// Where an icon is drawn, for something else to point at.
    pub fn rect_of(&self, index: usize) -> (i32, i32) {
        let rest = self.rest.get();
        let Some(centre) = self.centres().get(index).copied() else {
            return (0, 0);
        };
        let presence = self
            .entries
            .borrow()
            .get(index)
            .map(|entry| entry.life.get().presence())
            .unwrap_or(1.0);
        let placed = magnify::place(centre, self.lens(), rest.icon, self.scale_now());
        let size = placed.size * crate::motion::entry_scale(presence);
        ((placed.centre - size / 2.0) as i32, size as i32)
    }

    /// The room a magnified icon rises into, above the bar.
    pub fn overhead(&self) -> i32 {
        self.rest.get().overhead()
    }

    /// The room a magnified icon spreads into, at either end of the row.
    ///
    /// This is how much wider than the bar the window is made, and it is
    /// still needed for exactly that: the window is the surface everything is
    /// drawn on, the icons at the ends are drawn out here, and the bar's
    /// background is painted out here with them. What changed is only who
    /// covers it — the bar reaches into it now instead of leaving it as air
    /// (`ground::around`), so an icon drawn here has its background under it.
    /// The bar's *layout* still stops at the run, and the strut still
    /// reserves the bar and not this: a magnification nobody is looking at
    /// must not hold screen back.
    pub fn margin(&self) -> i32 {
        self.rest.get().margin as i32
    }

    /// What the pointer is on right now, if it is on the row at all.
    ///
    /// The last position is kept after the pointer leaves, so the lens can
    /// close over the icon it was open on — but a remembered position is not
    /// a pointer. Reading it as one left the label on screen, naming an icon
    /// nobody was pointing at, until something moved.
    pub fn hovered(&self) -> Option<(usize, DockItem)> {
        if !self.hovering.get() {
            return None;
        }
        self.item_at(self.pointer.get()?)
    }

    pub fn item_at(&self, x: f64) -> Option<(usize, DockItem)> {
        let index = self.at(x)?;
        let item = self.entries.borrow().get(index)?.item.clone();
        Some((index, item))
    }

    /// Every icon as it is drawn this instant, in the row's own coordinates.
    ///
    /// Read-only, and asked twice a frame: once to draw the icons and once to
    /// paint the bar around them. The bar cannot be painted around where the
    /// icons rest — the lens pushes the end ones out past that — so it is
    /// painted around this, and there is one answer rather than two.
    fn drawn(&self) -> Vec<Drawn> {
        let rest = self.rest.get();
        let pointer = self.lens();
        let scale = self.scale_now();
        let centres = self.centres();
        self.entries
            .borrow()
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                // A bounce is drawn, so the bar is painted around it too: the
                // alternative is an icon that swells out of its own
                // background on every click.
                let bounce = entry
                    .launched
                    .get()
                    .map(|started| started.elapsed())
                    .filter(|elapsed| crate::motion::is_launching(*elapsed))
                    .map(crate::motion::launch_scale)
                    .unwrap_or(1.0);
                Drawn::of(
                    centres[index],
                    pointer,
                    rest.icon,
                    scale,
                    entry.life.get().presence(),
                    bounce,
                )
            })
            .collect()
    }

    /// The outermost edges the icons reach this instant, in the row's own
    /// coordinates — what the bar has to be painted around.
    ///
    /// `None` is a row with nothing on it: no footprint of its own, and a bar
    /// that stays exactly where the layout put it.
    pub fn drawn_edges(&self) -> Option<(f64, f64)> {
        let rest = self.rest.get();
        if rest.icon <= 0.0 {
            return None;
        }
        self.drawn()
            .iter()
            .map(|icon| icon.edges(rest.icon))
            .reduce(|(left, right), (l, r)| (left.min(l), right.max(r)))
    }

    /// What to run each frame once the icons have been placed.
    pub fn followed_by(&self, follow: impl Fn() + 'static) {
        *self.following.borrow_mut() = Some(Box::new(follow));
    }

    fn draw(&self, area: &gtk::DrawingArea, cr: &cairo::Context) {
        let rest = self.rest.get();
        if rest.icon <= 0.0 {
            return;
        }
        let bottom = area.allocated_height() as f64 - INDICATOR as f64;
        let (running_dot, active_dot) = *self.dots.borrow();

        let drawn = self.drawn();
        let entries = self.entries.borrow();
        for (index, icon) in drawn.iter().enumerate() {
            let size = icon.size(rest.icon);
            let Some(entry) = entries.get(index) else {
                continue;
            };
            let Some(pixbuf) = entry.at(size) else {
                continue;
            };
            let centre = icon.placed.centre;
            let x = centre - size as f64 / 2.0;
            let y = bottom - size as f64;
            // An icon that is arriving or leaving is scaled about the spot it
            // stands on, rather than drawn at a size of its own: every size
            // an icon takes is a surface to keep, and a sixth of a second of
            // them is a cache entry a frame for a picture nobody will ask for
            // again. It grows out of the bar and fades in with it.
            let growing = icon.growth();
            if let Some(growth) = growing {
                let _ = cr.save();
                cr.translate(centre, bottom);
                cr.scale(growth, growth);
                cr.translate(-centre, -bottom);
            }
            cr.set_source_pixbuf(&pixbuf, x.round(), y.round());
            let _ = if growing.is_some() {
                cr.paint_with_alpha(icon.presence)
            } else {
                cr.paint()
            };
            if growing.is_some() {
                let _ = cr.restore();
            }

            if !entry.item.windows.is_empty() {
                let dot = if entry.item.active {
                    active_dot
                } else {
                    running_dot
                };
                cr.set_source_rgba(
                    dot.red(),
                    dot.green(),
                    dot.blue(),
                    dot.alpha() * icon.presence,
                );
                cr.rectangle(
                    (centre - DOT_WIDTH / 2.0).round(),
                    bottom + 3.0,
                    DOT_WIDTH,
                    DOT_HEIGHT,
                );
                let _ = cr.fill();
            }
        }

        let ringed = self.ringed.borrow();
        let ringed = ringed
            .as_deref()
            .and_then(|id| entries.iter().position(|entry| entry.item.id == id));
        if let Some(icon) = ringed.and_then(|index| drawn.get(index)) {
            self.draw_ring(cr, icon, rest.icon, bottom, area.allocated_height() as f64);
        }
    }

    /// The keyboard's ring round one icon and its dot: a wash of the accent
    /// and a line of it, both rounded like the bar.
    ///
    /// Inside the row's own surface on every side, because there is nothing
    /// beyond it to draw on: with the lens off the row is exactly as tall as
    /// an icon and its dot, so a ring that stood outside the icon would be
    /// cut off at the top. It is drawn after the icons so the line is on top
    /// of the picture, and the wash is faint enough not to hide it.
    fn draw_ring(&self, cr: &cairo::Context, icon: &Drawn, base: f64, bottom: f64, height: f64) {
        let size = icon.size(base) as f64 * icon.growth().unwrap_or(1.0);
        let pad = crate::dock::ITEM_PADDING as f64;
        let line = 2.0;
        let left = icon.placed.centre - size / 2.0 - pad + line / 2.0;
        let top = (bottom - size - pad).max(line / 2.0);
        let width = size + pad * 2.0 - line;
        let tall = (height - line / 2.0) - top;
        let radius = (size * 0.2).min(10.0);
        let colour = self.ring.get();

        let _ = cr.save();
        cr.new_sub_path();
        cr.arc(left + width - radius, top + radius, radius, -std::f64::consts::FRAC_PI_2, 0.0);
        cr.arc(left + width - radius, top + tall - radius, radius, 0.0, std::f64::consts::FRAC_PI_2);
        cr.arc(left + radius, top + tall - radius, radius, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
        cr.arc(left + radius, top + radius, radius, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
        cr.close_path();
        cr.set_source_rgba(colour.red(), colour.green(), colour.blue(), colour.alpha() * 0.18);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(colour.red(), colour.green(), colour.blue(), colour.alpha());
        cr.set_line_width(line);
        let _ = cr.stroke();
        let _ = cr.restore();
    }
}

/// What is on the row, in words: each item's name in order, and how many
/// windows it has open when it has any.
pub fn spoken(items: &[DockItem]) -> String {
    items
        .iter()
        .map(|item| match item.windows.len() {
            0 => item.name.clone(),
            1 => format!("{} (open)", item.name),
            n => format!("{} ({n} windows)", item.name),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
pub mod tests {
    #[test]
    fn the_row_is_said_in_the_order_it_is_shown() {
        let item = |name: &str, windows: usize| DockItem {
            id: name.to_lowercase(),
            name: name.to_string(),
            icon: String::new(),
            pinned: true,
            windows: (0..windows as u32).collect(),
            active: false,
        };
        assert_eq!(
            super::spoken(&[item("Files", 0), item("Firefox", 2), item("Code", 1)]),
            "Files, Firefox (2 windows), Code (open)"
        );
    }

    use super::*;

    fn item(id: &str) -> DockItem {
        DockItem {
            id: id.into(),
            name: id.into(),
            icon: "application-x-executable".into(),
            pinned: true,
            windows: Vec::new(),
            active: false,
        }
    }

    const ICON: i32 = 48;
    const SCALE: f64 = 1.6;

    /// The bar is *given* the run and *painted* wherever the icons are, so
    /// the surface has to be the wider of the two. Sizing the bar to the
    /// surface instead is how the dock came to span the whole screen.
    #[test]
    fn the_row_is_drawn_on_more_room_than_the_bar_is_laid_out_with() {
        let rest = Rest::new(ICON, 4, 4, SCALE);

        let run = rest.run_width(10);
        let surface = rest.width(10);

        assert!(run < surface, "the lens has nowhere to push an end icon");
        assert_eq!(surface - run, rest.margin as i32 * 2);
    }

    #[test]
    fn a_dock_with_no_lens_asks_for_no_room_beyond_its_icons() {
        let rest = Rest::new(ICON, 4, 4, 1.0);

        assert_eq!(rest.run_width(10), rest.width(10));
    }

    #[test]
    fn the_run_grows_by_exactly_one_slot_per_icon() {
        let rest = Rest::new(ICON, 4, 4, SCALE);

        let one_more = rest.run_width(11) - rest.run_width(10);

        assert_eq!(one_more, (rest.slot + rest.spacing) as i32);
    }

    #[test]
    fn an_empty_row_takes_no_width_at_all_not_even_the_lens_room() {
        let rest = Rest::new(ICON, 4, 4, SCALE);

        assert_eq!(rest.run_width(0), 0);
        assert_eq!(rest.width(0), 0);
    }

    #[test]
    fn the_margin_is_wide_enough_for_the_furthest_the_lens_throws_an_icon() {
        let rest = Rest::new(ICON, 4, 4, SCALE);
        let centres: Vec<f64> = (0..10).map(|i| rest.centre(i)).collect();
        let surface = rest.width(10) as f64;

        // The lens is opened on each end in turn: the icon it pushes outwards
        // has to stay on the surface, or it is drawn cut off.
        for pointer in [centres[0], centres[9]] {
            for centre in &centres {
                let placed = magnify::place(*centre, Some(pointer), rest.icon, SCALE);
                let left = placed.centre - placed.size / 2.0;

                assert!(left >= 0.0, "an icon is pushed off the left of the surface");
                assert!(
                    left + placed.size <= surface,
                    "an icon is pushed off the right of the surface"
                );
            }
        }
    }

    /// The row as the dock really builds it, rather than with round numbers:
    /// `ground::REACH` is the bar's padding plus the frame inside a slot, and
    /// the agreement the whole design rests on only holds if the slot is the
    /// one that padding was worked out from.
    fn real(scale: f64) -> Rest {
        Rest::new(
            ICON,
            crate::dock::ITEM_SPACING,
            crate::dock::ITEM_PADDING,
            scale,
        )
    }

    /// Where the bar sits in the window with the row at rest.
    ///
    /// The layout gives the perch the run and the bar pads it on both sides,
    /// so this is the run plus `dock::BAR_PADDING` twice. The x is arbitrary
    /// and deliberately not zero: the background is worked out in the window's
    /// coordinates, and a bar at the origin hides a missing offset.
    fn resting_bar(rest: &Rest, count: usize) -> gdk::Rectangle {
        gdk::Rectangle::new(
            137,
            0,
            rest.run_width(count) + crate::dock::BAR_PADDING * 2,
            rest.resting_height(),
        )
    }

    /// Every icon's resting centre, in the same coordinates as the bar.
    fn centres_in(rest: &Rest, count: usize, bar: gdk::Rectangle) -> Vec<f64> {
        let shift = (bar.x() + crate::dock::BAR_PADDING) as f64 - rest.margin;
        (0..count).map(|index| rest.centre(index) + shift).collect()
    }

    fn row_of(centres: &[f64], pointer: Option<f64>, rest: &Rest) -> Vec<Drawn> {
        centres
            .iter()
            .map(|centre| Drawn::of(*centre, pointer, rest.icon, rest.scale, 1.0, 1.0))
            .collect()
    }

    fn reach_of(drawn: &[Drawn], rest: &Rest) -> Option<(f64, f64)> {
        drawn
            .iter()
            .map(|icon| icon.edges(rest.icon))
            .reduce(|(left, right), (l, r)| (left.min(l), right.max(r)))
    }

    /// The hinge of the whole thing: at rest the icons ask for exactly the bar
    /// the layout gave them, so the background painted around the drawing and
    /// the background painted around the layout are the same background. Get
    /// this wrong by a pixel and the bar twitches every time the lens closes.
    #[test]
    fn a_row_at_rest_reaches_exactly_the_two_edges_of_the_bar_it_was_given() {
        for count in 1..=30usize {
            let rest = real(magnify::DEFAULT_SCALE);
            let bar = resting_bar(&rest, count);
            let centres = centres_in(&rest, count, bar);

            let reach = reach_of(&row_of(&centres, None, &rest), &rest).expect("icons");

            assert_eq!(
                reach,
                (
                    (bar.x() + crate::ground::REACH) as f64,
                    (bar.x() + bar.width() - crate::ground::REACH) as f64
                ),
                "a row of {count} at rest does not line up with its own bar"
            );
            assert_eq!(crate::ground::around(bar, Some(reach)), bar);
        }
    }

    /// What this item was opened about, as a property.
    ///
    /// The pointer is swept pixel by pixel across the bar, for every row
    /// length and every magnification the config will hand over, and no icon
    /// is ever drawn outside the background the bar paints. With the
    /// background painted around the resting run instead, this failed by about
    /// 19px a side at 48px icons and 1.6 — and by 108px at 2.5.
    #[test]
    fn no_icon_is_ever_drawn_outside_the_background_the_bar_paints() {
        for count in 1..=30usize {
            for tenths in 10..=25u32 {
                let rest = real(tenths as f64 / 10.0);
                let bar = resting_bar(&rest, count);
                let centres = centres_in(&rest, count, bar);

                for pointer in bar.x()..=(bar.x() + bar.width()) {
                    let drawn = row_of(&centres, Some(pointer as f64), &rest);
                    let painted = crate::ground::around(bar, reach_of(&drawn, &rest));
                    let (start, end) = (
                        painted.x() as f64,
                        (painted.x() + painted.width()) as f64,
                    );

                    for (index, icon) in drawn.iter().enumerate() {
                        let (left, right) = icon.edges(rest.icon);
                        assert!(
                            left >= start && right <= end,
                            "icon {index} of {count} at x{:.1} reaches {left:.1}..{right:.1}, \
                             outside a background of {start}..{end} at magnification {}",
                            pointer,
                            rest.scale
                        );
                    }
                }
            }
        }
    }

    /// Where the fix went, stated as what it did *not* touch.
    ///
    /// The lens is Plank's formula and nothing limits it: no clamp at the
    /// ends, no fold, no end zone. An icon is where `place` puts it at both
    /// ends of the row exactly as much as in the middle, so a row that was
    /// right before this item is identical to it, value for value. What
    /// changed is the background that gets painted around the answer.
    #[test]
    fn the_lens_is_not_clamped_or_folded_anywhere_along_the_row() {
        for count in [1usize, 2, 7, 28] {
            let rest = real(magnify::DEFAULT_SCALE);
            let bar = resting_bar(&rest, count);
            let centres = centres_in(&rest, count, bar);

            for pointer in bar.x()..=(bar.x() + bar.width()) {
                for (index, icon) in row_of(&centres, Some(pointer as f64), &rest)
                    .iter()
                    .enumerate()
                {
                    let plank = magnify::place(
                        centres[index],
                        Some(pointer as f64),
                        rest.icon,
                        rest.scale,
                    );

                    assert_eq!(
                        icon.placed, plank,
                        "icon {index} of {count} was moved off the lens's own answer"
                    );
                }
            }
        }
    }

    /// A background that follows the drawing must not move further than the
    /// drawing does, or the bar steps while the icons glide.
    ///
    /// Held to the drawing rather than to a figure: how far an icon travels
    /// for a pixel of pointer is the lens's own business, and at 2.5 a pixel
    /// of pointer is already worth more than a pixel of icon. The pixel of
    /// slack is the rounding — the background is painted on whole pixels.
    #[test]
    fn the_background_never_steps_further_than_the_icons_it_follows() {
        for count in [1usize, 2, 7, 28] {
            for tenths in 11..=25u32 {
                let rest = real(tenths as f64 / 10.0);
                let bar = resting_bar(&rest, count);
                let centres = centres_in(&rest, count, bar);
                let painted = |pointer: f64| {
                    let drawn = row_of(&centres, Some(pointer), &rest);
                    let reach = reach_of(&drawn, &rest).expect("icons");
                    (crate::ground::around(bar, Some(reach)), reach)
                };

                let mut before = painted(bar.x() as f64);
                for pointer in (bar.x() + 1)..=(bar.x() + bar.width()) {
                    let after = painted(pointer as f64);
                    let icons = (after.1 .0 - before.1 .0)
                        .abs()
                        .max((after.1 .1 - before.1 .1).abs());
                    let start = (after.0.x() - before.0.x()).abs() as f64;
                    let end = ((after.0.x() + after.0.width())
                        - (before.0.x() + before.0.width()))
                    .abs() as f64;

                    assert!(
                        start.max(end) <= icons + 1.0,
                        "the background jumped {:.1}px at x{pointer} while the icons \
                         moved {icons:.1}px, on a row of {count} at magnification {}",
                        start.max(end),
                        rest.scale
                    );
                    before = after;
                }
            }
        }
    }

    fn filled(count: usize) -> Row {
        let row = Row::new();
        let items: Vec<DockItem> = (0..count).map(|i| item(&format!("app{i}"))).collect();
        row.fill(&items, Rest::new(ICON, 4, 4, SCALE));
        row
    }

    /// Where every icon is drawn right now, left edge and width.
    fn drawn(row: &Row, count: usize) -> Vec<(i32, i32)> {
        (0..count).map(|index| row.rect_of(index)).collect()
    }

    fn edge(placed: magnify::Placement) -> f64 {
        placed.centre - placed.size / 2.0
    }

    /// The worst a frame of ordinary pointer movement costs.
    ///
    /// Not a constant: how far an icon moves for a given step of the pointer
    /// is a property of the lens's own shape, so it is measured off `place`
    /// rather than guessed at. This is the yardstick a jump is held to — a
    /// pointer that appeared somewhere must not move the row further in one
    /// frame than a pointer that swept there as fast as a lens follows.
    fn worst_frame_of_a_sweep(row: &Row, count: usize) -> f64 {
        let rest = row.rest.get();
        let step = crate::motion::AIM_SPEED * crate::motion::FRAME.as_secs_f64();
        let across = rest.centre(count - 1) + rest.slot;
        (0..across as i32)
            .step_by(2)
            .map(|at| {
                let (from, to) = (at as f64, at as f64 + step);
                (0..count)
                    .map(|index| {
                        let centre = rest.centre(index);
                        let before = magnify::place(centre, Some(from), rest.icon, rest.scale);
                        let after = magnify::place(centre, Some(to), rest.icon, rest.scale);
                        // The left edge, which is what is drawn and what the
                        // check below compares: an icon's edge moves with its
                        // centre *and* with half its growth.
                        (edge(after) - edge(before)).abs()
                    })
                    .fold(0.0f64, f64::max)
            })
            .fold(0.0f64, f64::max)
    }

    /// Measured before it was fixed, and the reason the fix is a ceiling and
    /// not an ease.
    ///
    /// The lens is open over the middle of the row, the pointer leaves and
    /// comes straight back at the far edge. The magnification reverses
    /// correctly — it never was the part that jumped — but the *centre* was
    /// the raw pointer, so the whole lens arrived somewhere else between two
    /// frames: 61px of icon movement at once, against the 27px a brisk sweep
    /// costs. Now it travels, and every frame of the travel is within what
    /// moving the pointer there would have cost.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_pointer_that_comes_back_elsewhere_travels_rather_than_teleports() {
        let count = 20;
        let row = filled(count);
        let rest = row.rest.get();
        let allowed = worst_frame_of_a_sweep(&row, count).ceil();

        row.aim(Some(rest.centre(10)));
        // The lens, fully open over the middle.
        row.hover_since.set(Instant::now() - crate::motion::ZOOM);
        let mut before = drawn(&row, count);

        // Away and back at the far edge, with no travel in between.
        row.aim(None);
        row.aim(Some(rest.centre(0)));
        assert!(row.scale_now() > 1.0, "the lens shut, and nothing could jump");

        let mut clock = Instant::now();
        let mut arrived = None;
        for frame in 1..=60 {
            clock += crate::motion::FRAME;
            row.step(clock);
            let now = drawn(&row, count);
            for (index, (was, is)) in before.iter().zip(&now).enumerate() {
                let moved = (is.0 - was.0).abs() as f64;
                assert!(
                    moved <= allowed,
                    "icon {index} moved {moved}px in frame {frame}, past the \
                     {allowed}px a sweep at the lens's own speed would cost"
                );
            }
            before = now;
            if row.lens() == Some(rest.centre(0)) {
                arrived = Some(frame);
                break;
            }
        }

        // The worst jump there is — one end of the row to the other — and it
        // costs about what opening the lens costs. Held to `ZOOM` rather than
        // to a number of frames, so the two cannot drift apart: this is the
        // figure the rest of the lens already moves on.
        let arrived = arrived.expect("the lens never reached the pointer");
        let took = crate::motion::FRAME * arrived;
        assert!(
            took <= crate::motion::ZOOM + crate::motion::FRAME,
            "catching up took {took:?}, longer than the lens takes to open"
        );
    }

    /// The other half of it, and the regression the ceiling could have been.
    ///
    /// Travelling only makes sense while there is a lens on screen to travel.
    /// A pointer arriving at a dock nobody has touched aims where it is, at
    /// once: making *that* glide would turn every entry into a quarter-second
    /// of the lens sliding in from wherever it was last.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_lens_that_is_shut_aims_at_once_rather_than_travelling() {
        let row = filled(20);
        let rest = row.rest.get();
        row.aim(Some(rest.centre(19)));
        row.aim(None);
        // Long enough ago that the lens is shut, not merely closing.
        row.hover_since.set(Instant::now() - crate::motion::ZOOM * 2);
        assert_eq!(row.scale_now(), 1.0, "the lens has not shut");

        row.aim(Some(rest.centre(0)));

        assert_eq!(
            row.lens(),
            Some(rest.centre(0)),
            "a shut lens took the scenic route to where the pointer already is"
        );
    }

    fn apps(count: usize) -> Vec<DockItem> {
        (0..count).map(|i| item(&format!("app{i}"))).collect()
    }

    /// Where the icons are, measured from the middle of the row.
    ///
    /// The bar is centred on the screen and is given the room the row asked
    /// for this instant, so this is the frame of reference in which the row
    /// is judged: a position that holds here holds on screen, whatever the
    /// bar's own width just did.
    fn about_the_middle(row: &Row) -> Vec<f64> {
        let half = row.area.size_request().0 as f64 / 2.0;
        row.centres().into_iter().map(|centre| centre - half).collect()
    }

    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn an_app_that_closed_keeps_its_place_until_it_has_finished_going() {
        let rest = Rest::new(ICON, 4, 4, SCALE);
        let row = Row::new();
        let five = apps(5);
        row.fill(&five, rest);

        let mut four = five.clone();
        four.remove(2);
        row.fill(&four, rest);

        assert_eq!(
            row.entries.borrow().len(),
            5,
            "the icon that left was dropped on the spot, with the neighbours \
             closing over it in one jump"
        );
        assert_eq!(
            row.entries.borrow()[2].item.id,
            "app2",
            "the departing icon lost its place in the row and is shrinking \
             somewhere it never stood"
        );
        assert!(row.entries.borrow()[2].life.get().leaving());
        // And so does its room, at first: the bar lets go of it as the icon
        // shrinks, not in the frame the app closed, so that whatever stands
        // beside the row on the bar slides with it instead of jumping.
        assert_eq!(row.perch.size_request().0, rest.run_width(5));

        // On its way out it is a picture and not a target.
        let (left, width) = row.rect_of(2);
        let over_it = left as f64 + width as f64 / 2.0;
        assert_ne!(
            row.at(over_it),
            Some(2),
            "an app that is leaving can still be clicked, and would be launched again"
        );

        std::thread::sleep(crate::motion::ENTRY);
        row.settle();

        assert_eq!(row.entries.borrow().len(), 4, "the space was never let go of");
        assert_eq!(
            row.perch.size_request().0,
            rest.run_width(4),
            "the icon went and the bar kept the room it had held"
        );
        assert!(
            row.entries.borrow().iter().all(|entry| entry.item.id != "app2"),
            "the icon that left is still on the row"
        );
    }

    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn an_app_that_opened_grows_into_the_room_rather_than_appearing_in_it() {
        let rest = Rest::new(ICON, 4, 4, SCALE);
        let row = Row::new();
        row.fill(&apps(5), rest);
        let before = about_the_middle(&row);

        // A sixth app, third from the left.
        let mut six = apps(5);
        six.insert(2, item("newcomer"));
        row.fill(&six, rest);

        assert_eq!(row.entries.borrow()[2].item.id, "newcomer");
        let (_, width) = row.rect_of(2);
        assert!(
            width < ICON,
            "the new icon was drawn at its full {ICON}px the instant it appeared"
        );

        // And the five that were already there have not moved on screen.
        let after = about_the_middle(&row);
        let others: Vec<f64> = after
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != 2)
            .map(|(_, centre)| *centre)
            .collect();
        for (index, (was, is)) in before.iter().zip(&others).enumerate() {
            assert!(
                (was - is).abs() <= 1.0,
                "icon {index} jumped {}px when a neighbour opened",
                (was - is).abs()
            );
        }

        std::thread::sleep(crate::motion::ENTRY);
        assert_eq!(
            row.rect_of(2).1,
            ICON,
            "the new icon never finished arriving"
        );
    }

    /// The regression this could so easily have been.
    ///
    /// The icon size is a slider in the preferences window, and the icon
    /// theme is a setting the desktop can change: both refill the row with
    /// every icon new to it. Neither is an app opening, and animating them
    /// would mean the whole row fading in and out under the hand of someone
    /// dragging a slider.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_row_that_was_resized_or_reloaded_is_not_a_row_six_apps_just_opened_on() {
        let row = Row::new();
        let five = apps(5);

        row.fill(&five, Rest::new(ICON, 4, 4, SCALE));
        assert!(
            row.entries.borrow().iter().all(|entry| !entry.life.get().moving()),
            "the dock announced every icon it had the moment it started"
        );

        row.fill(&five, Rest::new(ICON + 8, 4, 4, SCALE));
        assert!(
            row.entries.borrow().iter().all(|entry| !entry.life.get().moving()),
            "moving the icon-size slider faded the whole row in again"
        );

        row.reload_icons();
        row.fill(&five, Rest::new(ICON + 8, 4, 4, SCALE));
        assert!(
            row.entries.borrow().iter().all(|entry| !entry.life.get().moving()),
            "a new icon theme faded the whole row in again"
        );
    }

    /// A dock switched for one with more apps on it can take the icons down a
    /// size to fit. That is still a switch, and not a row starting over: the
    /// apps both docks share stay on the row, and only what is new arrives.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_switch_that_took_the_icons_down_a_size_still_carries_the_shared_apps_over() {
        let row = Row::new();
        row.fill(&apps(5), Rest::new(ICON, 4, 4, SCALE));

        let mut other = apps(3);
        other.push(item("newcomer"));
        row.fill(&other, Rest::new(ICON - 8, 4, 4, SCALE));

        let entries = row.entries.borrow();
        let life = |id: &str| {
            entries
                .iter()
                .find(|entry| entry.item.id == id)
                .map(|entry| entry.life.get())
                .unwrap_or_else(|| panic!("{id} is not on the row at all"))
        };
        for shared in ["app0", "app1", "app2"] {
            assert!(
                !life(shared).moving(),
                "{shared} is on both docks and was faded in again"
            );
        }
        assert!(life("newcomer").moving() && !life("newcomer").leaving());
        for gone in ["app3", "app4"] {
            assert!(life(gone).leaving(), "{gone} was dropped instead of leaving");
        }
    }

    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn an_app_that_closed_and_opened_again_comes_back_from_where_it_had_got_to() {
        let rest = Rest::new(ICON, 4, 4, SCALE);
        let row = Row::new();
        let five = apps(5);
        row.fill(&five, rest);

        let mut four = five.clone();
        four.remove(2);
        row.fill(&four, rest);
        let going = row.entries.borrow()[2].life.get().presence();
        row.fill(&five, rest);

        let entries = row.entries.borrow();
        assert_eq!(entries.len(), 5, "the icon came back as well as never leaving");
        assert_eq!(entries[2].item.id, "app2");
        assert!(!entries[2].life.get().leaving(), "the icon that came back is still going");
        assert!(
            (entries[2].life.get().presence() - going).abs() < 0.2,
            "the icon jumped from {going:.2} to {:.2} on coming back",
            entries[2].life.get().presence()
        );
    }

    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_pointer_that_left_is_not_pointing_at_anything() {
        let row = Row::new();
        row.fill(&[item("a"), item("b")], Rest::new(48, 4, 4, 1.6));
        let over_the_first = row.rest.get().centre(0);

        row.aim(Some(over_the_first));
        assert!(row.hovered().is_some(), "the pointer is on the first icon");

        row.aim(None);
        assert!(
            row.hovered().is_none(),
            "the pointer has left, and nothing is under it"
        );
        assert!(
            row.pointer.get().is_some(),
            "the lens still needs somewhere to close over"
        );
    }
}
