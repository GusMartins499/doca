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

use doca_ipc::{Body, Simple, Tile, Water, WidgetState, NO_PROGRESS};
use gtk::cairo;
use gtk::prelude::*;

use crate::theme::Palette;

/// How tall a tile is: the icons' own height, so a tile sits in exactly the
/// slot an icon would and the bar's height is unchanged by having one.
pub fn height_of(icon_size: i32) -> i32 {
    icon_size
}

/// The room one tile takes on the bar: what the widget declared, plus the
/// frame the stylesheet draws round it and the gap to the next thing.
///
/// The same arithmetic the row does per icon, which is what makes the bar's
/// width the sum of its slots and nothing else.
pub fn slot_of(tile: Tile, icon_size: i32) -> i32 {
    tile.width(icon_size) + crate::dock::ITEM_PADDING * 2 + crate::dock::ITEM_SPACING
}

/// The room all of these tiles take together.
///
/// Worked out from the icon size the *config* asked for rather than the one
/// the row ends up with. The two differ only on a dock crowded enough to
/// shrink its icons, and sizing the tiles from the fitted size would be
/// circular: the fitted size is worked out from the room the tiles leave.
/// This way a tile is the size the user asked for, the icons give up the
/// pixels, and neither depends on the other twice.
pub fn room_for(widgets: &[WidgetState], icon_size: i32) -> i32 {
    widgets
        .iter()
        .map(|state| slot_of(state.tile().unwrap_or(Tile::Wide), icon_size))
        .sum()
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
}

/// What a click on a tile does: ask for this widget's panel, over this tile.
///
/// A closure rather than anything that knows what a panel is, for the reason
/// the folder grid takes one: a tile that cannot be built without a daemon on
/// the other end cannot be checked without one either.
pub type Expand = Rc<dyn Fn(&str, &gtk::Widget)>;

pub struct WidgetTile {
    pub root: gtk::Widget,
    area: gtk::DrawingArea,
    /// What the drawing path reads, and the only thing it reads.
    shown: Rc<RefCell<WidgetState>>,
    look: Rc<Cell<Look>>,
}

impl WidgetTile {
    pub fn new(state: &WidgetState, look: Look, expand: Expand) -> Self {
        let shown = Rc::new(RefCell::new(state.clone()));
        let held = Rc::new(Cell::new(look));

        let area = gtk::DrawingArea::new();
        area.set_widget_name("widget-face");
        let painting = shown.clone();
        let wearing = held.clone();
        area.connect_draw(move |area, cr| {
            draw(&painting.borrow(), wearing.get(), area, cr);
            glib::Propagation::Proceed
        });

        let tile = gtk::EventBox::new();
        tile.set_widget_name("widget");
        tile.add(&area);

        let id = state.id.clone();
        let root: gtk::Widget = tile.clone().upcast();
        let opening = root.clone();
        tile.connect_button_press_event(move |_, _| {
            // Either button, and nothing else: the controls live in the panel
            // now, so there is one way in rather than a left click that does
            // one thing and a right click that undoes it.
            expand(&id, &opening);
            glib::Propagation::Stop
        });

        let tile = Self {
            root,
            area,
            shown,
            look: held,
        };
        tile.resize();
        tile.mark();
        tile
    }

    /// What this tile is showing now.
    pub fn update(&self, state: &WidgetState) {
        let resize = self.shown.borrow().tile() != state.tile();
        *self.shown.borrow_mut() = state.clone();
        if resize {
            self.resize();
        }
        self.mark();
        self.area.queue_draw();
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
        let tile = self.shown.borrow().tile().unwrap_or(Tile::Wide);
        self.area
            .set_size_request(tile.width(look.icon_size), height_of(look.icon_size));
    }

