//! What the bar gives up when the screen is not wide enough for it, and the
//! order it gives it up in.
//!
//! The bar used to answer this by not answering: `icon_size_for` shrank the
//! icons to a floor and `clamp_to_screen` cut whatever still stuck out. Cutting
//! is the one answer that cannot be right — the part that goes is the part
//! nobody chose, and on a crowded dock that is the last few apps, silently.
//!
//! So the giving-way is a ladder, taken a rung at a time and only as far as it
//! has to go:
//!
//! 1. **the spacing**, down to [`MIN_SPACING`] — the cheapest thing to lose,
//!    because nobody looks at a gap;
//! 2. **the icons**, down to [`crate::dock::MIN_ICON_SIZE`];
//! 3. **the tiles**, through [`Level`] — a wide tile becomes a narrow one and
//!    then a bare value, rather than holding its full width while the icons
//!    beside it shrink to nothing;
//! 4. **and only then** are there more apps than there is bar, which is the
//!    one case where something has to leave the row. It leaves by being put
//!    somewhere reachable rather than by being cut off the end.
//!
//! The width here is a *model* of what GTK will lay out, not a measurement of
//! it, because a measurement cannot be asked "and what if the icons were
//! smaller". A model that disagrees with the real layout is worse than none,
//! so the one invariant it is held to is that it never guesses low:
//! `the_model_never_promises_room_the_bar_does_not_have` checks it against a
//! real measured bar, and everything else here rests on that.

use doca_ipc::Tile;

use crate::dock::{BAR_PADDING, ITEM_PADDING, ITEM_SPACING, MIN_ICON_SIZE, SCREEN_MARGIN};

/// The tightest the icons are allowed to sit.
///
/// Zero, and it still is not zero: every icon carries [`ITEM_PADDING`] on both
/// sides, so shoulders touching is four pixels apart. Which is why this is the
/// first rung — there is a gap left even after it is spent.
pub const MIN_SPACING: i32 = 0;

/// How much of itself a widget tile is still drawing.
///
/// The steps the issue asks for: whole, then one line, then the value alone.
/// A square tile has nothing to give at the first step — it is already one
/// line — so only the wide ones narrow, which is the point: the tiles that
/// cost the most give way first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Full,
    Compact,
    Minimal,
}

impl Level {
    /// How many icons wide a tile is at this level.
    pub fn icons(self, tile: Tile) -> f64 {
        match (self, tile) {
            (Level::Full, tile) => tile.icons(),
            // One line: the label and the value, without the detail under
            // them. A square was always one line and does not move.
            (Level::Compact, Tile::Wide) => 1.6,
            (Level::Compact, Tile::Square) => Tile::SQUARE_ICONS,
            // The value alone, in the room one icon takes.
            (Level::Minimal, _) => Tile::SQUARE_ICONS,
        }
    }

    pub fn width(self, tile: Tile, icon_size: i32) -> i32 {
        (icon_size as f64 * self.icons(tile)).round() as i32
    }
}

/// What the bar settled on, and what it had to give up to get there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fit {
    pub icon: i32,
    pub spacing: i32,
    pub tiles: Level,
    /// How many of the items are on the bar. Everything else is in `overflow`.
    pub shown: i32,
    /// How many did not fit. Zero on every dock that is not absurd.
    pub overflow: i32,
    /// What the whole dock will take across the screen, the room the lens
    /// opens into included. Never more than the screen it was given.
    pub width: i32,
}

impl Fit {
    pub fn gave_nothing_up(&self, preferred: i32) -> bool {
        self.icon == preferred
            && self.spacing == ITEM_SPACING
            && self.tiles == Level::Full
            && self.overflow == 0
    }
}

/// The widest the dock may be: the screen, less the margin it keeps off both
/// edges.
fn room_on(screen_width: i32) -> i32 {
    (screen_width - SCREEN_MARGIN * 2).max(0)
}

