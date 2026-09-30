use std::time::Duration;

pub const SLIDE: Duration = Duration::from_millis(160);
pub const FRAME: Duration = Duration::from_millis(16);
pub const PEEK: i32 = 2;
pub const LAUNCH: Duration = Duration::from_millis(700);
pub const LAUNCH_SWELL: f64 = 0.28;
pub const LAUNCH_PULSES: f64 = 2.0;

pub fn ease_out(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

pub fn slide_y(progress: f64, shown_y: i32, hidden_y: i32) -> i32 {
    let eased = ease_out(progress);
    let travel = (hidden_y - shown_y) as f64;
    shown_y + (travel * (1.0 - eased)).round() as i32
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

pub fn launch_size(elapsed: Duration, icon_size: i32) -> i32 {
    (icon_size as f64 * launch_scale(elapsed)).round() as i32
}

pub fn is_launching(elapsed: Duration) -> bool {
    elapsed < LAUNCH
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
    fn easing_slows_down_at_the_end_rather_than_stopping_dead() {
        let early = ease_out(0.1) - ease_out(0.0);
        let late = ease_out(1.0) - ease_out(0.9);

        assert!(early > late, "the slide should decelerate, not accelerate");
    }

    #[test]
    fn a_launching_icon_starts_and_ends_at_its_own_size() {
        assert_eq!(launch_scale(Duration::ZERO), 1.0);
        assert_eq!(launch_scale(LAUNCH), 1.0);
        assert_eq!(launch_scale(LAUNCH * 2), 1.0);
        assert_eq!(launch_size(LAUNCH, 48), 48);
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
