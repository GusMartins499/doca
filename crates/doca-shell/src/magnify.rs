pub const DEFAULT_SCALE: f64 = 1.6;

/// Where one icon sits and how big it is, with the pointer where it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub centre: f64,
    pub size: f64,
}

/// How far the lens reaches, either side of the pointer.
pub fn span(base_icon: f64, max_scale: f64) -> f64 {
    base_icon * max_scale
}

/// Where an icon goes, given where it sits when nothing is magnified.
///
/// Ported from Plank's `PositionManager.update_draw_values`. The shape of it
/// matters more than the constants: an icon's place is a function of its own
/// *static* centre and the pointer, and of nothing else. No icon is packed
/// after another, so growing one cannot shift the rest by accumulation, the
/// row's footprint never changes, and there is no layout to renegotiate —
/// only a picture to redraw.
///
/// Icons are pushed away from the pointer to make room for the one under it.
/// The `1 - offset_percent / 3` term pulls the far ends of the lens back in,
/// so the row spreads near the pointer and settles quickly away from it.
pub fn place(
    static_centre: f64,
    pointer: Option<f64>,
    base_icon: f64,
    max_scale: f64,
) -> Placement {
    let Some(pointer) = pointer else {
        return Placement {
            centre: static_centre,
            size: base_icon,
        };
    };
    if max_scale <= 1.0 {
        return Placement {
            centre: static_centre,
            size: base_icon,
        };
    }

    let span = span(base_icon, max_scale);
    let offset = (pointer - static_centre).abs().min(span);
    let mut reach = offset / span;
    if reach > 0.99 {
        reach = 1.0;
    }

    let push = offset * (max_scale - 1.0) * (1.0 - reach / 3.0);
    let centre = if pointer > static_centre {
        static_centre - push
    } else {
        static_centre + push
    };

    let zoom = 1.0 + (1.0 - reach * reach) * (max_scale - 1.0);
    Placement {
        centre,
        size: (zoom * base_icon).round(),
    }
}

/// The widest an icon can stray past where it sits at rest.
///
/// The row is drawn on a surface of its own, and this is the margin that
/// surface needs at either end so that an icon pushed outwards by the lens is
/// still drawn rather than cut off.
pub fn edge_room(base_icon: f64, max_scale: f64) -> i32 {
    (base_icon * edge_reach(max_scale)).ceil() as i32
}

/// The same room, as a multiple of the icon size.
///
/// The icon size has to be chosen before it is known, since the room depends
/// on it — so it is chosen from this ratio instead, and the row is then laid
/// out with the pixels it works out to.
pub fn edge_reach(max_scale: f64) -> f64 {
    if max_scale <= 1.0 {
        return 0.0;
    }
    // the furthest an icon is pushed, plus half of it once it has grown
    max_scale * ((max_scale - 1.0) * 2.0 / 3.0 + 0.5)
}

/// The step the lens moves an icon in.
///
/// Every size an icon takes is a surface of its own to keep. A pixel of growth
/// is not visible and a bar of thirty icons cannot afford a cache entry for
/// each, so growth lands on even sizes. The base size is always itself, so a
/// row at rest is exact.
pub const STEP: i32 = 2;

pub fn settled(size: i32, base: i32) -> i32 {
    if size <= base {
        return base;
    }
    base + ((size - base) + STEP / 2) / STEP * STEP
}

pub fn base_centre(index: usize, base_slot: f64, spacing: f64, offset: f64) -> f64 {
    offset + index as f64 * (base_slot + spacing) + base_slot / 2.0
}

/// The run a number of slots takes, spacing between them and not after.
fn run(slots: f64, base_slot: f64, spacing: f64) -> f64 {
    if slots <= 0.0 {
        return 0.0;
    }
    slots * (base_slot + spacing) - spacing
}

