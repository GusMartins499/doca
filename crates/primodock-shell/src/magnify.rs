use std::f64::consts::PI;

pub const DEFAULT_SCALE: f64 = 1.6;

pub fn radius_for(base_slot: f64) -> f64 {
    base_slot * 2.5
}

pub fn scale_at(distance: f64, radius: f64, max_scale: f64) -> f64 {
    let distance = distance.abs();
    if radius <= 0.0 || distance >= radius {
        return 1.0;
    }
    let falloff = (1.0 + (PI * distance / radius).cos()) / 2.0;
    1.0 + (max_scale - 1.0) * falloff
}

pub fn base_centre(index: usize, base_slot: f64, spacing: f64, offset: f64) -> f64 {
    offset + index as f64 * (base_slot + spacing) + base_slot / 2.0
}

pub fn sizes_under_pointer(
    count: usize,
    pointer_x: Option<f64>,
    base_icon: f64,
    base_slot: f64,
    spacing: f64,
    offset: f64,
    max_scale: f64,
) -> Vec<i32> {
    let Some(pointer_x) = pointer_x else {
        return vec![base_icon.round() as i32; count];
    };
    let radius = radius_for(base_slot);
    (0..count)
        .map(|index| {
            let centre = base_centre(index, base_slot, spacing, offset);
            let scale = scale_at(pointer_x - centre, radius, max_scale);
            (base_icon * scale).round() as i32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLOT: f64 = 64.0;
    const ICON: f64 = 48.0;
    const SPACING: f64 = 6.0;

    #[test]
    fn an_icon_directly_under_the_pointer_reaches_full_scale() {
        assert!((scale_at(0.0, 160.0, 1.6) - 1.6).abs() < 1e-9);
    }

    #[test]
    fn an_icon_beyond_the_lens_is_left_at_its_own_size() {
        assert_eq!(scale_at(160.0, 160.0, 1.6), 1.0);
        assert_eq!(scale_at(500.0, 160.0, 1.6), 1.0);
    }

    #[test]
    fn the_lens_is_symmetric_on_both_sides_of_the_pointer() {
        for distance in [10.0, 50.0, 120.0] {
            assert_eq!(
                scale_at(distance, 160.0, 1.6),
                scale_at(-distance, 160.0, 1.6)
            );
        }
    }

    #[test]
    fn the_lens_falls_off_without_a_step_anywhere_along_it() {
        let radius = 160.0;
        let mut previous = scale_at(0.0, radius, 1.6);

        for step in 1..=160 {
            let current = scale_at(step as f64, radius, 1.6);
            assert!(current <= previous + 1e-9, "scale grew with distance");
            assert!(
                previous - current < 0.02,
                "a {:.4} jump at {step}px would read as a jolt",
                previous - current
            );
            previous = current;
        }
    }

    #[test]
    fn a_pointer_outside_the_bar_leaves_every_icon_alone() {
        let sizes = sizes_under_pointer(6, None, ICON, SLOT, SPACING, 0.0, 1.6);

        assert_eq!(sizes, vec![48; 6]);
    }

    #[test]
    fn the_icon_under_the_pointer_is_the_largest_of_the_row() {
        let centre_of_third = base_centre(2, SLOT, SPACING, 0.0);

        let sizes = sizes_under_pointer(6, Some(centre_of_third), ICON, SLOT, SPACING, 0.0, 1.6);

        assert_eq!(*sizes.iter().max().unwrap(), sizes[2]);
        assert!(sizes[2] > sizes[1]);
        assert!(sizes[1] > sizes[0]);
    }

    #[test]
    fn icons_far_from_the_pointer_keep_their_base_size() {
        let sizes = sizes_under_pointer(20, Some(0.0), ICON, SLOT, SPACING, 0.0, 1.6);

        assert_eq!(sizes[19], 48);
    }

    #[test]
    fn no_icon_ever_exceeds_the_scale_it_was_given() {
        for pointer in 0..600 {
            let sizes =
                sizes_under_pointer(8, Some(pointer as f64), ICON, SLOT, SPACING, 0.0, 1.6);
            for size in sizes {
                assert!(size <= (ICON * 1.6).round() as i32);
                assert!(size >= ICON as i32);
            }
        }
    }

    #[test]
    fn the_lens_is_measured_against_the_base_layout_so_it_cannot_chase_itself() {
        let pointer = base_centre(3, SLOT, SPACING, 0.0);

        let first = sizes_under_pointer(8, Some(pointer), ICON, SLOT, SPACING, 0.0, 1.6);
        let again = sizes_under_pointer(8, Some(pointer), ICON, SLOT, SPACING, 0.0, 1.6);

        assert_eq!(first, again);
    }

    #[test]
    fn a_scale_of_one_is_the_same_as_no_magnification_at_all() {
        let sizes = sizes_under_pointer(6, Some(100.0), ICON, SLOT, SPACING, 0.0, 1.0);

        assert_eq!(sizes, vec![48; 6]);
    }
}
