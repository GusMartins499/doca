use std::time::Duration;

pub const SLIDE: Duration = Duration::from_millis(160);
/// A frame on a sixty-hertz screen. Nothing animates on this any more — both
/// the bar and the row ride the compositor's own clock — and it is kept as the
/// yardstick for how far a thing may move between two frames.
#[cfg(test)]
pub const FRAME: Duration = Duration::from_millis(16);
pub const PEEK: i32 = 2;
pub const LAUNCH: Duration = Duration::from_millis(700);
pub const LAUNCH_SWELL: f64 = 0.28;
pub const LAUNCH_PULSES: f64 = 2.0;

/// How long the lens takes to open, and to close again. Plank's figure.
pub const ZOOM: Duration = Duration::from_millis(200);

/// The fastest the lens travels along the bar when it has to catch up.
///
/// Measured rather than picked. With 48px icons, a pointer swept across the
/// bar at 2000 px/s already moves an icon 18px between two frames, and at
/// 3000 px/s, 27px — so no speed a hand produces is held back by this. A
/// pointer that did not travel at all is the case it is for: one that left
/// the bar at one end and came back at the other, which aims the lens
/// somewhere new between two frames and moves an icon 61px at once. Capped,
/// that jump is spread over about six frames, none of them larger than a
/// brisk sweep already is.
pub const AIM_SPEED: f64 = 3000.0;

