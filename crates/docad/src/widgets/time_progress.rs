use std::time::{Duration, SystemTime, UNIX_EPOCH};

use doca_ipc::{Body, Simple, WidgetState};

use super::clock::{civil_from_days, days_from_civil};
use super::Widget;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    Day,
    Week,
    Month,
    Year,
}

impl Span {
    pub fn next(self) -> Self {
        match self {
            Span::Day => Span::Week,
            Span::Week => Span::Month,
            Span::Month => Span::Year,
            Span::Year => Span::Day,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Span::Day => "of today",
            Span::Week => "of this week",
            Span::Month => "of this month",
            Span::Year => "of this year",
        }
    }
}

pub fn days_in_month(year: i64, month: u32) -> i64 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    days_from_civil(next_year, next_month, 1) - days_from_civil(year, month, 1)
}

pub fn days_in_year(year: i64) -> i64 {
    days_from_civil(year + 1, 1, 1) - days_from_civil(year, 1, 1)
}

pub fn fraction(span: Span, days_since_epoch: i64, second_of_day: i64) -> f64 {
    let day_fraction = second_of_day as f64 / 86_400.0;
    let (year, month, day) = civil_from_days(days_since_epoch);

    let (elapsed, total) = match span {
        Span::Day => (day_fraction, 1.0),
        Span::Week => {
            let weekday_from_monday = (days_since_epoch + 3).rem_euclid(7) as f64;
            (weekday_from_monday + day_fraction, 7.0)
        }
        Span::Month => (
            (day - 1) as f64 + day_fraction,
            days_in_month(year, month) as f64,
        ),
        Span::Year => {
            let into_year = days_since_epoch - days_from_civil(year, 1, 1);
            (into_year as f64 + day_fraction, days_in_year(year) as f64)
        }
    };

    (elapsed / total).clamp(0.0, 1.0)
}

pub struct TimeProgress {
    span: Span,
    offset: i64,
}

impl TimeProgress {
    pub fn new() -> Self {
        Self {
            span: Span::Day,
            offset: super::clock::local_offset_seconds(),
        }
    }
}

impl Widget for TimeProgress {
    fn actions(&self) -> &'static [&'static str] {
        &["toggle", "next", "reset"]
    }

    fn id(&self) -> &str {
        "time-progress"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(30)
    }

    fn poll(&mut self) -> WidgetState {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
            + self.offset;
        let fraction = fraction(self.span, now.div_euclid(86_400), now.rem_euclid(86_400));

        WidgetState::new(
            "time-progress",
            Body::Simple(Simple {
                label: format!("{}%", (fraction * 100.0).round() as i64),
                detail: self.span.name().to_string(),
                progress: fraction,
                active: false,
            }),
        )
    }

    fn invoke(&mut self, action: &str) {
        match action {
            "toggle" | "next" => self.span = self.span.next(),
            "reset" => self.span = Span::Day,
            unknown => tracing::warn!("time-progress ignoring unknown action {unknown}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(year: i64, month: u32, day: u32) -> i64 {
        days_from_civil(year, month, day)
    }

    #[test]
    fn midnight_is_the_start_of_the_day_and_one_second_to_midnight_is_the_end() {
        let d = day(2026, 6, 15);

        assert_eq!(fraction(Span::Day, d, 0), 0.0);
        assert!(fraction(Span::Day, d, 86_399) > 0.999);
    }

    #[test]
    fn noon_is_halfway_through_the_day() {
        assert!((fraction(Span::Day, day(2026, 6, 15), 43_200) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn the_week_starts_on_monday() {
        let monday = day(2026, 6, 15);

        assert_eq!(fraction(Span::Week, monday, 0), 0.0);
        assert!((fraction(Span::Week, monday + 6, 0) - 6.0 / 7.0).abs() < 1e-9);
    }

    #[test]
    fn the_first_of_the_month_is_the_start_of_the_month() {
        assert_eq!(fraction(Span::Month, day(2026, 6, 1), 0), 0.0);
    }

    #[test]
    fn a_short_month_and_a_long_one_are_measured_against_their_own_length() {
        let halfway_february = fraction(Span::Month, day(2026, 2, 15), 0);
        let halfway_january = fraction(Span::Month, day(2026, 1, 15), 0);

        assert!(halfway_february > halfway_january);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 1), 31);
    }

    #[test]
    fn a_leap_year_is_measured_against_three_hundred_and_sixty_six_days() {
        assert_eq!(days_in_year(2024), 366);
        assert_eq!(days_in_year(2026), 365);
    }

    #[test]
    fn the_first_of_january_is_the_start_of_the_year() {
        assert_eq!(fraction(Span::Year, day(2026, 1, 1), 0), 0.0);
    }

    #[test]
    fn the_last_day_of_the_year_is_nearly_all_of_it() {
        let fraction = fraction(Span::Year, day(2026, 12, 31), 43_200);

        assert!(fraction > 0.99 && fraction < 1.0);
    }

    #[test]
    fn every_span_stays_between_nothing_and_everything() {
        for span in [Span::Day, Span::Week, Span::Month, Span::Year] {
            for offset in 0..400 {
                let f = fraction(span, day(2026, 1, 1) + offset, 43_200);
                assert!((0.0..=1.0).contains(&f), "{span:?} escaped at day {offset}");
            }
        }
    }

    #[test]
    fn clicking_walks_through_every_span_and_comes_back() {
        let mut span = Span::Day;
        let mut seen = vec![span];

        for _ in 0..3 {
            span = span.next();
            seen.push(span);
        }

        assert_eq!(seen, vec![Span::Day, Span::Week, Span::Month, Span::Year]);
        assert_eq!(span.next(), Span::Day);
    }
}
