use std::time::{Duration, Instant};

use primodock_ipc::{WidgetState, NO_PROGRESS};

use super::Widget;

pub fn format_elapsed(elapsed: Duration) -> String {
    let total = elapsed.as_secs();
    if total >= 3600 {
        format!("{}:{:02}:{:02}", total / 3600, (total % 3600) / 60, total % 60)
    } else {
        format!("{:02}:{:02}", total / 60, total % 60)
    }
}

pub struct Stopwatch {
    elapsed: Duration,
    running: bool,
    laps: Vec<Duration>,
    last_tick: Option<Instant>,
}

impl Stopwatch {
    pub fn new() -> Self {
        Self {
            elapsed: Duration::ZERO,
            running: false,
            laps: Vec::new(),
            last_tick: None,
        }
    }

    #[cfg(test)]
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    #[cfg(test)]
    pub fn laps(&self) -> &[Duration] {
        &self.laps
    }

    #[cfg(test)]
    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn toggle(&mut self) {
        self.running = !self.running;
    }

    pub fn reset(&mut self) {
        self.elapsed = Duration::ZERO;
        self.running = false;
        self.laps.clear();
    }

    pub fn lap(&mut self) {
        if self.running {
            self.laps.push(self.elapsed);
        }
    }

    pub fn advance(&mut self, by: Duration) {
        if self.running {
            self.elapsed += by;
        }
    }

    fn advance_to(&mut self, now: Instant) {
        let step = match self.last_tick {
            Some(previous) => now.saturating_duration_since(previous),
            None => Duration::ZERO,
        };
        self.last_tick = Some(now);
        self.advance(step);
    }

    pub fn state(&self) -> WidgetState {
        WidgetState {
            id: "stopwatch".to_string(),
            label: format_elapsed(self.elapsed),
            detail: match (self.running, self.laps.len()) {
                (true, 0) => "running".to_string(),
                (true, laps) => format!("lap {laps}"),
                (false, 0) => "stopped".to_string(),
                (false, laps) => format!("{laps} laps"),
            },
            progress: NO_PROGRESS,
            active: self.running,
        }
    }
}

impl Widget for Stopwatch {
    fn id(&self) -> &str {
        "stopwatch"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn poll(&mut self) -> WidgetState {
        self.advance_to(Instant::now());
        self.state()
    }

    fn invoke(&mut self, action: &str) {
        match action {
            "toggle" => self.toggle(),
            "reset" => self.reset(),
            "lap" => self.lap(),
            unknown => tracing::warn!("stopwatch ignoring unknown action {unknown}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(watch: &mut Stopwatch, seconds: u64) {
        for _ in 0..seconds {
            watch.advance(Duration::from_secs(1));
        }
    }

    #[test]
    fn a_fresh_stopwatch_sits_at_zero_and_stopped() {
        let watch = Stopwatch::new();

        assert_eq!(watch.state().label, "00:00");
        assert!(!watch.is_running());
    }

    #[test]
    fn a_stopped_stopwatch_does_not_move() {
        let mut watch = Stopwatch::new();

        run(&mut watch, 60);

        assert_eq!(watch.elapsed(), Duration::ZERO);
    }

    #[test]
    fn minutes_give_way_to_hours_only_once_an_hour_has_passed() {
        assert_eq!(format_elapsed(Duration::from_secs(59)), "00:59");
        assert_eq!(format_elapsed(Duration::from_secs(3599)), "59:59");
        assert_eq!(format_elapsed(Duration::from_secs(3600)), "1:00:00");
        assert_eq!(format_elapsed(Duration::from_secs(3661)), "1:01:01");
    }

    #[test]
    fn pausing_keeps_the_time_already_counted() {
        let mut watch = Stopwatch::new();
        watch.toggle();
        run(&mut watch, 30);

        watch.toggle();
        run(&mut watch, 30);

        assert_eq!(watch.elapsed(), Duration::from_secs(30));
    }

    #[test]
    fn a_lap_records_the_moment_it_was_taken() {
        let mut watch = Stopwatch::new();
        watch.toggle();
        run(&mut watch, 10);
        watch.lap();
        run(&mut watch, 5);
        watch.lap();

        assert_eq!(
            watch.laps(),
            &[Duration::from_secs(10), Duration::from_secs(15)]
        );
    }

    #[test]
    fn a_lap_on_a_stopped_stopwatch_records_nothing() {
        let mut watch = Stopwatch::new();

        watch.lap();

        assert!(watch.laps().is_empty());
    }

    #[test]
    fn resetting_clears_the_time_and_the_laps() {
        let mut watch = Stopwatch::new();
        watch.toggle();
        run(&mut watch, 10);
        watch.lap();

        watch.reset();

        assert_eq!(watch.elapsed(), Duration::ZERO);
        assert!(watch.laps().is_empty());
        assert!(!watch.is_running());
    }

    #[test]
    fn reading_the_state_does_not_advance_the_stopwatch() {
        let mut watch = Stopwatch::new();
        watch.toggle();
        run(&mut watch, 5);

        for _ in 0..10 {
            let _ = watch.state();
        }

        assert_eq!(watch.elapsed(), Duration::from_secs(5));
    }

    #[test]
    fn the_stopwatch_follows_wall_clock_time_not_the_number_of_polls() {
        let mut watch = Stopwatch::new();
        watch.toggle();
        let start = Instant::now();

        watch.advance_to(start);
        watch.advance_to(start + Duration::from_secs(42));

        assert_eq!(watch.elapsed(), Duration::from_secs(42));
    }
}