pub fn ease_out(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

pub fn ease_in(t: f64) -> f64 {
    t.clamp(0.0, 1.0).powi(3)
}

/// How far open the lens is, `elapsed` after the pointer arrived or left.
///
/// It opens quickly and settles, and closes the other way round — the shape
/// Plank uses. Reversing mid-way is the caller's job: see `Row::hover`.
pub fn zoom_progress(elapsed: Duration, opening: bool) -> f64 {
    if elapsed >= ZOOM {
        return if opening { 1.0 } else { 0.0 };
    }
    let t = elapsed.as_secs_f64() / ZOOM.as_secs_f64();
    if opening {
        ease_out(t)
    } else {
        1.0 - ease_in(t)
    }
}

/// Where the lens is aimed `step` later, closing on the pointer.
///
/// A flat speed, and the one thing in this file that does not ease. An ease
/// would have to start slowly, and the lens is already in the wrong place
/// when this begins; and easing towards a *moving* target — which a pointer
/// is — restarts its curve every frame and never arrives. What this needs is
/// not a curve but a ceiling: under it the lens is exactly where the pointer
/// is and nothing is interpolated at all, and only over it does the lens
/// trail, for as long as it takes to catch up.
pub fn aimed(lens: f64, pointer: f64, step: Duration) -> f64 {
    let reach = AIM_SPEED * step.as_secs_f64();
    let gap = pointer - lens;
    if gap.abs() <= reach {
        pointer
    } else {
        lens + reach.copysign(gap)
    }
}

/// Where to pretend an animation started, so reversing it does not jump.
///
/// Leaving halfway through the opening should close from halfway, not from
/// wide open. Plank does this by moving the start time forward; so does this.
///
/// It lands exactly, rather than nearly: `ease_out` and `ease_in` are each
/// other's mirror, so a reversal at `t` resumes at `span - t` on the other
/// curve at the very position it was at. The time left shrinks with the
/// distance left, which is why reversing a slide that barely started is over
/// almost at once.
pub fn reversed_start(elapsed: Duration, span: Duration) -> Duration {
    span.saturating_sub(elapsed.min(span))
}

/// How far down the bar has slid, `elapsed` after it was told where to go.
///
/// `1.0` is on screen and `0.0` is tucked away. Showing eases out and hiding
/// eases in — the same mirrored pair the lens uses, so `reversed_start` can
/// turn one into the other mid-flight.
pub fn slide_progress(elapsed: Duration, showing: bool) -> f64 {
    if elapsed >= SLIDE {
        return if showing { 1.0 } else { 0.0 };
    }
    let t = elapsed.as_secs_f64() / SLIDE.as_secs_f64();
    if showing {
        ease_out(t)
    } else {
        1.0 - ease_in(t)
    }
}

/// Where the bar sits, `progress` of the way onto the screen.
///
/// `progress` is already eased: it comes from `slide_progress`, which knows
/// which way the bar is going.
pub fn slide_y(progress: f64, shown_y: i32, hidden_y: i32) -> i32 {
    let travel = (hidden_y - shown_y) as f64;
    shown_y + (travel * (1.0 - progress.clamp(0.0, 1.0))).round() as i32
}

pub fn hidden_y(shown_y: i32, height: i32) -> i32 {
    shown_y + height - PEEK
}

pub fn launch_scale(elapsed: Duration) -> f64 {
    if elapsed >= LAUNCH {
        return 1.0;
    }
    let t = elapsed.as_secs_f64() / LAUNCH.as_secs_f64();
    let pulse = (t * std::f64::consts::PI * LAUNCH_PULSES).sin().abs();
    1.0 + LAUNCH_SWELL * pulse * (1.0 - t)
}

pub fn is_launching(elapsed: Duration) -> bool {
    elapsed < LAUNCH
}

pub fn is_launching_since(started: std::time::Instant) -> bool {
    is_launching(started.elapsed())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHOWN: i32 = 900;
    const HEIGHT: i32 = 84;

    #[test]
    fn a_hidden_bar_leaves_a_sliver_on_screen_to_be_pointed_at() {
        let hidden = hidden_y(SHOWN, HEIGHT);

        assert_eq!(hidden, SHOWN + HEIGHT - PEEK);
        assert!(PEEK > 0, "a bar with nothing on screen can never be summoned");
    }

    #[test]
    fn a_slide_starts_where_it_was_and_ends_where_it_was_going() {
        let hidden = hidden_y(SHOWN, HEIGHT);

        assert_eq!(slide_y(0.0, SHOWN, hidden), hidden);
        assert_eq!(slide_y(1.0, SHOWN, hidden), SHOWN);
    }

    #[test]
    fn a_slide_only_ever_moves_towards_where_it_is_going() {
        let hidden = hidden_y(SHOWN, HEIGHT);
        let mut previous = slide_y(0.0, SHOWN, hidden);

        for step in 1..=100 {
            let current = slide_y(step as f64 / 100.0, SHOWN, hidden);
            assert!(current <= previous, "the bar jumped backwards at {step}");
            previous = current;
        }
    }

    #[test]
    fn a_slide_never_leaves_the_span_between_its_two_ends() {
        let hidden = hidden_y(SHOWN, HEIGHT);

        for step in 0..=100 {
            let y = slide_y(step as f64 / 100.0, SHOWN, hidden);
            assert!((SHOWN..=hidden).contains(&y), "the bar left the screen at {step}");
        }
    }

    #[test]
    fn progress_outside_the_slide_is_clamped_rather_than_extrapolated() {
        let hidden = hidden_y(SHOWN, HEIGHT);

        assert_eq!(slide_y(-1.0, SHOWN, hidden), hidden);
        assert_eq!(slide_y(5.0, SHOWN, hidden), SHOWN);
    }

    #[test]
    fn a_lens_opens_from_shut_and_ends_wide_open() {
        assert_eq!(zoom_progress(Duration::ZERO, true), 0.0);
        assert_eq!(zoom_progress(ZOOM, true), 1.0);
        assert_eq!(zoom_progress(ZOOM * 2, true), 1.0);
    }

    #[test]
    fn a_lens_closes_from_open_and_ends_shut() {
        assert_eq!(zoom_progress(Duration::ZERO, false), 1.0);
        assert_eq!(zoom_progress(ZOOM, false), 0.0);
        assert_eq!(zoom_progress(ZOOM * 2, false), 0.0);
    }

    #[test]
    fn a_lens_only_ever_moves_towards_where_it_is_going() {
        for opening in [true, false] {
            let mut previous = zoom_progress(Duration::ZERO, opening);
            for step in 1..=100 {
                let current = zoom_progress(ZOOM * step / 100, opening);
                if opening {
                    assert!(current >= previous, "the lens shut while opening at {step}");
                } else {
                    assert!(current <= previous, "the lens opened while shutting at {step}");
                }
                previous = current;
            }
        }
    }

    #[test]
    fn leaving_halfway_through_closes_from_halfway() {
        let halfway = ZOOM / 2;
        let open_to = zoom_progress(halfway, true);

        let closing_from = zoom_progress(reversed_start(halfway, ZOOM), false);

        assert!(
            (closing_from - open_to).abs() < 0.2,
            "the lens jumped from {open_to:.2} to {closing_from:.2} on reversing"
        );
    }

    #[test]
    fn leaving_after_it_finished_closes_from_wide_open() {
        assert_eq!(reversed_start(ZOOM, ZOOM), Duration::ZERO);
        assert_eq!(reversed_start(ZOOM * 3, ZOOM), Duration::ZERO);
    }

    #[test]
    fn a_slide_is_tucked_away_at_the_start_and_on_screen_at_the_end() {
        assert_eq!(slide_progress(Duration::ZERO, true), 0.0);
        assert_eq!(slide_progress(SLIDE, true), 1.0);
        assert_eq!(slide_progress(SLIDE * 2, true), 1.0);

        assert_eq!(slide_progress(Duration::ZERO, false), 1.0);
        assert_eq!(slide_progress(SLIDE, false), 0.0);
        assert_eq!(slide_progress(SLIDE * 2, false), 0.0);
    }

    #[test]
    fn a_slide_only_ever_moves_towards_where_it_was_sent() {
        for showing in [true, false] {
            let mut previous = slide_progress(Duration::ZERO, showing);
            for step in 1..=100 {
                let current = slide_progress(SLIDE * step / 100, showing);
                if showing {
                    assert!(current >= previous, "the bar fell back while rising at {step}");
                } else {
                    assert!(current <= previous, "the bar rose while falling at {step}");
                }
                previous = current;
            }
        }
    }

    /// The reason this issue existed: the pointer arrives mid-hide and the bar
    /// has to come back from wherever it is, without a jump.
    #[test]
    fn turning_a_slide_round_never_jumps_further_than_one_frame_of_travel() {
        let hidden = hidden_y(SHOWN, HEIGHT);
        let travel = (hidden - SHOWN) as f64;
        let one_frame = travel * (FRAME.as_secs_f64() / SLIDE.as_secs_f64());

        for step in 0..=100 {
            let at = SLIDE * step / 100;
            let before = slide_y(slide_progress(at, false), SHOWN, hidden);
            let after = slide_y(
                slide_progress(reversed_start(at, SLIDE), true),
                SHOWN,
                hidden,
            );

            assert!(
                (before - after).abs() as f64 <= one_frame.ceil(),
                "turning round at {step}% jumped {} px, more than the {one_frame:.1} px a frame moves",
                (before - after).abs()
            );
        }
    }

    #[test]
    fn turning_round_early_is_over_early() {
        let tenth = SLIDE / 10;

        let left = SLIDE - reversed_start(tenth, SLIDE);

        assert_eq!(
            left, tenth,
            "reversing a tenth of the way in should cost a tenth of the time, not all of it"
        );
    }

    #[test]
    fn a_slide_that_finished_turns_round_from_the_very_end() {
        assert_eq!(reversed_start(SLIDE, SLIDE), Duration::ZERO);
        assert_eq!(reversed_start(SLIDE * 4, SLIDE), Duration::ZERO);
    }

    #[test]
    fn easing_slows_down_at_the_end_rather_than_stopping_dead() {
        let early = ease_out(0.1) - ease_out(0.0);
        let late = ease_out(1.0) - ease_out(0.9);

        assert!(early > late, "the slide should decelerate, not accelerate");
    }

    /// The whole point: a pointer moving at any speed a hand manages is
    /// followed exactly, with nothing interpolated and nothing lagging.
    #[test]
    fn a_pointer_that_moved_is_followed_exactly() {
        // A fast sweep, in the step it covers between two frames.
        let step = 2000.0 * FRAME.as_secs_f64();

        assert_eq!(aimed(400.0, 400.0 + step, FRAME), 400.0 + step);
        assert_eq!(aimed(400.0, 400.0 - step, FRAME), 400.0 - step);
    }

    #[test]
    fn a_pointer_that_jumped_is_followed_no_faster_than_the_ceiling() {
        let ceiling = AIM_SPEED * FRAME.as_secs_f64();

        let after = aimed(0.0, 1200.0, FRAME);

        assert!(
            (after - ceiling).abs() < 1e-9,
            "the lens covered {after:.1}px in a frame, past the {ceiling:.1}px ceiling"
        );
    }

    #[test]
    fn a_lens_catching_up_always_arrives() {
        let mut lens = 0.0;
        for frame in 1..=600 {
            lens = aimed(lens, 1200.0, FRAME);
            if lens == 1200.0 {
                assert!(frame < 30, "catching up took {frame} frames");
                return;
            }
        }
        panic!("the lens never reached the pointer, stopping at {lens}");
    }

    #[test]
    fn a_lens_catching_up_never_overshoots_or_turns_back() {
        let mut lens = 0.0;
        let mut previous = lens;
        for _ in 0..60 {
            lens = aimed(lens, 900.0, FRAME);
            assert!(lens >= previous, "the lens went backwards");
            assert!(lens <= 900.0, "the lens overshot to {lens}");
            previous = lens;
        }
    }

    /// Read off the clock, not counted in frames, so a 120Hz screen takes the
    /// same time to catch up as a 60Hz one — in twice as many, smaller steps.
    #[test]
    fn catching_up_takes_the_same_time_however_often_it_is_asked() {
        let half = FRAME / 2;

        let once = aimed(0.0, 1200.0, FRAME);
        let twice = aimed(aimed(0.0, 1200.0, half), 1200.0, half);

        assert!((once - twice).abs() < 1e-9, "{once} against {twice}");
    }

    #[test]
    fn a_launching_icon_starts_and_ends_at_its_own_size() {
        assert_eq!(launch_scale(Duration::ZERO), 1.0);
        assert_eq!(launch_scale(LAUNCH), 1.0);
        assert_eq!(launch_scale(LAUNCH * 2), 1.0);
    }

    #[test]
    fn a_launching_icon_swells_rather_than_shrinking() {
        let swollen = launch_scale(LAUNCH / 4);

        assert!(swollen > 1.0, "a launch should grow the icon, not shrink it");
    }

    #[test]
    fn a_launching_icon_never_grows_past_the_swell_it_was_given() {
        for step in 0..=100 {
            let scale = launch_scale(LAUNCH * step / 100);
            assert!(
                (1.0..=1.0 + LAUNCH_SWELL).contains(&scale),
                "scale {scale} escaped at step {step}"
            );
        }
    }

    #[test]
    fn each_pulse_of_a_launch_is_smaller_than_the_one_before() {
        let first = (0..40)
            .map(|s| launch_scale(LAUNCH * s / 100))
            .fold(0.0f64, f64::max);
        let last = (60..100)
            .map(|s| launch_scale(LAUNCH * s / 100))
            .fold(0.0f64, f64::max);

        assert!(last < first, "the pulse should die down, not repeat forever");
    }

    #[test]
    fn a_launch_animation_is_over_once_its_time_is_up() {
        assert!(is_launching(LAUNCH / 2));
        assert!(!is_launching(LAUNCH));
    }
}
