use std::path::{Path, PathBuf};
use std::time::Duration;

use primodock_ipc::{WidgetState, NO_PROGRESS};

use super::Widget;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub percent: u8,
    pub status: String,
    pub now: u64,
    pub full: u64,
    pub rate: u64,
}

pub fn format_remaining(reading: &Reading) -> String {
    if reading.rate == 0 {
        return match reading.status.as_str() {
            "Full" => "full".to_string(),
            "Charging" => "charging".to_string(),
            _ => String::new(),
        };
    }

    let remaining = if reading.status == "Charging" {
        reading.full.saturating_sub(reading.now)
    } else {
        reading.now
    };

    let minutes = (remaining as f64 / reading.rate as f64 * 60.0).round() as u64;
    let text = if minutes >= 60 {
        format!("{}h {:02}", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}m")
    };

    if reading.status == "Charging" {
        format!("{text} to full")
    } else {
        text
    }
}

pub fn state_from(reading: &Reading) -> WidgetState {
    WidgetState {
        id: "battery".to_string(),
        label: format!("{}%", reading.percent),
        detail: format_remaining(reading),
        progress: reading.percent as f64 / 100.0,
        active: reading.status == "Charging",
    }
}

fn read_number(dir: &Path, name: &str) -> Option<u64> {
    std::fs::read_to_string(dir.join(name))
        .ok()?
        .trim()
        .parse()
        .ok()
}

pub fn read_reading(dir: &Path) -> Option<Reading> {
    let percent = read_number(dir, "capacity")? as u8;
    let status = std::fs::read_to_string(dir.join("status"))
        .ok()?
        .trim()
        .to_string();

    let (now, full, rate) = match read_number(dir, "energy_now") {
        Some(now) => (
            now,
            read_number(dir, "energy_full").unwrap_or(0),
            read_number(dir, "power_now").unwrap_or(0),
        ),
        None => (
            read_number(dir, "charge_now").unwrap_or(0),
            read_number(dir, "charge_full").unwrap_or(0),
            read_number(dir, "current_now").unwrap_or(0),
        ),
    };

    Some(Reading {
        percent,
        status,
        now,
        full,
        rate,
    })
}

fn first_battery() -> Option<PathBuf> {
    let entries = std::fs::read_dir("/sys/class/power_supply").ok()?;
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("BAT"))
        })
        .min()
}

pub struct Battery {
    path: Option<PathBuf>,
}

impl Battery {
    pub fn new() -> Self {
        Self {
            path: first_battery(),
        }
    }
}

impl Widget for Battery {
    fn id(&self) -> &str {
        "battery"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(30)
    }

    fn poll(&mut self) -> WidgetState {
        let reading = self.path.as_deref().and_then(read_reading);
        match reading {
            Some(reading) => state_from(&reading),
            None => WidgetState {
                id: "battery".to_string(),
                label: "—".to_string(),
                detail: "no battery".to_string(),
                progress: NO_PROGRESS,
                active: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(status: &str, now: u64, full: u64, rate: u64, percent: u8) -> Reading {
        Reading {
            percent,
            status: status.to_string(),
            now,
            full,
            rate,
        }
    }

    #[test]
    fn a_discharging_battery_reports_how_long_it_has_left() {
        let two_and_a_half_hours = reading("Discharging", 25_000_000, 38_290_000, 10_000_000, 65);

        assert_eq!(format_remaining(&two_and_a_half_hours), "2h 30");
    }

    #[test]
    fn a_charging_battery_reports_time_to_full_not_time_left() {
        let charging = reading("Charging", 36_990_000, 38_290_000, 4_486_000, 96);

        assert_eq!(format_remaining(&charging), "17m to full");
    }

    #[test]
    fn under_an_hour_is_reported_in_minutes_alone() {
        let nearly_empty = reading("Discharging", 2_000_000, 38_290_000, 10_000_000, 5);

        assert_eq!(format_remaining(&nearly_empty), "12m");
    }

    #[test]
    fn a_battery_drawing_no_power_does_not_divide_by_zero() {
        assert_eq!(format_remaining(&reading("Full", 38_290_000, 38_290_000, 0, 100)), "full");
        assert_eq!(format_remaining(&reading("Charging", 1, 2, 0, 50)), "charging");
        assert_eq!(format_remaining(&reading("Discharging", 1, 2, 0, 50)), "");
    }

    #[test]
    fn charging_is_the_state_that_makes_the_tile_active() {
        assert!(state_from(&reading("Charging", 1, 2, 1, 50)).active);
        assert!(!state_from(&reading("Discharging", 1, 2, 1, 50)).active);
    }

    #[test]
    fn the_progress_ring_follows_the_reported_percentage() {
        assert_eq!(state_from(&reading("Full", 1, 1, 0, 100)).progress, 1.0);
        assert_eq!(state_from(&reading("Discharging", 1, 2, 1, 50)).progress, 0.5);
    }

    #[test]
    fn a_laptop_reporting_charge_instead_of_energy_is_read_the_same_way() {
        let dir = tempdir();
        std::fs::write(dir.join("capacity"), "80\n").unwrap();
        std::fs::write(dir.join("status"), "Discharging\n").unwrap();
        std::fs::write(dir.join("charge_now"), "4000000\n").unwrap();
        std::fs::write(dir.join("charge_full"), "5000000\n").unwrap();
        std::fs::write(dir.join("current_now"), "2000000\n").unwrap();

        let reading = read_reading(&dir).unwrap();

        assert_eq!(reading.percent, 80);
        assert_eq!(format_remaining(&reading), "2h 00");
    }

    #[test]
    fn energy_is_preferred_when_a_laptop_reports_both() {
        let dir = tempdir();
        std::fs::write(dir.join("capacity"), "50\n").unwrap();
        std::fs::write(dir.join("status"), "Discharging\n").unwrap();
        std::fs::write(dir.join("energy_now"), "10\n").unwrap();
        std::fs::write(dir.join("energy_full"), "20\n").unwrap();
        std::fs::write(dir.join("power_now"), "5\n").unwrap();
        std::fs::write(dir.join("charge_now"), "999\n").unwrap();

        assert_eq!(read_reading(&dir).unwrap().now, 10);
    }

    fn tempdir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "primodock-battery-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}
