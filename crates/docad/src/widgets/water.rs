use std::time::Duration;

use doca_ipc::{Body, WidgetState};

use crate::config::{WaterSettings, WidgetSettings};
use crate::state::{State, WaterState};

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

    /// The count and the goal, and nothing worked out from them.
    ///
    /// The first widget with a variant of its own, and the reason it is this
    /// one: a label of `"3/8"` is a sentence the bar cannot draw a ring from,
    /// and the share of the goal is the shape of the drawing. What the tile
    /// says in words, and in what colour, is the bar's to decide — the daemon
    /// only knows how many glasses there were.
    pub fn state(&self) -> WidgetState {
        WidgetState::new(
            "water",
            Body::Water(doca_ipc::Water {
                glasses: self.drunk,
                goal: self.goal,
            }),
        )
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

    fn actions(&self) -> &'static [&'static str] {
        &["toggle", "drink", "undo", "reset"]
    }

    /// The glasses the day had already counted when the daemon came up — or,
    /// at midnight, the nothing a new day starts with.
    ///
    /// No date is read here. Whether this state belongs to today was settled
    /// before it arrived, in `State::roll_to`.
    fn adopt_state(&mut self, state: &State) {
        self.drunk = state.water.glasses;
    }

    fn remember(&self, state: &mut State) {
        state.water = WaterState {
            glasses: self.drunk,
        };
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

    pub fn water(goal: u32) -> Water {
        Water::new(WaterSettings { goal })
    }

    fn goal_of(goal: u32) -> WidgetSettings {
        WidgetSettings {
            water: WaterSettings { goal },
            ..WidgetSettings::default()
        }
    }

    /// What the tile is told, as the typed body rather than as a sentence.
    fn body_of(water: &Water) -> doca_ipc::Water {
        match water.state().body().expect("a water body reads back") {
            Body::Water(body) => body,
            other => panic!("the water widget sent a {other:?}"),
        }
    }

    #[test]
    fn a_new_goal_does_not_forget_what_was_already_drunk() {
        let mut water = water(8);
        water.drink();
        water.drink();

        water.adopt(&goal_of(4));

        assert_eq!(body_of(&water), doca_ipc::Water { glasses: 2, goal: 4 });
    }

    #[test]
    fn a_goal_the_day_has_already_passed_counts_as_done() {
        let mut water = water(8);
        for _ in 0..3 {
            water.drink();
        }

        water.adopt(&goal_of(2));

        let body = body_of(&water);
        assert!(body.glasses >= body.goal);
    }

    #[test]
    fn the_day_starts_with_an_empty_count() {
        assert_eq!(body_of(&water(8)), doca_ipc::Water { glasses: 0, goal: 8 });
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

        assert_eq!(body_of(&water).glasses, 2);
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
    fn reaching_the_goal_is_marked() {
        let mut water = water(2);

        water.drink();
        water.drink();

        assert_eq!(body_of(&water), doca_ipc::Water { glasses: 2, goal: 2 });
    }

    /// Past the goal the count keeps going, and the body says so plainly —
    /// it is the bar that decides not to draw a ring past full.
    #[test]
    fn drinking_past_the_goal_keeps_counting() {
        let mut water = water(2);

        for _ in 0..10 {
            water.drink();
        }

        assert_eq!(body_of(&water), doca_ipc::Water { glasses: 10, goal: 2 });
    }

    #[test]
    fn resetting_starts_the_day_over() {
        let mut water = water(8);
        water.drink();

        water.reset();

        assert_eq!(water.drunk(), 0);
    }
}

#[cfg(test)]
mod remembering {
    use super::tests::water;
    use super::*;

    #[test]
    fn the_glasses_counted_today_are_what_gets_written_down() {
        let mut water = water(8);
        water.drink();
        water.drink();
        water.drink();

        let mut state = State::default();
        water.remember(&mut state);

        assert_eq!(state.water.glasses, 3);
    }

    #[test]
    fn a_widget_built_today_starts_from_what_the_morning_left() {
        let mut water = water(8);

        water.adopt_state(&State {
            day: 20_735,
            water: WaterState { glasses: 5 },
        });

        assert_eq!(water.drunk(), 5);
    }

    /// Midnight: the state handed over has already been emptied, so the
    /// widget has nothing to decide.
    #[test]
    fn a_new_day_leaves_the_widget_at_zero() {
        let mut water = water(8);
        water.drink();

        water.adopt_state(&State::default());

        assert_eq!(water.drunk(), 0);
    }

    /// Every action a panel will draw a button for is one the widget takes.
    #[test]
    fn the_actions_offered_are_the_actions_answered() {
        let water = water(8);

        for (action, _) in doca_ipc::widget_action::of("water") {
            assert!(
                water.actions().contains(&action),
                "water is offered {action} and does not answer to it"
            );
        }
    }
}