/// What a dock of this shape would take, across the screen.
///
/// Counts the room the lens opens into at either end, because that room is
/// inside the *window* even though it is outside the bar — the window is what
/// has to fit on the screen. See `row::Rest::width`.
pub fn width_of(
    items: i32,
    tiles: &[Tile],
    level: Level,
    icon: i32,
    tile_icon: i32,
    spacing: i32,
    magnification: f64,
) -> i32 {
    let run = if items > 0 {
        items * (icon + ITEM_PADDING * 2 + spacing) - spacing
    } else {
        0
    };
    let tile_room: i32 = tiles
        .iter()
        .map(|tile| level.width(*tile, tile_icon) + ITEM_PADDING * 2 + spacing)
        .sum();
    let divider = if tiles.is_empty() { 0 } else { 1 + spacing };
    // A box with nothing in it still measures a pixel, so a dock with nothing
    // on it is a pixel wider than its own padding. Measured rather than
    // reasoned — `the_model_never_promises_room_the_bar_does_not_have` is what
    // found it, and would find it again if GTK ever stopped doing it.
    let empty = i32::from(items == 0 && tiles.is_empty());
    // The same figure the row reserves, arrived at the same way — rounding it
    // once here and twice there is a pixel of disagreement, and a model is only
    // useful while it agrees.
    let lens = crate::magnify::edge_room(icon as f64, magnification) * 2;

    BAR_PADDING * 2 + run + divider + tile_room + lens + empty
}

