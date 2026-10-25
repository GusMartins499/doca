//! The monitor the dock stands on. #18, item 11.
//!
//! Chosen by name, the one RandR gives the output — `eDP-1`, `DisplayPort-0`,
//! what `xrandr` lists and what GDK reports as a monitor's model. The name
//! and not an index, because indices are the order the X server happened to
//! enumerate outputs in, which a cable plugged into the other port changes;
//! and not the monitor's make and model, which two identical screens share.
//! Empty is the primary, which is what the dock did before there was a
//! choice, so a config that never named one keeps the dock where it was.
//!
//! "The monitor the pointer is on" was the third option the issue offered,
//! and it is not here, decided: a dock that moves to wherever the pointer
//! went moves every time a hand crosses to the other screen to read
//! something, and the strut moves with it — the windows on both screens
//! would be resized under the user. That is a dock to follow the pointer on
//! hover at the bottom edge, the way macOS does it, and a feature of its own.
//!
//! A named monitor that is not there — unplugged, or a laptop closed — puts
//! the dock on the primary, and back on the named one when it comes back:
//! the name stays in the config, because unplugging a screen is not changing
//! your mind about it. That is said once in the log, not on every rebuild.

use std::cell::{Cell, RefCell};

use gtk::prelude::*;

/// One monitor, as much of it as choosing needs.
#[derive(Debug, Clone)]
pub struct Monitor {
    pub name: String,
    pub primary: bool,
    pub area: gdk::Rectangle,
}

/// Which monitor to stand on, and whether it is the one that was asked for.
#[derive(Debug, PartialEq)]
pub enum Choice {
    /// The one asked for: the named one, or the primary when none was named.
    Asked(usize),
    /// The named one is not connected; this is where the dock stands until it is.
    Instead(usize),
}

impl Choice {
    pub fn index(&self) -> usize {
        match self {
            Choice::Asked(index) | Choice::Instead(index) => *index,
        }
    }
}

/// The primary — and the first there is when none is primary, which is what
/// a bare X server, or `xrandr` with no `--primary`, leaves you with: a dock
/// that refused to start there was a dock missing for no reason.
fn primary(monitors: &[Monitor]) -> Option<usize> {
    monitors
        .iter()
        .position(|monitor| monitor.primary)
        .or(if monitors.is_empty() { None } else { Some(0) })
}

pub fn choose(monitors: &[Monitor], wanted: &str) -> Option<Choice> {
    if wanted.is_empty() {
        return primary(monitors).map(Choice::Asked);
    }
    match monitors.iter().position(|monitor| monitor.name == wanted) {
        Some(index) => Some(Choice::Asked(index)),
        None => primary(monitors).map(Choice::Instead),
    }
}

/// The monitors connected now, in the order GDK has them.
pub fn connected() -> Vec<Monitor> {
    let Some(display) = gdk::Display::default() else {
        return Vec::new();
    };
    (0..display.n_monitors())
        .filter_map(|index| display.monitor(index))
        .map(|monitor| Monitor {
            name: monitor
                .model()
                .map(|name| name.to_string())
                .unwrap_or_default(),
            primary: monitor.is_primary(),
            area: monitor.geometry(),
        })
        .collect()
}

/// How far the bottom of the whole X screen is below this area's bottom.
///
/// The X screen is the box around every monitor, so a monitor shorter than
/// the one beside it has screen left below it that is no monitor's. Never
/// negative: an area is inside the screen it is measured against.
pub fn below(area: &gdk::Rectangle, monitors: &[Monitor]) -> i32 {
    let bottom = monitors
        .iter()
        .map(|monitor| monitor.area.y() + monitor.area.height())
        .max()
        .unwrap_or(0);
    (bottom - (area.y() + area.height())).max(0)
}

/// Where the dock stands, kept between rebuilds so a move can be told from
/// being told twice about the same screen, which X does.
pub struct Stand {
    area: Cell<gdk::Rectangle>,
    wanted: RefCell<String>,
    /// The named monitor last found missing, so its absence is logged once.
    missing: RefCell<Option<String>>,
}

impl Stand {
    pub fn new(area: gdk::Rectangle) -> Self {
        Self {
            area: Cell::new(area),
            wanted: RefCell::new(String::new()),
            missing: RefCell::new(None),
        }
    }