    /// The one thing still left to the stylesheet: the wash of accent behind
    /// a tile that is marking itself.
    fn mark(&self) {
        let active = matches!(
            self.shown.borrow().body(),
            Ok(Body::Simple(Simple { active: true, .. }))
        ) || matches!(
            self.shown.borrow().body(),
            Ok(Body::Water(Water { glasses, goal })) if glasses >= goal
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
    if width <= 0.0 || height <= 0.0 {
        return;
    }

    match state.body() {
        Ok(Body::Simple(simple)) => draw_simple(&simple, look, area, cr, width, height),
        Ok(Body::Water(water)) => draw_water(&water, look, area, cr, width, height),
        Err(_) => {
            let layout = line(area, "\u{2014}", text_size(look.icon_size, 0.30), true, width);
            let (_, logical) = layout.pixel_extents();
            paint(cr, &layout, 0.0, (height - logical.height() as f64) / 2.0, look.palette.detail);
        }
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

    let label_height = label.pixel_extents().1.height() as f64;
    let detail_height = if simple.detail.is_empty() {
        0.0
    } else {
        detail.pixel_extents().1.height() as f64
    };
    let bar = (simple.progress != NO_PROGRESS).then(|| bar_height(look.icon_size));
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

/// Water: the glasses counted, inside a ring of how much of the goal that is.
///
/// A square badge, which is the chassis' half of #28 — the wide card with the
/// week's bars and the next reminder in it is that issue's, and this becomes
/// it by changing which [`Tile`] the variant declares.
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
        (water.glasses as f64 / water.goal as f64).clamp(0.0, 1.0)
    };
    let thickness = (look.icon_size as f64 * 0.10).max(2.0);
    let radius = (width.min(height) - thickness) / 2.0 - 1.0;
    if radius <= 0.0 {
        return;
    }
    let (centre_x, centre_y) = (width / 2.0, height / 2.0);

    cr.set_line_width(thickness);
    let track = look.palette.track;
    cr.set_source_rgba(track.red(), track.green(), track.blue(), track.alpha());
    cr.arc(centre_x, centre_y, radius, 0.0, std::f64::consts::TAU);
    let _ = cr.stroke();

    if fraction > 0.0 {
        let fill = look.palette.fill;
        cr.set_source_rgba(fill.red(), fill.green(), fill.blue(), fill.alpha());
        // From the top, clockwise, the way a ring of anything counted is
        // read.
        let from = -std::f64::consts::FRAC_PI_2;
        cr.arc(centre_x, centre_y, radius, from, from + fraction * std::f64::consts::TAU);
        let _ = cr.stroke();
    }

    // The count alone. The goal is the ring, so saying "3/8" in a square this
    // size would be half the words at half the size for no more meaning.
    let layout = line(
        area,
        &water.glasses.to_string(),
        text_size(look.icon_size, 0.36),
        true,
        width,
    );
    let (_, logical) = layout.pixel_extents();
    paint(
        cr,
        &layout,
        0.0,
        centre_y - logical.height() as f64 / 2.0,
        look.palette.label,
    );
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
}

impl Shelf {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether this widget is one of ours, and must survive a rebuild.
    ///
    /// The rebuild sweeps the bar clear of everything it is about to make
    /// again. What the shelf owns is not made again, so it has to be asked.
    pub fn owns(&self, child: &gtk::Widget) -> bool {
        if self
            .divider
            .borrow()
            .as_ref()
            .is_some_and(|divider| divider.clone().upcast::<gtk::Widget>() == *child)
        {
            return true;
        }
        self.tiles
            .borrow()
            .iter()
            .any(|(_, tile)| tile.root == *child)
    }

    /// Put these widgets on the bar, keeping the tiles already there.
    ///
    /// A tile whose id comes back is the same tile, moved into place and told
    /// its new state rather than remade. One that does not come back is taken
    /// off the bar and dropped — which is when its widget is finally
    /// destroyed, because until then this held the only other reference to it.
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
        let mut known: HashMap<String, WidgetTile> =
            self.tiles.borrow_mut().drain(..).collect();
        let had = known.len();

        let shown: Vec<(String, WidgetTile)> = widgets
            .iter()
            .map(|state| {
                let tile = match known.remove(&state.id) {
                    Some(tile) => {
                        tile.relook(look);
                        tile.update(state);
                        tile
                    }
                    None => {
                        let tile = WidgetTile::new(state, look, expand.clone());
                        items.add(&tile.root);
                        tile
                    }
                };
                (state.id.clone(), tile)
            })
            .collect();

        // Whatever no dock asks for any more.
        let gone = known.len();
        for (_, departed) in known {
            items.remove(&departed.root);
        }
        let did = Reconciled {
            kept: had - gone,
            made: shown.len() - (had - gone),
            gone,
        };
        // Read back by scripts/check-live.sh, which has no other way to tell
        // a bar that kept its tiles from one that made them again.
        tracing::debug!(
            kept = did.kept,
            made = did.made,
            gone = did.gone,
            "the widget shelf"
        );

        if shown.is_empty() {
            // Nothing to divide from. A separator with no widgets behind it
            // is a stray line at the end of the bar.
            if let Some(divider) = self.divider.borrow().as_ref() {
                if divider.parent().is_some() {
                    items.remove(divider);
                }
            }
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
            for (_, tile) in &shown {
                items.reorder_child(&tile.root, -1);
            }
        }

        *self.tiles.borrow_mut() = shown;
        did
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
}

#[cfg(test)]
pub mod tests {
    use super::*;

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

    fn water(glasses: u32, goal: u32) -> WidgetState {
        WidgetState::new("water", Body::Water(Water { glasses, goal }))
    }

    const ICON: i32 = 48;

    fn look(icon_size: i32) -> Look {
        Look {
            icon_size,
            palette: crate::theme::palette(crate::theme::DEFAULT),
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
        assert!(
            !items.children().contains(&cpu),
            "a widget nobody asks for is still on the bar"
        );
        assert!(items.children().contains(&clock), "the survivor went too");
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
    pub fn a_tile_is_the_size_the_widget_declared() {
        let (items, _, shelf, expand) = bar();

        shelf.show(&items, &[simple("clock"), water(2, 8)], look(ICON), &expand);

        assert_eq!(
            widths(&shelf),
            vec![Tile::Wide.width(ICON), Tile::Square.width(ICON)],
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
        assert_eq!(small, vec![Tile::Wide.width(24), Tile::Square.width(24)]);
        assert_eq!(
            widths(&shelf),
            vec![Tile::Wide.width(96), Tile::Square.width(96)],
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

    #[test]
    fn a_tile_sits_in_the_same_slot_an_icon_would() {
        // The row pays `ITEM_SPACING + ITEM_PADDING * 2` per icon, and the
        // bar's width is the sum of its slots: a tile measured any other way
        // is a bar that is wider or narrower than it says it is.
        let frame = crate::dock::ITEM_PADDING * 2 + crate::dock::ITEM_SPACING;

        assert_eq!(slot_of(Tile::Square, 48), 48 + frame);
        assert_eq!(slot_of(Tile::Wide, 48), 120 + frame);
        assert_eq!(height_of(48), 48);
    }

    #[test]
    fn the_room_the_tiles_need_is_the_sum_of_what_each_declared() {
        let widgets = [simple("clock"), water(1, 8), simple("cpu")];

        assert_eq!(
            room_for(&widgets, 48),
            slot_of(Tile::Wide, 48) * 2 + slot_of(Tile::Square, 48)
        );
        assert_eq!(room_for(&[], 48), 0, "no widgets need no room");
    }

    #[test]
    fn a_bar_with_no_widgets_asks_for_no_room_at_any_icon_size() {
        for icon in [24, 48, 96] {
            assert_eq!(room_for(&[], icon), 0);
        }
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
