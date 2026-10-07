use std::cell::Cell;

use gtk::prelude::*;

use crate::dock::{BAR_PADDING, ITEM_PADDING};

/// How far past an icon the painted bar reaches: the bar's own padding, and
/// the frame every icon carries inside its slot.
pub const REACH: i32 = BAR_PADDING + ITEM_PADDING;

/// Where the bar's background goes, given the place the bar holds in the
/// window and the edges the icons are drawn at.
///
/// Plank's answer to the same question, and for the same reason.
/// `PositionManager.update_background_region` measures the background from
/// `draw_values[items.first ()]` and `draw_values[items.last ()]` — the
/// first and last items as they are *drawn*, not as they rest. The lens
/// pushes the icons at the ends of the row outwards and its push does not
/// decay, so a background painted around where the icons rest is a
/// background the end icons are drawn outside of. Painted around where they
/// are drawn, there is nothing left to leak.
///
/// The place the bar holds is the floor, never the answer on its own. The
/// row is only part of the bar — the widget tiles are laid out beside it and
/// the bar has to go on covering them — and at rest the icons ask for
/// exactly the bar, so the two agree and nothing moves. Everything past that
/// is the lens, and it only ever asks for more.
pub fn around(bar: gdk::Rectangle, icons: Option<(f64, f64)>) -> gdk::Rectangle {
    let Some((left, right)) = icons else {
        return bar;
    };
    let start = bar.x().min(left.floor() as i32 - REACH);
    let end = (bar.x() + bar.width()).max(right.ceil() as i32 + REACH);
    gdk::Rectangle::new(start, bar.y(), end - start, bar.height())
}

/// The hair of slack the repainted strip is widened by.
///
/// The strip is worked out on the frame clock and the painting happens a
/// moment later, and in that moment the lens may still be opening or closing
/// — so what is painted comes out a fraction wider or narrower than what was
/// asked for. That fraction has to fall inside the strip, or it is left on
/// screen until something else happens to invalidate it.
const SLIP: i32 = 2;

/// The strip that has to be repainted to get from one background to the next.
///
/// The bar grows and shrinks at its ends and never in the middle, so this is
/// the two ends and not the whole of it — which matters, because the middle
/// is where the widget tiles are, and a tile repainted sixty times a second
/// is a Pango layout sixty times a second for a picture that did not change.
pub fn changed(was: gdk::Rectangle, is: gdk::Rectangle) -> Option<gdk::Rectangle> {
    if was == is {
        return None;
    }
    let start = was.x().min(is.x()) - SLIP;
    let end = (was.x() + was.width()).max(is.x() + is.width()) + SLIP;
    let top = was.y().min(is.y()) - SLIP;
    let bottom = (was.y() + was.height()).max(is.y() + is.height()) + SLIP;
    Some(gdk::Rectangle::new(start, top, end - start, bottom - top))
}

/// The bar's background, painted on the window under everything else.
///
/// Not a widget of its own, and not the bar either. A widget's background is
/// painted at its allocation, and the whole point here is to paint somewhere
/// else: around a row of icons that moves sixty times a second while the
/// layout under it does not move at all. Growing the bar instead would be a
/// layout renegotiation a frame, and worse than slow — the row is positioned
/// off the bar's own allocation, so a bar that grew to cover the icons would
/// carry the icons it grew to cover along with it, and chase itself.
pub struct Ground {
    /// Carries the `#ground` rules, and nothing else. Never shown, never
    /// allocated, never drawn as itself: it is here for its style, which is
    /// the one thing a stylesheet can only hand to a widget.
    style: gtk::DrawingArea,
    /// Where it was painted last, so the next frame knows what to repaint.
    painted: Cell<(i32, i32, i32, i32)>,
}

impl Ground {
    pub fn new() -> Self {
        let style = gtk::DrawingArea::new();
        style.set_widget_name("ground");
        Self {
            style,
            painted: Cell::new((0, 0, 0, 0)),
        }
    }

