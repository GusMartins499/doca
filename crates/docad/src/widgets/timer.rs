use std::time::{Duration, Instant};

use doca_ipc::{Body, Simple, WidgetState};

use crate::config::{TimerSettings, WidgetSettings};

use super::Widget;

pub struct Timer {
    full: Duration,
    remaining: Duration,
    running: bool,
    rang: bool,
    last_tick: Option<Instant>,
}

impl Timer {
    pub fn new(settings: TimerSettings) -> Self {
        let full = Duration::from_secs(settings.minutes.max(1) as u64 * 60);
        Self {
            full,
            remaining: full,
            running: false,
            rang: false,
            last_tick: None,
        }
    }

    #[cfg(test)]
    pub fn remaining(&self) -> Duration {
        self.remaining
    }

    #[cfg(test)]
    pub fn has_rung(&self) -> bool {
        self.rang
    }

    pub fn toggle(&mut self) {
        if self.rang {
            self.reset();
            return;
        }
        self.running = !self.running;
    }

    pub fn reset(&mut self) {
        self.remaining = self.full;
        self.running = false;
        self.rang = false;
    }

    pub fn add(&mut self, extra: Duration) {
        self.remaining = (self.remaining + extra).min(Duration::from_secs(24 * 3600));
        self.full = self.full.max(self.remaining);
        self.rang = false;
    }

    pub fn tick(&mut self, elapsed: Duration) {
        if !self.running {
            return;
        }
        if self.remaining > elapsed {
            self.remaining -= elapsed;
            return;
        }
        self.remaining = Duration::ZERO;
        self.running = false;
        self.rang = true;
    }

    fn advance_to(&mut self, now: Instant) {
        let step = match self.last_tick {
            Some(previous) => now.saturating_duration_since(previous),
            None => Duration::ZERO,
        };
        self.last_tick = Some(now);
        self.tick(step);
    }

    pub fn state(&self) -> WidgetState {
        let seconds = self.remaining.as_secs();
        let elapsed = self.full.as_secs().saturating_sub(seconds);
        WidgetState::new(
            "timer",
            Body::Simple(Simple {
                label: format!("{:02}:{:02}", seconds / 60, seconds % 60),
                detail: if self.rang {
                    "done".to_string()
                } else if self.running {
                    "running".to_string()
                } else {
                    "paused".to_string()
                },
                progress: elapsed as f64 / self.full.as_secs().max(1) as f64,
                active: self.rang || self.running,
            }),
        )
    }
}

