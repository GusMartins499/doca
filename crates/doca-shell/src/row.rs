use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use gtk::cairo;
use gtk::gdk_pixbuf;
use gtk::prelude::*;
use doca_ipc::{DockItem, DocaProxy};

use crate::magnify::{self, Placement};

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
}

impl Entry {
    fn from(item: &DockItem, largest: i32) -> Self {
        Self {
            item: item.clone(),
            source: crate::dock::load_pixbuf(&item.icon, largest),
            scaled: RefCell::new(HashMap::new()),
            launched: Cell::new(None),
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

    pub fn width(&self, count: usize) -> i32 {
        if count == 0 {
            return 0;
        }
        (self.margin * 2.0 + count as f64 * (self.slot + self.spacing) - self.spacing).ceil()
            as i32
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
    pointer: Rc<Cell<Option<f64>>>,
    largest: Rc<Cell<i32>>,
    /// When the pointer arrived or left, and which of the two it was.
    hovering: Rc<Cell<bool>>,
    hover_since: Rc<Cell<Instant>>,
    ticking: Rc<Cell<bool>>,
    dots: Rc<RefCell<(gdk::RGBA, gdk::RGBA)>>,
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
            Some(gdk::Rectangle::new(
                perch.x(),
                perch.y() - rest.overhead(),
                perch.width(),
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

        let perch = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        perch.set_widget_name("perch");

        let row = Self {
            area,
            perch,
            entries: Rc::new(RefCell::new(Vec::new())),
            rest: Rc::new(Cell::new(Rest::default())),
            pointer: Rc::new(Cell::new(None)),
            largest: Rc::new(Cell::new(0)),
            hovering: Rc::new(Cell::new(false)),
            hover_since: Rc::new(Cell::new(Instant::now() - crate::motion::ZOOM)),
            ticking: Rc::new(Cell::new(false)),
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

        let mut known: HashMap<String, Entry> = if resized {
            HashMap::new()
        } else {
            self.entries
                .borrow_mut()
                .drain(..)
                .map(|entry| (entry.item.id.clone(), entry))
                .collect()
        };

        *self.entries.borrow_mut() = items
            .iter()
            .map(|item| match known.remove(&item.id) {
                Some(entry) if entry.item.icon == item.icon => entry.again(item),
                _ => Entry::from(item, largest),
            })
            .collect();

        self.rest.set(rest);
        let width = rest.width(items.len());
        self.perch.set_size_request(width, rest.resting_height());
        self.area.set_size_request(width, rest.height());
        self.area.queue_draw();
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

    /// Note where the pointer is. Redrawing waits for the next frame.
    ///
    /// Leaving keeps the last position: the lens closes over the icon it was
    /// open on, rather than snapping shut the moment the pointer is gone.
    pub fn aim(&self, pointer: Option<f64>) {
        match pointer {
            Some(x) => {
                self.pointer.set(Some(x));
                self.hover(true);
            }
            None => self.hover(false),
        }
        self.animate();
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
            area.queue_draw();
            if row.resting() {
                row.ticking.set(false);
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    fn resting(&self) -> bool {
        let launching = self
            .entries
            .borrow()
            .iter()
            .any(|entry| entry.launched.get().is_some_and(crate::motion::is_launching_since));
        !launching && !self.hovering.get() && self.scale_now() <= 1.0
    }

    /// Which icon is under `x`, by what is drawn rather than by what rests there.
    pub fn at(&self, x: f64) -> Option<usize> {
        let rest = self.rest.get();
        let scale = self.scale_now();
        let pointer = Some(x);
        (0..self.entries.borrow().len()).find(|index| {
            let placed = magnify::place(rest.centre(*index), pointer, rest.icon, scale);
            (x - placed.centre).abs() <= (placed.size / 2.0).max(rest.slot / 2.0)
        })
    }

    /// Where an icon is drawn, for something else to point at.
    pub fn rect_of(&self, index: usize) -> (i32, i32) {
        let rest = self.rest.get();
        let placed =
            magnify::place(rest.centre(index), self.pointer.get(), rest.icon, self.scale_now());
        ((placed.centre - placed.size / 2.0) as i32, placed.size as i32)
    }

    /// The room a magnified icon rises into, above the bar.
    pub fn overhead(&self) -> i32 {
        self.rest.get().overhead()
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

    fn draw(&self, area: &gtk::DrawingArea, cr: &cairo::Context) {
        let rest = self.rest.get();
        if rest.icon <= 0.0 {
            return;
        }
        let bottom = area.allocated_height() as f64 - INDICATOR as f64;
        let pointer = self.pointer.get();
        let scale = self.scale_now();
        let (running_dot, active_dot) = *self.dots.borrow();

        let entries = self.entries.borrow();
        let placed: Vec<(Placement, bool, bool)> = {
            entries
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    let mut placed = magnify::place(rest.centre(index), pointer, rest.icon, scale);
                    if let Some(started) = entry.launched.get() {
                        let elapsed = started.elapsed();
                        if crate::motion::is_launching(elapsed) {
                            placed.size *= crate::motion::launch_scale(elapsed);
                        } else {
                            entry.launched.set(None);
                        }
                    }
                    (placed, !entry.item.windows.is_empty(), entry.item.active)
                })
                .collect()
        };

        for (index, (placed, running, active)) in placed.iter().enumerate() {
            let size = magnify::settled(placed.size.round() as i32, rest.icon as i32);
            let Some(pixbuf) = entries.get(index).and_then(|entry| entry.at(size)) else {
                continue;
            };
            let x = placed.centre - size as f64 / 2.0;
            let y = bottom - size as f64;
            cr.set_source_pixbuf(&pixbuf, x.round(), y.round());
            let _ = cr.paint();

            if *running {
                let dot = if *active { active_dot } else { running_dot };
                cr.set_source_rgba(
                    dot.red(),
                    dot.green(),
                    dot.blue(),
                    dot.alpha(),
                );
                cr.rectangle(
                    (placed.centre - DOT_WIDTH / 2.0).round(),
                    bottom + 3.0,
                    DOT_WIDTH,
                    DOT_HEIGHT,
                );
                let _ = cr.fill();
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
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
