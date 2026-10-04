use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use zbus::zvariant::{OwnedValue, Type};

pub const BUS_NAME: &str = "io.github.gusmartins499.Doca";
pub const OBJECT_PATH: &str = "/io/github/gusmartins499/Doca";
pub const INTERFACE: &str = "io.github.gusmartins499.Doca1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub app_id: String,
    pub workspace: i32,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DockItem {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub pinned: bool,
    pub windows: Vec<u32>,
    pub active: bool,
}

/// One dock as the daemon holds it: everything a window can change about it.
///
/// `pinned` and `widgets` are here for the same reason every `Appearance`
/// field is readable: a window that can reorder pins but not read them back
/// would have to keep its own guess of the order beside the daemon's, and the
/// two would disagree the first time something else wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EnvironmentInfo {
    pub name: String,
    pub workspaces: Vec<i32>,
    pub current: bool,
    pub pinned: Vec<String>,
    pub widgets: Vec<String>,
}

/// An installed application, as something offering a list of them needs it.
///
/// Not a `DockItem`: an app nobody has pinned and nobody is running has no
/// windows, no pin and no focus, and three fields saying so would invite a
/// caller to believe them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Application {
    pub id: String,
    pub name: String,
    pub icon: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct WidgetState {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub progress: f64,
    pub active: bool,
}

pub const NO_PROGRESS: f64 = -1.0;

/// The look of the dock as the daemon reports it.
///
/// Every field a writer can set is a field a reader can see: a preferences
/// window that could turn the trash on but not read back whether it is on
/// would have to keep its own guess of the truth beside the daemon's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Appearance {
    pub theme: String,
    pub icon_size: i32,
    pub magnification: f64,
    pub auto_hide: bool,
    pub show_trash: bool,
    /// An icon, GTK or cursor theme for the dock alone. Empty follows the
    /// system, which is what it does unless someone says otherwise.
    pub icon_theme: String,
    pub gtk_theme: String,
    pub cursor_theme: String,
}

/// The settings the widgets take, as the daemon reports them.
///
/// Flat, and named `widget_key` by `widget_key`, because that is the shape
/// `SetWidgetSetting` writes in: a getter grouped differently from the setter
/// is two spellings of the same five values to keep in step. Every field here
/// is a field that can be written, for the reason [`Appearance`] gives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct WidgetSettings {
    pub countdown_date: String,
    pub countdown_label: String,
    pub note_text: String,
    pub timer_minutes: u32,
    pub water_goal: u32,
}

/// The widgets that take a setting, and what `SetWidgetSetting` calls each.
///
/// A window building the controls needs the exact pair the daemon validates
/// against; the daemon's own `match` is the other half of this and `docad`
/// has a test that no pair here is refused.
pub mod widget_key {
    pub const COUNTDOWN: &str = "countdown";
    pub const NOTE: &str = "note";
    pub const TIMER: &str = "timer";
    pub const WATER: &str = "water";

    pub const DATE: &str = "date";
    pub const LABEL: &str = "label";
    pub const TEXT: &str = "text";
    pub const MINUTES: &str = "minutes";
    pub const GOAL: &str = "goal";

    /// Every (widget, key) pair that exists, for a test to walk.
    pub const ALL: [(&str, &str); 5] = [
        (COUNTDOWN, DATE),
        (COUNTDOWN, LABEL),
        (NOTE, TEXT),
        (TIMER, MINUTES),
        (WATER, GOAL),
    ];
}

