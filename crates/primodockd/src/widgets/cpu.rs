use std::time::Duration;

use primodock_ipc::WidgetState;

use super::Widget;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sample {
    pub busy: u64,
    pub total: u64,
}

pub fn parse_sample(proc_stat_line: &str) -> Option<Sample> {
    let mut fields = proc_stat_line.split_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    let values: Vec<u64> = fields.filter_map(|field| field.parse().ok()).collect();
    if values.len() < 4 {
        return None;
    }
    let idle = values[3] + values.get(4).copied().unwrap_or(0);
    let total: u64 = values.iter().sum();
    Some(Sample {
        busy: total.saturating_sub(idle),
        total,
    })
}

pub fn usage_between(previous: Sample, current: Sample) -> f64 {
    let total = current.total.saturating_sub(previous.total);
    if total == 0 {
        return 0.0;
    }
    let busy = current.busy.saturating_sub(previous.busy);
    (busy as f64 / total as f64).clamp(0.0, 1.0)
}

pub struct Cpu {
    previous: Option<Sample>,
}

impl Cpu {
    pub fn new() -> Self {
        Self { previous: None }
    }
}

fn read_sample() -> Option<Sample> {
    let contents = std::fs::read_to_string("/proc/stat").ok()?;
    parse_sample(contents.lines().next()?)
}

impl Widget for Cpu {
    fn id(&self) -> &str {
        "cpu"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(2)
    }

    fn poll(&mut self) -> WidgetState {
        let current = read_sample().unwrap_or_default();
        let usage = match self.previous {
            Some(previous) => usage_between(previous, current),
            None => 0.0,
        };
        self.previous = Some(current);

        WidgetState {
            id: "cpu".to_string(),
            label: format!("{}%", (usage * 100.0).round() as u8),
            detail: "CPU".to_string(),
            progress: usage,
            active: usage > 0.8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proc_stat_line_splits_into_busy_and_total() {
        let sample = parse_sample("cpu  100 10 50 800 40 0 0 0 0 0").unwrap();

        assert_eq!(sample.total, 1000);
        assert_eq!(sample.busy, 160);
    }

    #[test]
    fn usage_is_the_busy_share_of_the_interval_not_of_all_time() {
        let previous = Sample { busy: 100, total: 1000 };
        let current = Sample { busy: 150, total: 1200 };

        assert_eq!(usage_between(previous, current), 0.25);
    }

    #[test]
    fn two_identical_samples_report_no_usage_rather_than_dividing_by_zero() {
        let sample = Sample { busy: 100, total: 1000 };

        assert_eq!(usage_between(sample, sample), 0.0);
    }

    #[test]
    fn a_counter_that_went_backwards_does_not_panic_or_exceed_one() {
        let previous = Sample { busy: 500, total: 1000 };
        let current = Sample { busy: 100, total: 1200 };

        let usage = usage_between(previous, current);

        assert!((0.0..=1.0).contains(&usage));
    }

    #[test]
    fn a_per_core_line_is_not_mistaken_for_the_aggregate() {
        assert!(parse_sample("cpu0 100 10 50 800 40 0 0 0 0 0").is_none());
        assert!(parse_sample("intr 12345").is_none());
    }
}