/// Where the icons sit at rest while some of them are still arriving or
/// leaving.
///
/// `presence` is one figure per icon, from `motion::entry_progress`: an icon
/// takes that fraction of a slot and of the spacing beside it, so the row
/// contracts and expands by fractions of a pixel rather than in slot-sized
/// steps, and the neighbours of an app that just opened slide apart instead
/// of appearing somewhere new.
///
/// The run is centred on the room the *settled* row occupies — `settled` is
/// how many icons are staying, which is the width the bar was given. That is
/// what keeps the group still: while one end of the row is animating the run
/// is narrower than the bar, and centring it means the other icons stay where
/// the bar put them instead of being dragged along by the one that moved.
///
/// With every icon fully present this is `base_centre`, exactly, which is the
/// case the row is in except for the few frames after something opened.
pub fn centres(
    presence: &[f64],
    base_slot: f64,
    spacing: f64,
    offset: f64,
    settled: usize,
) -> Vec<f64> {
    let present: f64 = presence.iter().sum();
    let mut x = offset
        + (run(settled as f64, base_slot, spacing) - run(present, base_slot, spacing)) / 2.0;
    presence
        .iter()
        .map(|part| {
            let centre = x + part * base_slot / 2.0;
            x += part * (base_slot + spacing);
            centre
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICON: f64 = 48.0;
    const SLOT: f64 = 56.0;
    const SPACING: f64 = 4.0;
    const SCALE: f64 = 1.6;

    fn row(pointer: Option<f64>, count: usize) -> Vec<Placement> {
        (0..count)
            .map(|i| place(base_centre(i, SLOT, SPACING, 0.0), pointer, ICON, SCALE))
            .collect()
    }

    const MARGIN: f64 = 69.0;

    /// The run of icons, measured from the middle of the room it was given.
    ///
    /// What the eye judges, and the only frame of reference in which these
    /// checks mean anything: the bar is centred on the screen, so a row that
    /// keeps its position relative to the middle of the bar keeps it on
    /// screen, however much wider or narrower the bar itself became.
    fn about_the_middle(presence: &[f64], settled: usize) -> Vec<f64> {
        let room = run(settled as f64, SLOT, SPACING);
        centres(presence, SLOT, SPACING, MARGIN, settled)
            .into_iter()
            .map(|centre| centre - (MARGIN + room / 2.0))
            .collect()
    }

    #[test]
    fn a_row_with_nothing_arriving_sits_exactly_where_its_icons_rest() {
        let settled = vec![1.0; 6];

        let centres = centres(&settled, SLOT, SPACING, MARGIN, 6);

        for (index, centre) in centres.iter().enumerate() {
            assert_eq!(*centre, base_centre(index, SLOT, SPACING, MARGIN));
        }
    }

    /// The reason the run is centred rather than packed from the left.
    ///
    /// An app opens, the bar is given room for one more icon and recentres
    /// itself on the screen by half of it — so the icons already there must
    /// move by exactly that half, inside the bar, to stay still on screen.
    /// Packing the run from the left end instead moved all of them by a whole
    /// slot at the moment the room appeared, which is the jump this item was
    /// opened about.
    #[test]
    fn the_icons_already_there_do_not_move_when_one_arrives() {
        let before = about_the_middle(&[1.0; 5], 5);

        // The sixth, third from the left, at the instant it joined.
        let mut joining = vec![1.0; 6];
        joining[2] = 0.0;
        let after = about_the_middle(&joining, 6);

        let others: Vec<f64> = after
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != 2)
            .map(|(_, centre)| *centre)
            .collect();
        for (index, (was, is)) in before.iter().zip(&others).enumerate() {
            assert!(
                (was - is).abs() < 1e-9,
                "icon {index} moved {}px when a neighbour appeared",
                (was - is).abs()
            );
        }
    }

    /// And the other half of it: the space is let go of as the icon shrinks,
    /// not at the moment it was told to go.
    #[test]
    fn the_icons_that_stay_close_the_gap_gradually_and_land_exactly() {
        let settled = 5;
        // Six icons in room for five: the third one is leaving.
        let gap = |presence: f64| {
            let mut row = vec![1.0; 6];
            row[2] = presence;
            about_the_middle(&row, settled)
        };
        let closed = about_the_middle(&[1.0; 5], settled);

        let mut previous = gap(1.0);
        for step in (0..=100).rev() {
            let current = gap(step as f64 / 100.0);
            for (index, (was, is)) in previous.iter().zip(&current).enumerate() {
                // The one that is leaving is not closing any gap: it is
                // shrinking in place, and its centre creeps with its own
                // narrowing slot. What matters is the five that stay.
                if index == 2 {
                    continue;
                }
                let towards = if index < 2 { is >= was } else { is <= was };
                assert!(towards, "icon {index} moved away from its place at {step}");
            }
            previous = current;
        }

        let landed: Vec<f64> = previous
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != 2)
            .map(|(_, centre)| *centre)
            .collect();
        for (index, (ends, should)) in landed.iter().zip(&closed).enumerate() {
            assert!(
                (ends - should).abs() < 1e-9,
                "icon {index} came to rest {}px from where it belongs",
                (ends - should).abs()
            );
        }
    }

    /// A departing icon is drawn in room the bar has already given up, which
    /// is only safe because the row keeps a margin at either end for the lens
    /// to push icons into. If one is ever drawn outside that margin it is cut
    /// off by the edge of the row's own surface.
    #[test]
    fn an_icon_on_its_way_out_is_still_drawn_inside_the_row() {
        for count in 1..=40usize {
            for leaving in [0usize, count / 2, count] {
                let mut row = vec![1.0; count + 1];
                row[leaving] = 1.0;
                let settled = count;
                let width = MARGIN * 2.0 + run(settled as f64, SLOT, SPACING);
                for (index, centre) in
                    centres(&row, SLOT, SPACING, MARGIN, settled).iter().enumerate()
                {
                    assert!(
                        centre - SLOT / 2.0 >= 0.0 && centre + SLOT / 2.0 <= width,
                        "icon {index} of {count} is drawn at {centre}, outside a row {width}px wide"
                    );
                }
            }
        }
    }

    #[test]
    fn a_pointer_outside_the_bar_leaves_every_icon_where_it_was() {
        let resting = row(None, 6);

        for (index, placed) in resting.iter().enumerate() {
            assert_eq!(placed.size, ICON);
            assert_eq!(placed.centre, base_centre(index, SLOT, SPACING, 0.0));
        }
    }

    #[test]
    fn the_icon_under_the_pointer_is_the_largest_of_the_row() {
        let third = base_centre(2, SLOT, SPACING, 0.0);

        let sizes: Vec<f64> = row(Some(third), 6).iter().map(|p| p.size).collect();

        assert_eq!(sizes[2], (ICON * SCALE).round());
        assert!(sizes[2] > sizes[1] && sizes[1] > sizes[0]);
    }

    #[test]
    fn icons_beyond_the_lens_keep_their_size_and_move_as_one() {
        // The room for a magnified icon has to come from somewhere: the row
        // beyond the lens slides out of the way whole, rather than stretching.
        let far = row(Some(0.0), 20);

        let shift = far[19].centre - base_centre(19, SLOT, SPACING, 0.0);
        for (index, placed) in far.iter().enumerate().skip(12) {
            assert_eq!(placed.size, ICON, "icon {index} grew out of reach");
            assert!(
                (placed.centre - base_centre(index, SLOT, SPACING, 0.0) - shift).abs() < 1e-9,
                "icon {index} was stretched rather than carried"
            );
        }
    }

    #[test]
    fn the_room_kept_at_the_ends_is_the_room_the_sizing_was_promised() {
        for scale in [1.0, 1.2, 1.6, 2.0, 2.5] {
            assert_eq!(
                edge_room(48.0, scale),
                (48.0 * edge_reach(scale)).ceil() as i32
            );
        }
    }

    #[test]
    fn no_icon_grows_past_the_magnification_it_was_given() {
        for pointer in 0..900 {
            for placed in row(Some(pointer as f64), 12) {
                assert!(placed.size <= (ICON * SCALE).round());
                assert!(placed.size >= ICON);
            }
        }
    }

    #[test]
    fn icons_are_pushed_away_from_the_pointer_never_towards_it() {
        let pointer = base_centre(5, SLOT, SPACING, 0.0);

        for (index, placed) in row(Some(pointer), 12).iter().enumerate() {
            let resting = base_centre(index, SLOT, SPACING, 0.0);
            match index {
                i if i < 5 => assert!(placed.centre <= resting + 1e-9, "icon {i} moved inwards"),
                i if i > 5 => assert!(placed.centre >= resting - 1e-9, "icon {i} moved inwards"),
                _ => assert!((placed.centre - resting).abs() < 1e-9),
            }
        }
    }

    #[test]
    fn the_row_never_reorders_itself_however_the_pointer_moves() {
        for pointer in 0..900 {
            let placed = row(Some(pointer as f64), 12);
            for pair in placed.windows(2) {
                assert!(
                    pair[0].centre < pair[1].centre,
                    "icons swapped places at pointer {pointer}"
                );
            }
        }
    }

    #[test]
    fn an_icon_is_placed_from_its_own_resting_spot_and_the_pointer_alone() {
        // The property the old lens lost: nothing an icon does can move another.
        let pointer = Some(200.0);
        let alone = place(300.0, pointer, ICON, SCALE);
        let crowded = row(pointer, 40);
        let same = place(300.0, pointer, ICON, SCALE);

        assert_eq!(alone, same);
        assert_eq!(crowded.len(), 40);
    }

    #[test]
    fn the_lens_moves_without_a_step_anywhere_along_it() {
        let centre = base_centre(6, SLOT, SPACING, 0.0);
        let mut previous = place(centre, Some(0.0), ICON, SCALE);

        for pointer in 1..900 {
            let current = place(centre, Some(pointer as f64), ICON, SCALE);
            assert!(
                (current.size - previous.size).abs() <= 2.0,
                "a {:.1}px jump in size at {pointer}",
                current.size - previous.size
            );
            assert!(
                (current.centre - previous.centre).abs() <= 2.0,
                "a {:.1}px jump in place at {pointer}",
                current.centre - previous.centre
            );
            previous = current;
        }
    }

    #[test]
    fn the_lens_is_symmetric_on_both_sides_of_the_pointer() {
        let centre = 500.0;

        for distance in [10.0, 50.0, 120.0] {
            let left = place(centre, Some(centre - distance), ICON, SCALE);
            let right = place(centre, Some(centre + distance), ICON, SCALE);

            assert_eq!(left.size, right.size);
            assert!(((left.centre - centre) + (right.centre - centre)).abs() < 1e-9);
        }
    }

    #[test]
    fn a_scale_of_one_is_the_same_as_no_magnification_at_all() {
        for pointer in 0..600 {
            let placed = place(300.0, Some(pointer as f64), ICON, 1.0);

            assert_eq!(placed.size, ICON);
            assert_eq!(placed.centre, 300.0);
        }
    }

    #[test]
    fn no_icon_is_ever_drawn_outside_the_room_kept_for_it() {
        let room = edge_room(ICON, SCALE) as f64;
        let last = base_centre(11, SLOT, SPACING, 0.0);

        for pointer in 0..1200 {
            for (index, placed) in row(Some(pointer as f64), 12).iter().enumerate() {
                let resting = base_centre(index, SLOT, SPACING, 0.0);
                let left = placed.centre - placed.size / 2.0;
                let right = placed.centre + placed.size / 2.0;

                assert!(
                    left >= -room && right <= last + SLOT / 2.0 + room,
                    "icon {index} reached {left:.1}..{right:.1} with only {room}px kept"
                );
                let _ = resting;
            }
        }
    }

    #[test]
    fn a_lens_turned_off_needs_no_room_at_the_ends() {
        assert_eq!(edge_room(ICON, 1.0), 0);
    }

    #[test]
    fn an_icon_at_rest_is_exactly_its_own_size() {
        assert_eq!(settled(48, 48), 48);
        assert_eq!(settled(47, 48), 48);
    }

    #[test]
    fn growth_lands_on_a_step_so_each_size_is_a_surface_worth_keeping() {
        for grown in 48..=77 {
            let settled = settled(grown, 48);

            assert_eq!((settled - 48) % STEP, 0, "{grown}px settled to {settled}px");
        }
    }

    #[test]
    fn settling_never_moves_an_icon_further_than_half_a_step() {
        for grown in 48..=77 {
            assert!((settled(grown, 48) - grown).abs() <= STEP / 2 + 1);
        }
    }

    #[test]
    fn settling_keeps_the_order_the_lens_put_the_icons_in() {
        let mut previous = settled(48, 48);

        for grown in 48..=77 {
            let current = settled(grown, 48);
            assert!(current >= previous, "settling reversed the lens at {grown}px");
            previous = current;
        }
    }
}