    /// Paint it under the stage, and keep it following the row.
    ///
    /// `draw` on a container is `RUN_LAST`, so a handler connected here runs
    /// before the one that draws the children: painting and then letting the
    /// signal through puts the background under the bar, the tiles and the
    /// icons without any of them having to know it is there.
    pub fn under(self: &std::rc::Rc<Self>, stage: &gtk::Overlay, bar: &gtk::Box, row: &crate::row::Row) {
        let painting = self.clone();
        let painted_bar = bar.clone();
        let painted_row = row.clone();
        stage.connect_draw(move |stage, cr| {
            painting.paint(cr, painting.rect(stage, &painted_bar, &painted_row));
            glib::Propagation::Proceed
        });

        let following = self.clone();
        let followed_bar = bar.clone();
        let followed_row = row.clone();
        let followed_stage = stage.clone();
        row.followed_by(move || {
            let was = following.painted.get();
            let is = following.rect(&followed_stage, &followed_bar, &followed_row);
            if let Some(strip) = changed(rectangle(was), is) {
                followed_stage.queue_draw_area(
                    strip.x(),
                    strip.y(),
                    strip.width(),
                    strip.height(),
                );
            }
        });
    }

    /// Where the background belongs this instant, in the stage's coordinates.
    fn rect(
        &self,
        stage: &gtk::Overlay,
        bar: &gtk::Box,
        row: &crate::row::Row,
    ) -> gdk::Rectangle {
        let origin = stage.allocation();
        let bar_at = bar.allocation();
        let bar_at = gdk::Rectangle::new(
            bar_at.x() - origin.x(),
            bar_at.y() - origin.y(),
            bar_at.width(),
            bar_at.height(),
        );
        // The row draws in its own surface, which sits in the stage at its
        // own allocation; the edges it reports are in that surface.
        let at = row.area.allocation().x() - origin.x();
        let icons = row
            .drawn_edges()
            .map(|(left, right)| (left + at as f64, right + at as f64));
        around(bar_at, icons)
    }

    fn paint(&self, cr: &gtk::cairo::Context, rect: gdk::Rectangle) {
        if rect.width() <= 0 || rect.height() <= 0 {
            return;
        }
        let context = self.style.style_context();
        gtk::render_background(
            &context,
            cr,
            rect.x() as f64,
            rect.y() as f64,
            rect.width() as f64,
            rect.height() as f64,
        );
        gtk::render_frame(
            &context,
            cr,
            rect.x() as f64,
            rect.y() as f64,
            rect.width() as f64,
            rect.height() as f64,
        );
        self.painted
            .set((rect.x(), rect.y(), rect.width(), rect.height()));
    }
}