/// A date as the countdown widget reads it: `YYYY-MM-DD` and nothing else.
///
/// Here rather than in the daemon because both ends need the same answer: the
/// widget turns the text into a day, and a window has to be able to say "that
/// is not a date" before sending text that would show up in the bar as
/// "bad date" with no explanation of why.
pub fn parse_date(value: &str) -> Option<(i64, u32, u32)> {
    let mut parts = value.trim().split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

/// The themes the bar can wear, in the order something listing them should.
///
/// Here rather than in the shell because three crates need to agree on it: the
/// shell owns the stylesheets, the daemon validates what is asked for, and a
/// preferences window has to offer the list without writing it out by hand —
/// a hand-written copy is a list that goes stale the first time a theme is
/// added.
pub const THEMES: [&str; 4] = ["system", "native", "midnight", "paper"];

/// The widgets a dock can show, in the order something listing them should.
///
/// Here for the reason [`THEMES`] is: the daemon builds them, and a window has
/// to offer the list without writing it out by hand. `docad` has a test that
/// every id here builds into a real widget, so a name that goes stale fails
/// the suite rather than logging "ignoring unknown widget" at someone.
pub const WIDGETS: [&str; 12] = [
    "clock",
    "battery",
    "cpu",
    "network",
    "music",
    "time-progress",
    "pomodoro",
    "timer",
    "stopwatch",
    "countdown",
    "water",
    "note",
];

/// The theme a config that never mentioned one gets.
pub const DEFAULT_THEME: &str = "native";

/// The limits every writer is held to, so a window can show them as a range
/// instead of guessing and being corrected after the fact.
pub const MIN_ICON_SIZE: i32 = 24;
pub const MAX_ICON_SIZE: i32 = 96;
/// 1.0 is how the lens is turned off, so it is also the floor.
pub const MIN_MAGNIFICATION: f64 = 1.0;
pub const MAX_MAGNIFICATION: f64 = 2.5;
/// A timer of no minutes has nothing to count, so one is the floor.
pub const MIN_TIMER_MINUTES: u32 = 1;
pub const MAX_TIMER_MINUTES: u32 = 24 * 60;
pub const MIN_WATER_GOAL: u32 = 1;
pub const MAX_WATER_GOAL: u32 = 64;

/// The keys `SetAppearance` understands, by the name they carry on the wire.
pub mod appearance_key {
    pub const THEME: &str = "theme";
    pub const ICON_SIZE: &str = "icon_size";
    pub const MAGNIFICATION: &str = "magnification";
    pub const AUTO_HIDE: &str = "auto_hide";
    pub const SHOW_TRASH: &str = "show_trash";
    pub const ICON_THEME: &str = "icon_theme";
    pub const GTK_THEME: &str = "gtk_theme";
    pub const CURSOR_THEME: &str = "cursor_theme";

    pub const ALL: [&str; 8] = [
        THEME,
        ICON_SIZE,
        MAGNIFICATION,
        AUTO_HIDE,
        SHOW_TRASH,
        ICON_THEME,
        GTK_THEME,
        CURSOR_THEME,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct FolderEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[zbus::proxy(
    interface = "io.github.gusmartins499.Doca1",
    default_service = "io.github.gusmartins499.Doca",
    default_path = "/io/github/gusmartins499/Doca"
)]
pub trait Doca {
    fn list_environments(&self) -> zbus::Result<Vec<EnvironmentInfo>>;
    fn current_environment(&self) -> zbus::Result<String>;
    fn set_environment(&self, name: &str) -> zbus::Result<()>;
    fn cycle_environment(&self) -> zbus::Result<String>;

    fn appearance(&self) -> zbus::Result<Appearance>;

    /// Change only the keys named, leaving the rest of the look alone.
    ///
    /// A dictionary rather than a whole `Appearance` so that a window with one
    /// slider under the pointer does not have to send back five values it read
    /// a minute ago — and so adding a key later does not break a caller that
    /// never heard of it.
    fn set_appearance(&self, changes: HashMap<String, OwnedValue>) -> zbus::Result<()>;

    /// What every widget setting is right now.
    ///
    /// The counterpart of `SetWidgetSetting`, and already clamped: a window
    /// can put these straight on its controls without checking them again.
    fn widget_settings(&self) -> zbus::Result<WidgetSettings>;

    fn set_widget_setting(
        &self,
        widget: &str,
        key: &str,
        value: OwnedValue,
    ) -> zbus::Result<()>;

    fn set_environment_widgets(&self, name: &str, widgets: Vec<String>) -> zbus::Result<()>;
    fn set_environment_workspaces(&self, name: &str, workspaces: Vec<i32>) -> zbus::Result<()>;
    fn reorder_pinned(&self, name: &str, order: Vec<String>) -> zbus::Result<()>;

    /// Put the docks in a new order, which is the order the cycle walks.
    ///
    /// Only an order: a name nobody knows is refused, and a dock left out
    /// keeps its place at the end — so a window holding a list drawn before a
    /// rename reorders what it knows instead of deleting what it does not.
    fn reorder_environments(&self, order: Vec<String>) -> zbus::Result<()>;
    fn add_environment(&self, name: &str) -> zbus::Result<String>;
    fn remove_environment(&self, name: &str) -> zbus::Result<()>;
    fn rename_environment(&self, from: &str, to: &str) -> zbus::Result<String>;

    /// Every installed application, for a window that has to offer a choice
    /// of them. Sorted by name, and apps that ask not to be shown are not.
    fn list_applications(&self) -> zbus::Result<Vec<Application>>;

    /// Pin to a named dock rather than the one on screen.
    ///
    /// `PinItem` pins where the user is looking, which is what a click on the
    /// bar means. A preferences window is editing a dock it may not be
    /// standing in, so it has to name the one it means.
    fn pin_in(&self, name: &str, id: &str) -> zbus::Result<()>;
    fn unpin_in(&self, name: &str, id: &str) -> zbus::Result<()>;

    fn list_widgets(&self) -> zbus::Result<Vec<WidgetState>>;
    fn invoke_widget(&self, id: &str, action: &str) -> zbus::Result<()>;

    fn list_items(&self) -> zbus::Result<Vec<DockItem>>;
    fn activate_item(&self, id: &str) -> zbus::Result<()>;
    fn launch_item(&self, id: &str) -> zbus::Result<()>;
    fn open_with(&self, id: &str, paths: &[&str]) -> zbus::Result<()>;
    fn list_folder(&self, id: &str) -> zbus::Result<Vec<FolderEntry>>;
    fn open_path(&self, path: &str) -> zbus::Result<()>;
    fn item_windows(&self, id: &str) -> zbus::Result<Vec<WindowInfo>>;
    fn pin_item(&self, id: &str) -> zbus::Result<()>;
    fn unpin_item(&self, id: &str) -> zbus::Result<()>;
    fn close_window(&self, id: u32) -> zbus::Result<()>;

    fn list_windows(&self) -> zbus::Result<Vec<WindowInfo>>;
    fn activate_window(&self, id: u32) -> zbus::Result<()>;
    fn current_workspace(&self) -> zbus::Result<i32>;
    fn workspace_count(&self) -> zbus::Result<i32>;
    fn set_workspace(&self, index: i32) -> zbus::Result<()>;

    /// The config on disk changed, whoever changed it.
    ///
    /// One signal for the whole file rather than one per key: everything a
    /// reader does with it ends in the same re-read, and a bar that rebuilds
    /// once is cheaper than a bar that rebuilds five times because five keys
    /// moved together.
    #[zbus(signal)]
    fn config_changed(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn environment_changed(&self, name: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    fn widget_changed(&self, state: WidgetState) -> zbus::Result<()>;

    #[zbus(signal)]
    fn items_changed(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn windows_changed(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn workspace_changed(&self, index: i32) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_string_parses_only_when_it_is_really_a_date() {
        assert_eq!(parse_date("2026-12-25"), Some((2026, 12, 25)));
        assert_eq!(parse_date(" 2026-01-01 "), Some((2026, 1, 1)));
        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("2026-00-10"), None);
        assert_eq!(parse_date("25/12/2026"), None);
        assert_eq!(parse_date("2026-12-25-01"), None);
        assert_eq!(parse_date("tomorrow"), None);
        assert_eq!(parse_date(""), None);
    }

    /// Both lists are walked by something; neither may hold a name twice.
    #[test]
    fn nothing_is_offered_twice() {
        let mut widgets = WIDGETS.to_vec();
        widgets.sort_unstable();
        let was = widgets.len();
        widgets.dedup();
        assert_eq!(widgets.len(), was, "a widget id is in the list twice");

        let mut pairs = widget_key::ALL.to_vec();
        pairs.sort_unstable();
        let was = pairs.len();
        pairs.dedup();
        assert_eq!(pairs.len(), was, "a widget setting is in the list twice");
    }

    /// Every widget a setting is addressed to has to be a widget on offer.
    #[test]
    fn a_setting_belongs_to_a_widget_that_exists() {
        for (widget, key) in widget_key::ALL {
            assert!(
                WIDGETS.contains(&widget),
                "{widget}.{key} names a widget that is not on offer"
            );
        }
    }
}