/// Walk down the ladder until the dock fits the screen, and stop there.
///
/// `tile_icon` is the icon size the *config* asked for rather than the one the
/// row ends up with: a tile is the size the user chose and the icons give up
/// the pixels, which is also what keeps this from being circular — see
/// `widget_tile::room_for`.
pub fn fits(
    items: i32,
    tiles: &[Tile],
    screen_width: i32,
    preferred: i32,
    magnification: f64,
) -> Fit {
    let preferred = preferred.clamp(MIN_ICON_SIZE, crate::dock::MAX_ICON_SIZE);
    let room = room_on(screen_width);
    let items = items.max(0);

    let measure = |shown: i32, icon: i32, spacing: i32, level: Level| {
        width_of(shown, tiles, level, icon, preferred, spacing, magnification)
    };

    // The rungs, in the order the ladder is taken, as one list. Written out
    // rather than nested, because the concessions *accumulate*: once the gap
    // is spent it stays spent while the icons go, and once the icons are spent
    // they stay spent while the tiles narrow. Nesting the loops the other way
    // spends a pixel of icon to keep a pixel of gap, which is backwards — a
    // gap is the one thing here nobody looks at.
    let mut ladder: Vec<(i32, i32, Level)> = Vec::new();
    for spacing in (MIN_SPACING..=ITEM_SPACING).rev() {
        ladder.push((preferred, spacing, Level::Full));
    }
    for icon in (MIN_ICON_SIZE..preferred).rev() {
        ladder.push((icon, MIN_SPACING, Level::Full));
    }
    for level in [Level::Compact, Level::Minimal] {
        ladder.push((MIN_ICON_SIZE, MIN_SPACING, level));
    }

    for (icon, spacing, level) in ladder {
        let width = measure(items, icon, spacing, level);
        if width <= room {
            return Fit {
                icon,
                spacing,
                tiles: level,
                shown: items,
                overflow: 0,
                width,
            };
        }
    }

    // Everything is spent and it still does not fit: there are more apps than
    // there is screen. One slot is kept for the way to reach the rest, or the
    // ones that come off the end would simply be gone.
    let (icon, spacing, level) = (MIN_ICON_SIZE, MIN_SPACING, Level::Minimal);
    let mut shown = items;
    while shown > 0 && measure(shown + 1, icon, spacing, level) > room {
        shown -= 1;
    }
    Fit {
        icon,
        spacing,
        tiles: level,
        shown,
        overflow: items - shown,
        width: measure(shown + 1, icon, spacing, level).min(room),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use gtk::prelude::*;

    /// The promise the rest of this module rests on: the model never says
    /// there is room that the bar, really laid out, does not have.
    ///
    /// Everything here reasons about a width nobody measured. If the model
    /// guessed low the ladder would stop a rung early and GTK would squeeze
    /// the difference out of the icons — which is the shoving this was written
    /// to end. So it is checked against a bar that was actually built, at the
    /// sizes and shapes the dock really takes.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn the_model_never_promises_room_the_bar_does_not_have() {
        let css = gtk::CssProvider::new();
        css.load_from_data(crate::theme::css(crate::theme::DEFAULT).as_bytes())
            .expect("the default sheet parses");
        gtk::StyleContext::add_provider_for_screen(
            &gdk::Screen::default().expect("no screen"),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        for items in [0usize, 1, 8, 28] {
            for widgets in [0usize, 1, 4] {
                for icon in [MIN_ICON_SIZE, 48, crate::dock::MAX_ICON_SIZE] {
                    for spacing in [MIN_SPACING, ITEM_SPACING] {
                        let real = measured(items, widgets, icon, spacing);
                        let model = width_of(
                            items as i32,
                            &vec![Tile::Wide; widgets],
                            Level::Full,
                            icon,
                            icon,
                            spacing,
                            LENS,
                        );

                        assert!(
                            model >= real,
                            "{items} apps, {widgets} widgets at {icon}px/{spacing}: \
                             the model promised {model}px where the bar wants {real}px"
                        );
                    }
                }
            }
        }
    }

    /// A bar built the way `rebuild` builds one, measured.
    fn measured(items: usize, widgets: usize, icon: i32, spacing: i32) -> i32 {
        let row = crate::row::Row::new();
        let list: Vec<doca_ipc::DockItem> = (0..items)
            .map(|at| doca_ipc::DockItem {
                id: format!("app{at}"),
                name: format!("app{at}"),
                icon: "application-x-executable".into(),
                pinned: true,
                windows: Vec::new(),
                active: false,
            })
            .collect();
        row.fill(
            &list,
            crate::row::Rest::new(icon, spacing, ITEM_PADDING, LENS),
        );

        let shelf = crate::widget_tile::Shelf::new();
        let states: Vec<doca_ipc::WidgetState> = (0..widgets)
            .map(|at| crate::widget_tile::tests::simple(&format!("w{at}")))
            .collect();

        let inner = gtk::Box::new(gtk::Orientation::Horizontal, spacing);
        inner.set_halign(gtk::Align::Center);
        inner.add(&row.perch);
        let look = crate::widget_tile::Look {
            icon_size: icon,
            palette: crate::theme::palette(crate::theme::DEFAULT),
            level: Level::Full,
        };
        let expand: crate::widget_tile::Expand = std::rc::Rc::new(|_: &str, _: &gtk::Widget| {});
        shelf.show(&inner, &states, look, &expand);

        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        bar.set_widget_name("bar");
        bar.pack_start(&inner, true, true, 0);
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.add(&bar);
        window.show_all();

        let natural = crate::dock::natural_size(&bar);
        let answer = natural.0 + crate::magnify::edge_room(icon as f64, LENS) * 2;
        window.close();
        answer
    }


    const SCREEN: i32 = 1920;
    const LENS: f64 = 1.6;
    const ICON: i32 = 48;

    fn wide(count: usize) -> Vec<Tile> {
        vec![Tile::Wide; count]
    }

    /// The ordinary dock, which is most docks: nothing is given up at all.
    #[test]
    fn a_dock_that_fits_gives_up_nothing() {
        let fit = fits(12, &wide(3), SCREEN, ICON, LENS);

        assert!(fit.gave_nothing_up(ICON), "{fit:?}");
    }

    /// The order the issue asks for, read off the ladder one rung at a time:
    /// the gap goes before a pixel of icon does, and every icon is spent
    /// before a tile gives up a line.
    #[test]
    fn the_gap_is_the_first_thing_spent_and_the_tiles_the_last() {
        let crowded = |items: i32| fits(items, &wide(4), SCREEN, ICON, LENS);

        let mut seen_tight_spacing = false;
        let mut seen_small_icons = false;
        for items in 1..=80 {
            let fit = crowded(items);
            if fit.spacing < ITEM_SPACING {
                seen_tight_spacing = true;
            }
            if fit.icon < ICON {
                assert!(
                    seen_tight_spacing,
                    "an icon shrank at {items} while there was still a gap to spend: {fit:?}"
                );
                seen_small_icons = true;
            }
            if fit.tiles != Level::Full {
                assert!(
                    seen_small_icons && fit.icon == MIN_ICON_SIZE,
                    "a tile narrowed at {items} with icons still to spend: {fit:?}"
                );
            }
        }
        assert!(seen_small_icons, "80 apps and four tiles never crowded 1920px");
    }

    /// What the issue asks for in as many words: for any dock anyone could
    /// have, on any screen anyone could have, the bar fits and the icons stay
    /// legible. The old answer failed this by cutting; this one fails it only
    /// if the ladder has a rung missing.
    #[test]
    fn no_dock_on_any_screen_is_ever_wider_than_the_screen() {
        for items in 0..=80 {
            for widgets in 0..=12 {
                for screen in [800, 1024, 1280, 1366, 1920, 2560, 3440, 5120] {
                    let tiles = wide(widgets);
                    let fit = fits(items, &tiles, screen, ICON, LENS);

                    assert!(
                        fit.width <= room_on(screen),
                        "{items} apps and {widgets} widgets took {}px of {screen}px: {fit:?}",
                        fit.width
                    );
                    assert!(
                        fit.icon >= MIN_ICON_SIZE,
                        "icons fell below the floor: {fit:?}"
                    );
                    assert_eq!(
                        fit.shown + fit.overflow,
                        items,
                        "an app went missing rather than overflowing: {fit:?}"
                    );
                }
            }
        }
    }

    /// Overflow is the last rung, not an early one: a dock only sheds apps
    /// once every other thing has been given up.
    #[test]
    fn nothing_overflows_until_everything_else_has_been_spent() {
        for items in 0..=80 {
            for screen in [800, 1280, 1920, 3440] {
                let fit = fits(items, &wide(4), screen, ICON, LENS);

                if fit.overflow > 0 {
                    assert_eq!(fit.icon, MIN_ICON_SIZE, "{fit:?}");
                    assert_eq!(fit.spacing, MIN_SPACING, "{fit:?}");
                    assert_eq!(fit.tiles, Level::Minimal, "{fit:?}");
                }
            }
        }
    }

    /// A wider screen can never be a worse dock.
    #[test]
    fn more_screen_is_never_less_dock() {
        for items in [1, 8, 28, 60, 80] {
            let mut previous: Option<Fit> = None;
            for screen in [800, 1024, 1280, 1366, 1920, 2560, 3440, 5120] {
                let fit = fits(items, &wide(4), screen, ICON, LENS);
                if let Some(narrower) = previous {
                    assert!(fit.icon >= narrower.icon, "{items} apps: {fit:?}");
                    assert!(fit.spacing >= narrower.spacing, "{items} apps: {fit:?}");
                    assert!(fit.tiles <= narrower.tiles, "{items} apps: {fit:?}");
                    assert!(fit.overflow <= narrower.overflow, "{items} apps: {fit:?}");
                }
                previous = Some(fit);
            }
        }
    }

    /// And one more app is never a roomier dock.
    #[test]
    fn one_more_app_never_buys_the_others_more_room() {
        let mut previous = fits(0, &wide(3), SCREEN, ICON, LENS);

        for items in 1..=80 {
            let fit = fits(items, &wide(3), SCREEN, ICON, LENS);

            assert!(fit.icon <= previous.icon, "at {items}: {fit:?}");
            assert!(fit.spacing <= previous.spacing, "at {items}: {fit:?}");
            assert!(fit.tiles >= previous.tiles, "at {items}: {fit:?}");
            previous = fit;
        }
    }

    /// Where `widget_tile::slot_of` used to say it: a tile sits in the same
    /// slot an icon would, so the bar's width is the sum of its slots. A tile
    /// measured any other way is a bar that is wider or narrower than it says.
    #[test]
    fn a_tile_is_charged_the_same_frame_an_icon_is() {
        let frame = ITEM_PADDING * 2 + ITEM_SPACING;
        let bare = width_of(0, &[], Level::Full, ICON, ICON, ITEM_SPACING, LENS);

        let square = width_of(0, &[Tile::Square], Level::Full, ICON, ICON, ITEM_SPACING, LENS);
        let wide = width_of(0, &[Tile::Wide], Level::Full, ICON, ICON, ITEM_SPACING, LENS);

        // The divider arrives with the first tile, and the empty bar's pixel
        // leaves with it.
        let divider = 1 + ITEM_SPACING - 1;
        assert_eq!(square - bare, ICON + frame + divider);
        assert_eq!(wide - bare, (ICON as f64 * 2.5) as i32 + frame + divider);
    }

    #[test]
    fn the_tiles_are_charged_one_by_one_and_nothing_else_is() {
        let three = width_of(
            0,
            &[Tile::Wide, Tile::Square, Tile::Wide],
            Level::Full,
            ICON,
            ICON,
            ITEM_SPACING,
            LENS,
        );
        let one = width_of(0, &[Tile::Wide], Level::Full, ICON, ICON, ITEM_SPACING, LENS);
        let frame = ITEM_PADDING * 2 + ITEM_SPACING;

        assert_eq!(
            three - one,
            (ICON as f64 * 2.5) as i32 + frame + ICON + frame,
            "the bar charged for something other than its tiles"
        );
    }

    #[test]
    fn an_empty_dock_still_answers() {
        let fit = fits(0, &[], SCREEN, ICON, LENS);

        assert_eq!(fit.shown, 0);
        assert_eq!(fit.overflow, 0);
        assert!(fit.width <= room_on(SCREEN));
    }

    /// A square tile is already one line, so the first step down asks nothing
    /// of it — the tiles that cost the most are the ones that give way.
    #[test]
    fn narrowing_the_tiles_takes_it_out_of_the_wide_ones() {
        let wide_full = Level::Full.width(Tile::Wide, ICON);
        let wide_compact = Level::Compact.width(Tile::Wide, ICON);

        assert!(wide_compact < wide_full);
        assert_eq!(
            Level::Compact.width(Tile::Square, ICON),
            Level::Full.width(Tile::Square, ICON)
        );
        for tile in [Tile::Wide, Tile::Square] {
            assert_eq!(Level::Minimal.width(tile, ICON), ICON);
        }
    }

    // ── What `dock::icon_size_for` knew ───────────────────────────────────
    //
    // The ladder replaced it, so its tests moved here rather than going with
    // it. Two did not come: both walked a count asserting the result fits the
    // screen, which `no_dock_on_any_screen_is_ever_wider_than_the_screen` now
    // says over a far wider range and without the `size > MIN_ICON_SIZE`
    // escape hatch they needed.

    const NO_LENS: f64 = 1.0;

    fn icon_for(items: i32, tiles: &[Tile], preferred: i32, lens: f64) -> i32 {
        fits(items, tiles, SCREEN, preferred, lens).icon
    }

    #[test]
    fn a_smaller_preferred_size_is_honoured_even_when_there_is_room_to_spare() {
        assert_eq!(icon_for(4, &wide(0), 32, NO_LENS), 32);
    }

    #[test]
    fn an_icon_size_larger_than_the_default_is_honoured_when_there_is_room() {
        assert_eq!(icon_for(6, &wide(2), 72, NO_LENS), 72);
    }

    #[test]
    fn no_config_can_ask_for_an_icon_larger_than_the_dock_allows() {
        assert_eq!(icon_for(4, &wide(0), 400, NO_LENS), crate::dock::MAX_ICON_SIZE);
    }

    #[test]
    fn a_crowded_dock_of_twenty_nine_apps_still_leaves_the_icons_legible() {
        let crowded = icon_for(29, &wide(4), ICON, LENS);

        assert!(crowded >= 32, "{crowded}px icons are too small to recognise");
    }

    #[test]
    fn turning_the_lens_off_gives_the_icons_the_room_it_was_holding() {
        let with_lens = icon_for(40, &wide(4), ICON, LENS);
        let without = icon_for(40, &wide(4), ICON, NO_LENS);

        assert!(
            without > with_lens,
            "a dock with no magnification should spend that room on the icons"
        );
    }

    #[test]
    fn a_preferred_size_never_overrides_the_need_to_fit() {
        assert!(icon_for(40, &wide(4), ICON, NO_LENS) < ICON);
    }

    #[test]
    fn a_handful_of_apps_keeps_icons_at_full_size() {
        assert_eq!(icon_for(6, &wide(2), ICON, NO_LENS), ICON);
    }

    #[test]
    fn twenty_eight_pinned_apps_and_two_widgets_fit_at_full_size() {
        // What the tighter moulding bought, and the ladder keeps: at a frame
        // of twelve pixels a slot a dock this full had to drop below the size
        // the config asked for just to fit.
        assert_eq!(icon_for(28, &wide(2), ICON, NO_LENS), ICON);
    }

    /// What a declared tile size costs, named rather than discovered: a wide
    /// tile is two and a half icons where every tile used to be 88 pixels.
    #[test]
    fn a_wide_tile_is_paid_for_somewhere_on_a_dock_that_is_already_full() {
        let two = fits(28, &wide(2), SCREEN, ICON, NO_LENS);
        let four = fits(28, &wide(4), SCREEN, ICON, NO_LENS);

        assert!(two.gave_nothing_up(ICON), "{two:?}");
        assert!(
            !four.gave_nothing_up(ICON),
            "four wide tiles cost nothing, which cannot be"
        );
        assert!(
            four.icon >= 40,
            "{}px icons are too small for four tiles to be worth",
            four.icon
        );
    }

    /// A square tile is the cheap one, which is the point of there being two
    /// sizes at all.
    #[test]
    fn a_square_tile_costs_less_of_the_bar_than_a_wide_one() {
        let square = vec![Tile::Square; 4];

        assert!(icon_for(40, &square, ICON, NO_LENS) > icon_for(40, &wide(4), ICON, NO_LENS));
    }

    #[test]
    fn icons_never_shrink_below_the_point_of_being_recognisable() {
        assert_eq!(icon_for(500, &wide(4), ICON, NO_LENS), MIN_ICON_SIZE);
    }

    #[test]
    fn an_empty_dock_does_not_divide_by_zero_sizing_its_icons() {
        assert_eq!(icon_for(0, &wide(0), ICON, NO_LENS), ICON);
    }
}
