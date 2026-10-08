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
        // So that "the keyboard goes back to the bar when the panel closes"
        // is literally true rather than a hope: a widget that cannot take
        // focus cannot be given it back, and `grab_focus` on one is a call
        // that quietly does nothing.
        tile.set_can_focus(true);
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
            .set_size_request(look.level.width(tile, look.icon_size), height_of(look.icon_size));
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
        Ok(Body::Music(music)) => draw_music(&music, look, area, cr, width, height),
        Ok(Body::Note(note)) => draw_note(&note, look, area, cr, width, height),
        Err(_) => {
            let layout = line(area, "\u{2014}", text_size(look.icon_size, 0.30), true, width);
            let (_, logical) = layout.pixel_extents();
            paint(cr, &layout, 0.0, (height - logical.height() as f64) / 2.0, look.palette.detail);
        }
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
