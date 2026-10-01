use std::time::{Duration, SystemTime, UNIX_EPOCH};

use doca_ipc::{WidgetState, NO_PROGRESS};

use crate::config::CountdownSettings;

use super::clock::{days_from_civil, local_offset_seconds, parse_date};
use super::Widget;

pub fn days_until(target: (i64, u32, u32), today: i64) -> i64 {
    days_from_civil(target.0, target.1, target.2) - today
}

pub fn label_for(days: Option<i64>) -> String {
    match days {
        None => "—".to_string(),
        Some(0) => "today".to_string(),
        Some(1) => "1 day".to_string(),
        Some(days) if days > 0 => format!("{days} days"),
        Some(-1) => "yesterday".to_string(),
        Some(days) => format!("{} days ago", -days),
    }
}

pub struct Countdown {
    settings: CountdownSettings,
    offset: i64,
}

impl Countdown {
    pub fn new(settings: CountdownSettings) -> Self {
        Self {
            settings,
            offset: local_offset_seconds(),
        }
    }
}

impl Widget for Countdown {
    fn id(&self) -> &str {
        "countdown"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(60)
    }

    fn poll(&mut self) -> WidgetState {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
            + self.offset;
        let today = now.div_euclid(86_400);
        let days = parse_date(&self.settings.date).map(|target| days_until(target, today));

        WidgetState {
            id: "countdown".to_string(),
            label: label_for(days),
            detail: if days.is_none() && !self.settings.date.trim().is_empty() {
                "bad date".to_string()
            } else {
                self.settings.label.clone()
            },
            progress: NO_PROGRESS,
            active: days == Some(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> i64 {
        days_from_civil(2026, 6, 15)
    }

    #[test]
    fn a_date_in_the_future_counts_the_days_to_it() {
        assert_eq!(days_until((2026, 6, 25), today()), 10);
    }

    #[test]
    fn a_date_in_the_past_counts_negative() {
        assert_eq!(days_until((2026, 6, 5), today()), -10);
    }

    #[test]
    fn a_leap_day_is_counted_along_the_way() {
        let before = days_from_civil(2024, 2, 28);

        assert_eq!(days_until((2024, 3, 1), before), 2);
    }

    #[test]
    fn today_reads_as_today_rather_than_zero_days() {
        assert_eq!(label_for(Some(0)), "today");
    }

    #[test]
    fn one_day_is_singular_and_two_are_plural() {
        assert_eq!(label_for(Some(1)), "1 day");
        assert_eq!(label_for(Some(2)), "2 days");
    }

    #[test]
    fn a_date_already_past_says_so_instead_of_showing_a_minus_sign() {
        assert_eq!(label_for(Some(-1)), "yesterday");
        assert_eq!(label_for(Some(-5)), "5 days ago");
    }

    #[test]
    fn no_date_configured_shows_a_dash_rather_than_a_wrong_number() {
        assert_eq!(label_for(None), "—");
    }

    #[test]
    fn a_date_that_cannot_be_parsed_is_reported_as_such() {
        let mut widget = Countdown::new(CountdownSettings {
            date: "next tuesday".to_string(),
            label: "whatever".to_string(),
        });

        let state = widget.poll();

        assert_eq!(state.label, "—");
        assert_eq!(state.detail, "bad date");
    }

    #[test]
    fn an_empty_date_is_not_treated_as_a_mistake() {
        let mut widget = Countdown::new(CountdownSettings::default());

        assert_ne!(widget.poll().detail, "bad date");
    }
}