    /// The area to put the dock on for this monitor name, measured now.
    /// With no monitor at all, the last one there was: a bar left where it
    /// was beats a bar put nowhere.
    pub fn measure(&self, wanted: &str) -> gdk::Rectangle {
        self.wanted.replace(wanted.to_string());
        let monitors = connected();
        let Some(choice) = choose(&monitors, wanted) else {
            tracing::warn!("no monitor left to put the dock on; staying where it is");
            return self.area.get();
        };
        let chosen = &monitors[choice.index()];
        match choice {
            Choice::Instead(_) => {
                if self.missing.borrow().as_deref() != Some(wanted) {
                    tracing::warn!(
                        wanted,
                        instead = chosen.name,
                        "the monitor the dock was given is not connected; standing on the primary until it is"
                    );
                    self.missing.replace(Some(wanted.to_string()));
                }
            }
            Choice::Asked(_) => {
                if self.missing.replace(None).is_some() {
                    tracing::info!(monitor = chosen.name, "the dock's own monitor is back");
                }
            }
        }
        let area = chosen.area;
        if self.area.replace(area) != area {
            tracing::info!(
                monitor = chosen.name,
                width = area.width(),
                height = area.height(),
                x = area.x(),
                y = area.y(),
                "the screen changed under the dock"
            );
        }
        area
    }

    /// Measure again for the monitor last asked for. Whether anything moved.
    pub fn moved(&self) -> bool {
        let before = self.area.get();
        let wanted = self.wanted.borrow().clone();
        self.measure(&wanted) != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(name: &str, primary: bool, x: i32) -> Monitor {
        Monitor {
            name: name.to_string(),
            primary,
            area: gdk::Rectangle::new(x, 0, 1920, 1080),
        }
    }

    fn two() -> Vec<Monitor> {
        vec![
            monitor("DisplayPort-0", true, 0),
            monitor("eDP", false, 1920),
        ]
    }

    #[test]
    fn nothing_named_is_the_primary() {
        assert_eq!(choose(&two(), ""), Some(Choice::Asked(0)));
    }

    #[test]
    fn a_named_monitor_is_the_one_stood_on_primary_or_not() {
        assert_eq!(choose(&two(), "eDP"), Some(Choice::Asked(1)));
        assert_eq!(choose(&two(), "DisplayPort-0"), Some(Choice::Asked(0)));
    }

    #[test]
    fn a_named_monitor_that_is_gone_is_stood_in_for_by_the_primary() {
        assert_eq!(choose(&two(), "HDMI-1"), Some(Choice::Instead(0)));
    }

    #[test]
    fn a_name_is_matched_as_it_is_written() {
        assert_eq!(
            choose(&two(), "edp"),
            Some(Choice::Instead(0)),
            "RandR's names are case-sensitive, and so is xrandr --output"
        );
    }

    #[test]
    fn with_no_primary_the_first_monitor_stands_in() {
        let monitors = vec![monitor("screen", false, 0), monitor("other", false, 1920)];
        assert_eq!(choose(&monitors, ""), Some(Choice::Asked(0)));
        assert_eq!(choose(&monitors, "gone"), Some(Choice::Instead(0)));
    }

    #[test]
    fn side_by_side_monitors_of_one_height_share_a_bottom_edge() {
        let monitors = two();
        assert_eq!(below(&monitors[0].area, &monitors), 0);
        assert_eq!(below(&monitors[1].area, &monitors), 0);
    }

    #[test]
    fn a_shorter_monitor_has_screen_below_it_that_the_strut_must_count() {
        let laptop = Monitor {
            name: "eDP".to_string(),
            primary: false,
            area: gdk::Rectangle::new(2560, 0, 1920, 1080),
        };
        let monitors = vec![
            Monitor {
                name: "DisplayPort-0".to_string(),
                primary: true,
                area: gdk::Rectangle::new(0, 0, 2560, 1440),
            },
            laptop.clone(),
        ];
        assert_eq!(below(&laptop.area, &monitors), 360);
        assert_eq!(below(&monitors[0].area, &monitors), 0);
    }

    #[test]
    fn with_no_monitor_there_is_nothing_to_stand_on() {
        assert_eq!(choose(&[], ""), None);
        assert_eq!(choose(&[], "eDP"), None);
    }
}
