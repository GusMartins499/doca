use std::time::{Duration, Instant};

use doca_ipc::{Body, Simple, WidgetState, NO_PROGRESS};

use super::Widget;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counters {
    pub received: u64,
    pub sent: u64,
}

pub fn parse_counters(proc_net_dev: &str) -> Counters {
    let mut total = Counters::default();
    for line in proc_net_dev.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name == "lo" || name.starts_with("docker") || name.starts_with("veth") {
            continue;
        }
        let fields: Vec<u64> = rest
            .split_whitespace()
            .filter_map(|field| field.parse().ok())
            .collect();
        if fields.len() < 9 {
            continue;
        }
        total.received += fields[0];
        total.sent += fields[8];
    }
    total
}

pub fn format_rate(bytes_per_second: f64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    if bytes_per_second >= MIB {
        format!("{:.1}M", bytes_per_second / MIB)
    } else if bytes_per_second >= KIB {
        format!("{:.0}K", bytes_per_second / KIB)
    } else {
        "0K".to_string()
    }
}

pub fn rate_between(previous: u64, current: u64, seconds: f64) -> f64 {
    if seconds <= 0.0 {
        return 0.0;
    }
    current.saturating_sub(previous) as f64 / seconds
}

pub struct Network {
    previous: Option<(Counters, Instant)>,
}

impl Network {
    pub fn new() -> Self {
        Self { previous: None }
    }
}

fn read_counters() -> Counters {
    std::fs::read_to_string("/proc/net/dev")
        .map(|contents| parse_counters(&contents))
        .unwrap_or_default()
}

impl Widget for Network {
    fn id(&self) -> &str {
        "network"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(2)
    }

    fn poll(&mut self) -> WidgetState {
        let now = Instant::now();
        let current = read_counters();

        let (down, up) = match self.previous {
            Some((previous, when)) => {
                let seconds = now.saturating_duration_since(when).as_secs_f64();
                (
                    rate_between(previous.received, current.received, seconds),
                    rate_between(previous.sent, current.sent, seconds),
                )
            }
            None => (0.0, 0.0),
        };
        self.previous = Some((current, now));

        WidgetState::new(
            "network",
            Body::Simple(Simple {
                label: format!("\u{2193}{}", format_rate(down)),
                detail: format!("\u{2191}{}", format_rate(up)),
                progress: NO_PROGRESS,
                active: down + up > 256.0 * 1024.0,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 1000000    1000    0    0    0     0          0         0  1000000    1000    0    0    0     0       0          0
  eth0: 5000    50    0    0    0     0          0         0     2000      20    0    0    0     0       0          0
 wlan0: 3000    30    0    0    0     0          0         0     1000      10    0    0    0     0       0          0
";

    #[test]
    fn loopback_traffic_is_not_counted_as_network_traffic() {
        let counters = parse_counters(SAMPLE);

        assert_eq!(counters.received, 8000);
        assert_eq!(counters.sent, 3000);
    }

    #[test]
    fn every_real_interface_is_added_together() {
        let counters = parse_counters(SAMPLE);

        assert_eq!(counters.received, 5000 + 3000);
    }

    #[test]
    fn container_and_virtual_interfaces_are_left_out() {
        let with_docker = format!(
            "{SAMPLE}docker0: 9000    90    0    0    0     0          0         0     9000      90    0    0    0     0       0          0\n"
        );

        assert_eq!(parse_counters(&with_docker).received, 8000);
    }

    #[test]
    fn a_rate_is_the_difference_over_the_time_it_took() {
        assert_eq!(rate_between(1000, 3000, 2.0), 1000.0);
    }

    #[test]
    fn a_counter_that_wrapped_or_reset_reports_no_traffic_rather_than_a_huge_spike() {
        assert_eq!(rate_between(5000, 1000, 2.0), 0.0);
    }

    #[test]
    fn two_samples_at_the_same_instant_do_not_divide_by_zero() {
        assert_eq!(rate_between(1000, 3000, 0.0), 0.0);
    }

    #[test]
    fn rates_are_shown_in_units_a_person_reads() {
        assert_eq!(format_rate(0.0), "0K");
        assert_eq!(format_rate(500.0), "0K");
        assert_eq!(format_rate(2048.0), "2K");
        assert_eq!(format_rate(1024.0 * 1024.0 * 3.5), "3.5M");
    }

    #[test]
    fn a_malformed_line_is_skipped_rather_than_counted_wrong() {
        let broken = "h1\nh2\n  eth0: not numbers here\n";

        assert_eq!(parse_counters(broken), Counters::default());
    }
}