impl Widget for Timer {
    fn actions(&self) -> &'static [&'static str] {
        &["toggle", "reset", "add"]
    }

    fn id(&self) -> &str {
        "timer"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn poll(&mut self) -> WidgetState {
        self.advance_to(Instant::now());
        self.state()
    }

    /// A new length, without yanking a countdown already under way.
    ///
    /// A timer sitting at its full length has not started, so it simply
    /// becomes the new length — which is what someone who just typed 15 in a
    /// settings window is asking for. One that is running, paused part-way or
    /// has rung keeps the number on screen and takes the new length at its
    /// next reset: changing the setting is not the same as pressing reset,
    /// and silently doing both would lose a count nobody asked to lose.
    fn adopt(&mut self, settings: &WidgetSettings) {
        let full = Duration::from_secs(settings.timer.minutes.max(1) as u64 * 60);
        let idle = !self.running && !self.rang && self.remaining == self.full;
        self.full = full;
        if idle {
            self.remaining = full;
        }
    }

    fn invoke(&mut self, action: &str) {
        match action {
            "toggle" => self.toggle(),
            "reset" => self.reset(),
            "add" => self.add(Duration::from_secs(60)),
            unknown => tracing::warn!("timer ignoring unknown action {unknown}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::Drawn;

    fn timer(minutes: u32) -> Timer {
        Timer::new(TimerSettings { minutes })
    }

    fn run(timer: &mut Timer, seconds: u64) {
        for _ in 0..seconds {
            timer.tick(Duration::from_secs(1));
        }
    }

    fn minutes(minutes: u32) -> WidgetSettings {
        WidgetSettings {
            timer: TimerSettings { minutes },
            ..WidgetSettings::default()
        }
    }

    #[test]
    fn a_new_length_is_shown_at_once_by_a_timer_that_never_started() {
        let mut timer = timer(10);

        timer.adopt(&minutes(15));

        assert_eq!(timer.remaining(), Duration::from_secs(15 * 60));
        assert_eq!(timer.state().label(), "15:00");
    }

    #[test]
    fn a_running_timer_keeps_counting_the_count_it_was_already_on() {
        let mut timer = timer(10);
        timer.toggle();
        run(&mut timer, 30);

        timer.adopt(&minutes(15));

        assert_eq!(timer.remaining(), Duration::from_secs(9 * 60 + 30));
        assert_eq!(timer.state().detail(), "running");
    }

    #[test]
    fn a_timer_paused_part_way_is_not_moved_either() {
        let mut timer = timer(10);
        timer.toggle();
        run(&mut timer, 60);
        timer.toggle();

        timer.adopt(&minutes(2));

        assert_eq!(timer.remaining(), Duration::from_secs(9 * 60));
    }

    #[test]
    fn the_new_length_is_what_a_reset_goes_back_to() {
        let mut timer = timer(10);
        timer.toggle();
        run(&mut timer, 60);

        timer.adopt(&minutes(5));
        timer.reset();

        assert_eq!(timer.remaining(), Duration::from_secs(5 * 60));
    }

    #[test]
    fn a_new_timer_shows_its_configured_length_and_waits() {
        let timer = timer(10);

        assert_eq!(timer.state().label(), "10:00");
        assert_eq!(timer.state().detail(), "paused");
    }

    #[test]
    fn a_timer_configured_at_zero_minutes_still_has_a_length() {
        assert_eq!(timer(0).remaining(), Duration::from_secs(60));
    }

    #[test]
    fn a_running_timer_counts_down() {
        let mut timer = timer(10);
        timer.toggle();

        run(&mut timer, 90);

        assert_eq!(timer.state().label(), "08:30");
    }

    #[test]
    fn a_timer_that_reaches_zero_stops_there_and_says_it_is_done() {
        let mut timer = timer(1);
        timer.toggle();

        run(&mut timer, 120);

        assert_eq!(timer.remaining(), Duration::ZERO);
        assert!(timer.has_rung());
        assert_eq!(timer.state().detail(), "done");
    }

    #[test]
    fn a_timer_that_has_rung_is_reset_by_the_next_click_rather_than_resuming() {
        let mut timer = timer(1);
        timer.toggle();
        run(&mut timer, 120);

        timer.toggle();

        assert_eq!(timer.remaining(), Duration::from_secs(60));
        assert!(!timer.has_rung());
    }

    #[test]
    fn adding_a_minute_extends_the_ring_without_breaking_the_progress_bar() {
        let mut timer = timer(5);
        timer.toggle();
        run(&mut timer, 60);

        timer.add(Duration::from_secs(60));

        assert_eq!(timer.remaining(), Duration::from_secs(5 * 60));
        assert!((0.0..=1.0).contains(&timer.state().progress()));
    }

    #[test]
    fn a_timer_cannot_be_extended_past_a_day() {
        let mut timer = timer(10);

        for _ in 0..2000 {
            timer.add(Duration::from_secs(60));
        }

        assert!(timer.remaining() <= Duration::from_secs(24 * 3600));
    }

    #[test]
    fn reading_the_state_does_not_advance_the_timer() {
        let mut timer = timer(10);
        timer.toggle();
        run(&mut timer, 5);

        for _ in 0..10 {
            let _ = timer.state();
        }

        assert_eq!(timer.remaining(), Duration::from_secs(10 * 60 - 5));
    }

    #[test]
    fn progress_stays_within_range_the_whole_way_down() {
        let mut timer = timer(2);
        timer.toggle();

        for _ in 0..200 {
            timer.tick(Duration::from_secs(1));
            let progress = timer.state().progress();
            assert!((0.0..=1.0).contains(&progress), "progress escaped: {progress}");
        }
    }
}
