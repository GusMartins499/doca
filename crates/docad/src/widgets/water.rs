use std::time::Duration;

use doca_ipc::WidgetState;

use crate::config::{WaterSettings, WidgetSettings};

use super::Widget;

pub struct Water {
    goal: u32,
    drunk: u32,
}

impl Water {
    pub fn new(settings: WaterSettings) -> Self {
        Self {
            goal: settings.goal.max(1),
            drunk: 0,
        }
    }

    #[cfg(test)]
    pub fn drunk(&self) -> u32 {
        self.drunk
    }

    #[cfg(test)]
    pub fn goal(&self) -> u32 {
        self.goal
    }

    pub fn drink(&mut self) {
        self.drunk = self.drunk.saturating_add(1);
    }

    pub fn undo(&mut self) {
        self.drunk = self.drunk.saturating_sub(1);
    }

    pub fn reset(&mut self) {
        self.drunk = 0;
    }

    pub fn state(&self) -> WidgetState {
        WidgetState {
            id: "water".to_string(),
            label: format!("{}/{}", self.drunk, self.goal),
            detail: if self.drunk >= self.goal {
                "done for today".to_string()
            } else {
                "glasses".to_string()
            },
            progress: (self.drunk as f64 / self.goal as f64).clamp(0.0, 1.0),
            active: self.drunk >= self.goal,
        }
    }
}

impl Widget for Water {
    fn id(&self) -> &str {
        "water"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(60)
    }

    fn poll(&mut self) -> WidgetState {
        self.state()
    }

    /// A new goal, and the glasses already counted today.
    ///
    /// Resetting `drunk` here would be a settings change that quietly undoes
    /// the user's afternoon.
    fn adopt(&mut self, settings: &WidgetSettings) {
        self.goal = settings.water.goal.max(1);
    }

    fn invoke(&mut self, action: &str) {
        match action {
            "toggle" | "drink" => self.drink(),
            "undo" => self.undo(),
            "reset" => self.reset(),
            unknown => tracing::warn!("water ignoring unknown action {unknown}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn water(goal: u32) -> Water {
        Water::new(WaterSettings { goal })
    }

    fn goal_of(goal: u32) -> WidgetSettings {
        WidgetSettings {
            water: WaterSettings { goal },
            ..WidgetSettings::default()
        }
    }

    #[test]
    fn a_new_goal_does_not_forget_what_was_already_drunk() {
        let mut water = water(8);
        water.drink();
        water.drink();

        water.adopt(&goal_of(4));

        assert_eq!(water.state().label, "2/4");
    }

    #[test]
    fn a_goal_the_day_has_already_passed_counts_as_done() {
        let mut water = water(8);
        for _ in 0..3 {
            water.drink();
        }

        water.adopt(&goal_of(2));

        assert!(water.state().active);
    }

    #[test]
    fn the_day_starts_with_an_empty_count() {
        assert_eq!(water(8).state().label, "0/8");
    }

    #[test]
    fn a_goal_of_zero_is_treated_as_one_so_progress_means_something() {
        assert_eq!(water(0).goal(), 1);
    }

    #[test]
    fn each_click_is_one_more_glass() {
        let mut water = water(8);

        water.drink();
        water.drink();

        assert_eq!(water.state().label, "2/8");
    }

    #[test]
    fn a_glass_counted_by_mistake_can_be_taken_back() {
        let mut water = water(8);
        water.drink();

        water.undo();

        assert_eq!(water.drunk(), 0);
    }

    #[test]
    fn undoing_at_zero_does_not_wrap_around_to_a_huge_number() {
        let mut water = water(8);

        water.undo();

        assert_eq!(water.drunk(), 0);
    }

    #[test]
    fn reaching_the_goal_is_marked_and_says_so() {
        let mut water = water(2);

        water.drink();
        water.drink();

        assert!(water.state().active);
        assert_eq!(water.state().detail, "done for today");
    }

    #[test]
    fn drinking_past_the_goal_keeps_counting_without_overflowing_the_bar() {
        let mut water = water(2);

        for _ in 0..10 {
            water.drink();
        }

        assert_eq!(water.state().label, "10/2");
        assert_eq!(water.state().progress, 1.0);
    }

    #[test]
    fn resetting_starts_the_day_over() {
        let mut water = water(8);
        water.drink();

        water.reset();

        assert_eq!(water.drunk(), 0);
    }
}
