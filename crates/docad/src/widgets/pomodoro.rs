use std::time::{Duration, Instant};

use doca_ipc::{Body, Simple, WidgetState};

use super::Widget;

pub const FOCUS: Duration = Duration::from_secs(25 * 60);
pub const SHORT_BREAK: Duration = Duration::from_secs(5 * 60);
pub const LONG_BREAK: Duration = Duration::from_secs(15 * 60);
pub const CYCLES_BEFORE_LONG_BREAK: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Focus,
    ShortBreak,
    LongBreak,
}

impl Phase {
    pub fn duration(self) -> Duration {
        match self {
            Phase::Focus => FOCUS,
            Phase::ShortBreak => SHORT_BREAK,
            Phase::LongBreak => LONG_BREAK,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Phase::Focus => "focus",
            Phase::ShortBreak => "break",
            Phase::LongBreak => "long break",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Pomodoro {
    phase: Phase,
    remaining: Duration,
    running: bool,
    completed_focus: u32,
    last_tick: Option<Instant>,
}

impl Pomodoro {
    pub fn new() -> Self {
        Self {
            phase: Phase::Focus,
            remaining: FOCUS,
            running: false,
            completed_focus: 0,
            last_tick: None,
        }
    }

    #[cfg(test)]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    #[cfg(test)]
    pub fn remaining(&self) -> Duration {
        self.remaining
    }

    #[cfg(test)]
    pub fn is_running(&self) -> bool {
        self.running
    }

    #[cfg(test)]
    pub fn completed_focus(&self) -> u32 {
        self.completed_focus
    }

    pub fn start(&mut self) {
        self.running = true;
    }

    pub fn pause(&mut self) {
        self.running = false;
    }

    pub fn toggle(&mut self) {
        self.running = !self.running;
    }

    pub fn reset(&mut self) {
        self.phase = Phase::Focus;
        self.remaining = FOCUS;
        self.running = false;
        self.completed_focus = 0;
    }

    fn advance_to(&mut self, now: Instant) {
        let elapsed = match self.last_tick {
            Some(previous) => now.saturating_duration_since(previous),
            None => Duration::ZERO,
        };
        self.last_tick = Some(now);
        self.tick(elapsed);
    }

    pub fn skip(&mut self) {
        self.advance();
    }

    pub fn tick(&mut self, elapsed: Duration) {
        if !self.running {
            return;
        }
        if self.remaining > elapsed {
            self.remaining -= elapsed;
            return;
        }
        self.advance();
    }

    fn advance(&mut self) {
        self.phase = match self.phase {
            Phase::Focus => {
                self.completed_focus += 1;
                if self.completed_focus % CYCLES_BEFORE_LONG_BREAK == 0 {
                    Phase::LongBreak
                } else {
                    Phase::ShortBreak
                }
            }
            Phase::ShortBreak | Phase::LongBreak => Phase::Focus,
        };
        self.remaining = self.phase.duration();
    }

    pub fn state(&self) -> WidgetState {
        let seconds = self.remaining.as_secs();
        let elapsed = self.phase.duration().as_secs() - seconds;
        WidgetState::new(
            "pomodoro",
            Body::Simple(Simple {
                label: format!("{:02}:{:02}", seconds / 60, seconds % 60),
                detail: if self.running {
                    self.phase.name().to_string()
                } else {
                    format!("{} · paused", self.phase.name())
                },
                progress: elapsed as f64 / self.phase.duration().as_secs() as f64,
                active: self.running && self.phase == Phase::Focus,
            }),
        )
    }
}

impl Widget for Pomodoro {
    fn actions(&self) -> &'static [&'static str] {
        &["start", "pause", "toggle", "reset", "skip"]
    }

    fn id(&self) -> &str {
        "pomodoro"
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
            "start" => self.start(),
            "pause" => self.pause(),
            "toggle" => self.toggle(),
            "reset" => self.reset(),
            "skip" => self.skip(),
            unknown => tracing::warn!("pomodoro ignoring unknown action {unknown}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::Drawn;

    fn run(pomodoro: &mut Pomodoro, seconds: u64) {
        for _ in 0..seconds {
            pomodoro.tick(Duration::from_secs(1));
        }
    }

    #[test]
    fn a_fresh_pomodoro_is_paused_at_a_full_focus_block() {
        let pomodoro = Pomodoro::new();

        assert_eq!(pomodoro.phase(), Phase::Focus);
        assert_eq!(pomodoro.remaining(), FOCUS);
        assert!(!pomodoro.is_running());
        assert_eq!(pomodoro.state().label(), "25:00");
    }

    #[test]
    fn a_paused_pomodoro_does_not_lose_time() {
        let mut pomodoro = Pomodoro::new();

        run(&mut pomodoro, 120);

        assert_eq!(pomodoro.remaining(), FOCUS);
    }

    #[test]
    fn a_running_pomodoro_counts_down() {
        let mut pomodoro = Pomodoro::new();
        pomodoro.start();

        run(&mut pomodoro, 60);

        assert_eq!(pomodoro.state().label(), "24:00");
    }

    #[test]
    fn focus_is_followed_by_a_short_break() {
        let mut pomodoro = Pomodoro::new();
        pomodoro.start();

        run(&mut pomodoro, FOCUS.as_secs());

        assert_eq!(pomodoro.phase(), Phase::ShortBreak);
        assert_eq!(pomodoro.remaining(), SHORT_BREAK);
        assert_eq!(pomodoro.completed_focus(), 1);
    }

    #[test]
    fn the_fourth_focus_block_earns_a_long_break() {
        let mut pomodoro = Pomodoro::new();
        pomodoro.start();

        for _ in 0..CYCLES_BEFORE_LONG_BREAK {
            run(&mut pomodoro, FOCUS.as_secs());
            let rest = pomodoro.remaining().as_secs();
            run(&mut pomodoro, rest);
        }

        assert_eq!(pomodoro.completed_focus(), CYCLES_BEFORE_LONG_BREAK);
        assert_eq!(pomodoro.phase(), Phase::Focus);
    }

    #[test]
    fn the_break_after_the_fourth_focus_block_is_the_long_one() {
        let mut pomodoro = Pomodoro::new();
        pomodoro.start();

        for _ in 0..CYCLES_BEFORE_LONG_BREAK - 1 {
            run(&mut pomodoro, FOCUS.as_secs());
            let rest = pomodoro.remaining().as_secs();
            run(&mut pomodoro, rest);
        }
        run(&mut pomodoro, FOCUS.as_secs());

        assert_eq!(pomodoro.phase(), Phase::LongBreak);
    }

    #[test]
    fn resetting_returns_to_a_paused_full_focus_block_and_clears_the_count() {
        let mut pomodoro = Pomodoro::new();
        pomodoro.start();
        run(&mut pomodoro, FOCUS.as_secs() + 30);

        pomodoro.reset();

        assert_eq!(pomodoro.phase(), Phase::Focus);
        assert_eq!(pomodoro.remaining(), FOCUS);
        assert!(!pomodoro.is_running());
        assert_eq!(pomodoro.completed_focus(), 0);
    }

    #[test]
    fn skipping_moves_to_the_next_phase_without_waiting() {
        let mut pomodoro = Pomodoro::new();

        pomodoro.skip();

        assert_eq!(pomodoro.phase(), Phase::ShortBreak);
    }

    #[test]
    fn only_a_running_focus_block_makes_the_tile_active() {
        let mut pomodoro = Pomodoro::new();
        assert!(!pomodoro.state().active());

        pomodoro.start();
        assert!(pomodoro.state().active());

        pomodoro.skip();
        assert!(!pomodoro.state().active());
    }

    #[test]
    fn progress_runs_from_the_start_of_a_phase_to_its_end() {
        let mut pomodoro = Pomodoro::new();
        assert_eq!(pomodoro.state().progress(), 0.0);

        pomodoro.start();
        run(&mut pomodoro, FOCUS.as_secs() / 2);

        assert!((pomodoro.state().progress() - 0.5).abs() < 0.01);
    }

    #[test]
    fn reading_the_state_does_not_advance_the_timer() {
        let mut pomodoro = Pomodoro::new();
        pomodoro.start();
        run(&mut pomodoro, 10);

        let before = pomodoro.remaining();
        for _ in 0..5 {
            let _ = pomodoro.state();
        }

        assert_eq!(pomodoro.remaining(), before);
    }

    #[test]
    fn the_timer_follows_wall_clock_time_not_the_number_of_polls() {
        let mut pomodoro = Pomodoro::new();
        pomodoro.start();
        let start = Instant::now();

        pomodoro.advance_to(start);
        pomodoro.advance_to(start + Duration::from_secs(30));

        assert_eq!(pomodoro.remaining(), FOCUS - Duration::from_secs(30));
    }

    #[test]
    fn a_burst_of_polls_in_the_same_instant_costs_no_time() {
        let mut pomodoro = Pomodoro::new();
        pomodoro.start();
        let now = Instant::now();

        pomodoro.advance_to(now);
        let before = pomodoro.remaining();
        for _ in 0..10 {
            pomodoro.advance_to(now);
        }

        assert_eq!(pomodoro.remaining(), before);
    }

    #[test]
    fn a_paused_timer_does_not_jump_when_it_is_resumed() {
        let mut pomodoro = Pomodoro::new();
        let start = Instant::now();
        pomodoro.advance_to(start);

        pomodoro.advance_to(start + Duration::from_secs(60));
        pomodoro.start();
        pomodoro.advance_to(start + Duration::from_secs(61));

        assert_eq!(pomodoro.remaining(), FOCUS - Duration::from_secs(1));
    }

    #[test]
    fn toggle_starts_a_paused_timer_and_pauses_a_running_one() {
        let mut pomodoro = Pomodoro::new();

        pomodoro.toggle();
        assert!(pomodoro.is_running());

        pomodoro.toggle();
        assert!(!pomodoro.is_running());
    }
}