fn rectangle(kept: (i32, i32, i32, i32)) -> gdk::Rectangle {
    gdk::Rectangle::new(kept.0, kept.1, kept.2, kept.3)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// The one question only a running GTK can answer: whether an unparented
    /// widget named `ground` is handed the stylesheet's rules at all.
    ///
    /// The background is painted from that widget's style and from nothing
    /// else, so a name that does not resolve is a dock with no background —
    /// and on a translucent window that is an invisible dock, which is the
    /// worst way for this to fail because there is nothing on screen to say
    /// what went wrong. So it is painted onto a surface here and the pixel is
    /// read back.
    pub fn the_ground_is_painted_in_the_colours_the_stylesheet_names() {
        let css = gtk::CssProvider::new();
        css.load_from_data(crate::theme::css(crate::theme::DEFAULT).as_bytes())
            .expect("the default sheet does not parse");
        gtk::StyleContext::add_provider_for_screen(
            &gdk::Screen::default().expect("no screen"),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let (width, height) = (60, 40);
        let mut surface =
            gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, width, height)
                .expect("no surface");
        {
            let cr = gtk::cairo::Context::new(&surface).expect("no context");
            Ground::new().paint(&cr, gdk::Rectangle::new(0, 0, width, height));
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let data = surface.data().expect("the surface is still borrowed");
        // ARGB32 is premultiplied and little-endian: the alpha is the last of
        // the four bytes. The middle of the rectangle, well inside the
        // rounding at its corners.
        let middle = (height as usize / 2) * stride + (width as usize / 2) * 4;

        assert!(
            data[middle + 3] > 0,
            "#ground painted nothing: the bar would be invisible"
        );
    }

    fn bar() -> gdk::Rectangle {
        gdk::Rectangle::new(100, 20, 400, 72)
    }

    /// The case the dock is in whenever the pointer is elsewhere, which is
    /// most of the time: the icons ask for the bar, so the bar is the answer
    /// and the background is exactly where the layout put it.
    #[test]
    fn a_row_at_rest_is_painted_around_exactly_the_bar_it_sits_in() {
        let icons = (
            (bar().x() + REACH) as f64,
            (bar().x() + bar().width() - REACH) as f64,
        );

        let painted = around(bar(), Some(icons));

        assert_eq!(painted, bar());
    }

    #[test]
    fn an_empty_row_leaves_the_bar_where_the_layout_put_it() {
        assert_eq!(around(bar(), None), bar());
    }

    #[test]
    fn an_icon_pushed_past_the_end_takes_the_background_with_it() {
        let left = (bar().x() + REACH) as f64 - 19.0;
        let right = (bar().x() + bar().width() - REACH) as f64;

        let painted = around(bar(), Some((left, right)));

        assert_eq!(painted.x(), bar().x() - 19);
        assert_eq!(
            painted.x() + painted.width(),
            bar().x() + bar().width(),
            "the far end moved for something that happened at the near one"
        );
    }

    /// Where the row ends is not where the bar ends: with widgets on it the
    /// run of icons stops at the divider, and the bar has to go on covering
    /// the tiles past it.
    #[test]
    fn the_bar_keeps_covering_the_tiles_beside_a_row_that_is_shorter_than_it() {
        let icons = (
            (bar().x() + REACH) as f64,
            (bar().x() + 150 - REACH) as f64,
        );

        let painted = around(bar(), Some(icons));

        assert_eq!(painted, bar(), "the bar shrank away from its own tiles");
    }

    #[test]
    fn the_background_is_never_painted_short_of_an_icon_at_either_end() {
        for left in -40..40 {
            for right in -40..40 {
                let edges = (
                    (bar().x() + REACH + left) as f64,
                    (bar().x() + bar().width() - REACH + right) as f64,
                );

                let painted = around(bar(), Some(edges));

                assert!(
                    (painted.x() as f64) <= edges.0 && edges.1 <= (painted.x() + painted.width()) as f64,
                    "icons reaching {edges:?} are drawn outside a background of \
                     {}..{}",
                    painted.x(),
                    painted.x() + painted.width()
                );
            }
        }
    }

    /// The whole reason the strip is worked out rather than the stage simply
    /// redrawn: nothing in the middle of the bar is touched, so the tiles are
    /// left alone while the lens is open.
    #[test]
    fn only_the_ends_of_the_bar_are_repainted_when_the_background_grows() {
        let was = bar();
        let is = around(bar(), Some(((bar().x() + REACH - 19) as f64, 0.0)));

        let strip = changed(was, is).expect("the background moved");

        assert_eq!(strip.x(), is.x() - SLIP);
        assert_eq!(strip.x() + strip.width(), was.x() + was.width() + SLIP);
    }

    #[test]
    fn a_background_that_did_not_move_is_not_repainted_at_all() {
        assert!(changed(bar(), bar()).is_none());
    }

    /// The background shrinking back has to repaint the strip it is giving
    /// up, or the bar keeps a stub of itself out past the icons.
    #[test]
    fn the_strip_given_up_by_a_shrinking_background_is_repainted_too() {
        let wide = around(bar(), Some(((bar().x() + REACH - 19) as f64, 0.0)));

        let strip = changed(wide, bar()).expect("the background moved");

        assert_eq!(strip.x(), wide.x() - SLIP);
        assert!(strip.width() >= wide.width());
    }
}
