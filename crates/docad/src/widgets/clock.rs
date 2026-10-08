use std::time::{Duration, SystemTime, UNIX_EPOCH};

use doca_ipc::{Body, Simple, WidgetState, NO_PROGRESS};

use super::Widget;

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

pub fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as i64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = if month > 2 { month - 3 } else { month + 9 } as i64;
    let day_of_year = (153 * shifted_month + 2) / 5 + day as i64 - 1;
    let day_of_era =
        year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub fn weekday(days_since_epoch: i64) -> &'static str {
    WEEKDAYS[(days_since_epoch + 4).rem_euclid(7) as usize]
}

pub fn format_clock(seconds_since_epoch: u64, offset_seconds: i64) -> (String, String) {
    let local = seconds_since_epoch as i64 + offset_seconds;
    let day_seconds = local.rem_euclid(86_400);
    let days = local.div_euclid(86_400);
    let (_, month, day) = civil_from_days(days);
    (
        format!("{:02}:{:02}", day_seconds / 3600, (day_seconds % 3600) / 60),
        format!("{} {day:02}/{month:02}", weekday(days)),
    )
}

pub fn local_offset_seconds() -> i64 {
    let output = std::process::Command::new("date").arg("+%z").output().ok();
    let Some(output) = output else { return 0 };
    let text = String::from_utf8_lossy(&output.stdout);
    parse_utc_offset(text.trim()).unwrap_or(0)
}

pub fn parse_utc_offset(value: &str) -> Option<i64> {
    let bytes = value.as_bytes();
    if bytes.len() != 5 {
        return None;
    }
    let sign = match bytes[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let hours: i64 = value.get(1..3)?.parse().ok()?;
    let minutes: i64 = value.get(3..5)?.parse().ok()?;
    Some(sign * (hours * 3600 + minutes * 60))
}

pub struct Clock {
    offset: i64,
}

impl Clock {
    pub fn new() -> Self {
        Self {
            offset: local_offset_seconds(),
        }
    }
}

impl Widget for Clock {
    fn id(&self) -> &str {
        "clock"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn poll(&mut self) -> WidgetState {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let (label, detail) = format_clock(now, self.offset);

        WidgetState::new(
            "clock",
            Body::Simple(Simple {
                label,
                detail,
                progress: NO_PROGRESS,
                active: false,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midnight_utc_formats_as_zero_hundred() {
        assert_eq!(format_clock(0, 0).0, "00:00");
    }

    #[test]
    fn an_offset_shifts_the_clock_without_changing_the_instant() {
        let noon_utc = 12 * 3600;

        assert_eq!(format_clock(noon_utc, 0).0, "12:00");
        assert_eq!(format_clock(noon_utc, -3 * 3600).0, "09:00");
    }

    #[test]
    fn a_negative_offset_before_midnight_wraps_to_the_previous_day() {
        let one_am_utc = 3600;

        assert_eq!(format_clock(one_am_utc, -3 * 3600).0, "22:00");
    }

    #[test]
    fn the_label_only_changes_once_a_minute_however_often_it_is_polled() {
        let base = 10 * 3600 + 30 * 60;

        let at_zero = format_clock(base, 0).0;
        let at_thirty = format_clock(base + 30, 0).0;
        let at_sixty = format_clock(base + 60, 0).0;

        assert_eq!(at_zero, at_thirty);
        assert_ne!(at_zero, at_sixty);
    }

    #[test]
    fn nothing_the_clock_shows_changes_more_often_than_once_a_minute() {
        let base = 10 * 3600 + 30 * 60;

        for second in 1..60 {
            assert_eq!(
                format_clock(base + second, 0),
                format_clock(base, 0),
                "the tile changed {second}s into a minute, which wakes the bar for nothing"
            );
        }
    }

    #[test]
    fn the_epoch_is_a_thursday_in_january() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(weekday(0), "Thu");
    }

    #[test]
    fn a_date_and_its_day_number_convert_both_ways() {
        for days in [0i64, 1, 19_782, 20_000, -1, -719_468 + 1] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "round trip failed for {days}");
        }
    }

    #[test]
    fn a_leap_day_is_counted_as_a_real_day() {
        let leap_day = 19_782;

        assert_eq!(civil_from_days(leap_day), (2024, 2, 29));
    }

    #[test]
    fn the_detail_carries_the_date_rather_than_a_ticking_second() {
        let (_, detail) = format_clock(0, 0);

        assert_eq!(detail, "Thu 01/01");
    }

    #[test]
    fn a_utc_offset_string_parses_in_both_directions() {
        assert_eq!(parse_utc_offset("+0000"), Some(0));
        assert_eq!(parse_utc_offset("-0300"), Some(-10_800));
        assert_eq!(parse_utc_offset("+0530"), Some(19_800));
        assert_eq!(parse_utc_offset("garbage"), None);
    }
}
