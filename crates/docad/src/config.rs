use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_ENVIRONMENT: &str = "Default";
pub const DEFAULT_THEME: &str = "native";
pub const MIN_ICON_SIZE: i32 = 24;
pub const MAX_ICON_SIZE: i32 = 96;
/// 1.0 is how the lens is turned off, so it is also the floor.
pub const MIN_MAGNIFICATION: f64 = 1.0;
pub const MAX_MAGNIFICATION: f64 = 2.5;
pub const MIN_TIMER_MINUTES: u32 = 1;
pub const MAX_TIMER_MINUTES: u32 = 24 * 60;
pub const MIN_WATER_GOAL: u32 = 1;
pub const MAX_WATER_GOAL: u32 = 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_icon_size")]
    pub icon_size: i32,
    #[serde(default = "default_magnification")]
    pub magnification: f64,
    #[serde(default = "default_show_trash")]
    pub show_trash: bool,
    #[serde(default)]
    pub auto_hide: bool,
}

fn default_show_trash() -> bool {
    true
}

fn default_magnification() -> f64 {
    1.6
}

fn default_theme() -> String {
    DEFAULT_THEME.to_string()
}

fn default_icon_size() -> i32 {
    48
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            icon_size: default_icon_size(),
            magnification: default_magnification(),
            show_trash: default_show_trash(),
            auto_hide: false,
        }
    }
}

impl Appearance {
    pub fn sanitised(&self) -> Self {
        Self {
            theme: self.theme.trim().to_lowercase(),
            icon_size: self.icon_size.clamp(MIN_ICON_SIZE, MAX_ICON_SIZE),
            magnification: if self.magnification.is_finite() {
                self.magnification.clamp(MIN_MAGNIFICATION, MAX_MAGNIFICATION)
            } else {
                MIN_MAGNIFICATION
            },
            show_trash: self.show_trash,
            auto_hide: self.auto_hide,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    pub name: String,
    #[serde(default)]
    pub workspaces: Vec<i32>,
    #[serde(default)]
    pub pinned: Vec<String>,
    #[serde(default)]
    pub widgets: Vec<String>,
    #[serde(default)]
    pub folders: Vec<String>,
}

impl Environment {
    pub fn claims(&self, workspace: i32) -> bool {
        self.workspaces.contains(&workspace)
    }

    pub fn is_catch_all(&self) -> bool {
        self.workspaces.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CountdownSettings {
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub label: String,
}

impl Default for CountdownSettings {
    fn default() -> Self {
        Self {
            date: String::new(),
            label: "until".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoteSettings {
    #[serde(default)]
    pub text: String,
}

impl Default for NoteSettings {
    fn default() -> Self {
        Self {
            text: "a note lives here".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimerSettings {
    #[serde(default = "default_timer_minutes")]
    pub minutes: u32,
}

fn default_timer_minutes() -> u32 {
    10
}

impl Default for TimerSettings {
    fn default() -> Self {
        Self {
            minutes: default_timer_minutes(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaterSettings {
    #[serde(default = "default_water_goal")]
    pub goal: u32,
}

fn default_water_goal() -> u32 {
    8
}

impl Default for WaterSettings {
    fn default() -> Self {
        Self {
            goal: default_water_goal(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WidgetSettings {
    #[serde(default)]
    pub countdown: CountdownSettings,
    #[serde(default)]
    pub note: NoteSettings,
    #[serde(default)]
    pub timer: TimerSettings,
    #[serde(default)]
    pub water: WaterSettings,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub appearance: Appearance,
    #[serde(default)]
    pub widgets: WidgetSettings,
    #[serde(default)]
    pub environments: Vec<Environment>,
    #[serde(default, skip_serializing)]
    pinned: Vec<String>,
}

/// A name no other writer is using, next to the file it will become.
///
/// Same directory, because `rename` is only atomic within one filesystem.
fn temporary_name(path: &Path) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stem = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "config.toml".to_string());
    format!(
        ".{stem}.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("doca/config.toml")
}

impl WidgetSettings {
    #[cfg(test)]
    pub fn date_is_unset(&self) -> bool {
        self.countdown.date.trim().is_empty()
    }

    pub fn sanitised(&self) -> Self {
        Self {
            countdown: CountdownSettings {
                date: self.countdown.date.trim().to_string(),
                label: self.countdown.label.trim().to_string(),
            },
            note: NoteSettings {
                text: self.note.text.clone(),
            },
            timer: TimerSettings {
                minutes: self.timer.minutes.clamp(MIN_TIMER_MINUTES, MAX_TIMER_MINUTES),
            },
            water: WaterSettings {
                goal: self.water.goal.clamp(MIN_WATER_GOAL, MAX_WATER_GOAL),
            },
        }
    }

    /// The keys a caller is allowed to name, and nothing else.
    ///
    /// A typo in a widget id or a key is told to the caller rather than
    /// written to disk — a silently ignored setting is the kind of thing the
    /// user reads as "the dock is broken".
    pub fn set(&mut self, widget: &str, key: &str, value: Setting) -> Result<()> {
        match (widget, key) {
            ("countdown", "date") => self.countdown.date = value.into_text()?,
            ("countdown", "label") => self.countdown.label = value.into_text()?,
            ("note", "text") => self.note.text = value.into_text()?,
            ("timer", "minutes") => self.timer.minutes = value.into_count()?,
            ("water", "goal") => self.water.goal = value.into_count()?,
            _ => anyhow::bail!("no setting {key} on widget {widget}"),
        }
        *self = self.sanitised();
        Ok(())
    }
}

/// One widget setting's value, in the only two shapes any of them take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setting {
    Text(String),
    Count(u32),
}

impl Setting {
    fn into_text(self) -> Result<String> {
        match self {
            Setting::Text(text) => Ok(text),
            Setting::Count(count) => Ok(count.to_string()),
        }
    }

    fn into_count(self) -> Result<u32> {
        match self {
            Setting::Count(count) => Ok(count),
            Setting::Text(text) => text
                .trim()
                .parse()
                .with_context(|| format!("{text:?} is not a whole number")),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        Self::load_from(&config_path())
    }

    pub fn load_from(path: &Path) -> Self {
        let Ok(contents) = std::fs::read_to_string(path) else {
            return Self::default().migrated();
        };
        match toml::from_str::<Config>(&contents) {
            Ok(config) => config.migrated(),
            Err(e) => {
                tracing::warn!("ignoring unreadable config at {}: {e}", path.display());
                Self::default().migrated()
            }
        }
    }

    pub fn migrated(mut self) -> Self {
        if self.environments.is_empty() {
            self.environments.push(Environment {
                name: DEFAULT_ENVIRONMENT.to_string(),
                workspaces: Vec::new(),
                pinned: std::mem::take(&mut self.pinned),
                widgets: Vec::new(),
                folders: Vec::new(),
            });
        }
        self.pinned.clear();
        self
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&config_path())
    }

    /// Write the config out whole, or leave the old one untouched.
    ///
    /// The file is the only record of what the dock looks like, and a session
    /// that dies mid-write would otherwise leave half a TOML behind — which
    /// `load` cannot parse, so the next start silently falls back to defaults
    /// and the user's dock is gone. Writing beside the real file and renaming
    /// over it makes the swap atomic: a reader sees the old file or the new
    /// one, never a torn one.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("{} has no directory to write into", path.display()))?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;

        let body = toml::to_string_pretty(self)?;
        let scratch = parent.join(temporary_name(path));

        // The rename is the commit, so everything that can fail has to fail
        // first: the bytes are written and flushed to the disk before the old
        // file is replaced.
        let written = (|| -> std::io::Result<()> {
            let mut file = std::fs::File::create(&scratch)?;
            file.write_all(body.as_bytes())?;
            file.sync_all()
        })();

        if let Err(e) = written {
            let _ = std::fs::remove_file(&scratch);
            return Err(anyhow::Error::new(e)
                .context(format!("cannot write beside {}", path.display())));
        }

        if let Err(e) = std::fs::rename(&scratch, path) {
            let _ = std::fs::remove_file(&scratch);
            return Err(anyhow::Error::new(e)
                .context(format!("cannot replace {}", path.display())));
        }
        Ok(())
    }

    pub fn environment_for(&self, workspace: i32) -> &Environment {
        self.environments
            .iter()
            .find(|environment| environment.claims(workspace))
            .or_else(|| {
                self.environments
                    .iter()
                    .find(|environment| environment.is_catch_all())
            })
            .unwrap_or_else(|| &self.environments[0])
    }

    /// The environment on screen: the one chosen by hand, if it still exists,
    /// and otherwise the one this workspace calls for.
    ///
    /// A dock is not a workspace. Environments follow the workspace by default
    /// — that is what `workspaces` is for — but a key can put any of them on
    /// screen without moving anything, and that choice is what `chosen` holds.
    pub fn environment_shown(&self, chosen: Option<&str>, workspace: i32) -> &Environment {
        chosen
            .and_then(|name| self.environment_named(name))
            .unwrap_or_else(|| self.environment_for(workspace))
    }

    /// The environment a cycle key should land on, by name.
    ///
    /// Plain config order, wrapping round, catch-alls included: switching by
    /// hand needs no workspace to switch to, so an environment without one is
    /// as good a destination as any. One environment has nowhere to go.
    pub fn next_environment_after(&self, current: &str) -> Option<&Environment> {
        let position = self
            .environments
            .iter()
            .position(|environment| environment.name == current)
            .unwrap_or(0);

        self.environments
            .iter()
            .cycle()
            .skip(position + 1)
            .take(self.environments.len())
            .find(|environment| environment.name != current)
    }

    /// Whether some environment asked for this workspace by name.
    ///
    /// A workspace only a catch-all covers was never really claimed, so moving
    /// onto one is no reason to drop a dock the user chose by hand.
    pub fn workspace_is_claimed(&self, workspace: i32) -> bool {
        self.environments
            .iter()
            .any(|environment| environment.claims(workspace))
    }

    pub fn appearance(&self) -> Appearance {
        self.appearance.sanitised()
    }

    pub fn all_widgets(&self) -> Vec<String> {
        let mut seen = Vec::new();
        for environment in &self.environments {
            for widget in &environment.widgets {
                if !seen.contains(widget) {
                    seen.push(widget.clone());
                }
            }
        }
        seen
    }

    pub fn environment_named(&self, name: &str) -> Option<&Environment> {
        self.environments
            .iter()
            .find(|environment| environment.name == name)
    }

    pub fn pin(&mut self, name: &str, id: &str) {
        if let Some(environment) = self.environment_mut(name) {
            if !environment.pinned.iter().any(|existing| existing == id) {
                environment.pinned.push(id.to_string());
            }
        }
    }

    pub fn unpin(&mut self, name: &str, id: &str) {
        if let Some(environment) = self.environment_mut(name) {
            environment.pinned.retain(|existing| existing != id);
        }
    }

    fn environment_mut(&mut self, name: &str) -> Option<&mut Environment> {
        self.environments
            .iter_mut()
            .find(|environment| environment.name == name)
    }

    /// Replace the look of the dock, within the limits `sanitised` sets.
    ///
    /// Nothing reaches the field unsanitised, so a caller cannot put a
    /// magnification of 40 or an icon of 4000 px on disk and leave the next
    /// start to deal with it.
    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.appearance = appearance.sanitised();
    }

    pub fn set_widget_setting(&mut self, widget: &str, key: &str, value: Setting) -> Result<()> {
        self.widgets.set(widget, key, value)
    }

    /// The widgets one dock shows, in the order it shows them.
    pub fn set_environment_widgets(&mut self, name: &str, widgets: Vec<String>) -> Result<()> {
        let environment = self.named_mut(name)?;
        let mut kept: Vec<String> = Vec::new();
        for widget in widgets {
            let widget = widget.trim().to_string();
            if !widget.is_empty() && !kept.contains(&widget) {
                kept.push(widget);
            }
        }
        environment.widgets = kept;
        Ok(())
    }

    /// Put one dock's pins in a new order — and only in a new order.
    ///
    /// A reorder that could also add or drop a pin would make this method a
    /// second, sloppier `pin`/`unpin`: a GUI sending a stale list would quietly
    /// unpin whatever it had not heard about yet. So the set is fixed here. Ids
    /// the caller names are taken in the order given, anything it left out
    /// keeps its place at the end, and anything it invented is refused.
    pub fn reorder_pinned(&mut self, name: &str, order: Vec<String>) -> Result<()> {
        let environment = self.named_mut(name)?;
        if let Some(unknown) = order
            .iter()
            .find(|id| !environment.pinned.iter().any(|pinned| pinned == *id))
        {
            anyhow::bail!("{unknown} is not pinned in {name}");
        }

        let mut reordered: Vec<String> = Vec::with_capacity(environment.pinned.len());
        for id in order {
            if !reordered.contains(&id) {
                reordered.push(id);
            }
        }
        for id in &environment.pinned {
            if !reordered.contains(id) {
                reordered.push(id.clone());
            }
        }
        environment.pinned = reordered;
        Ok(())
    }

    /// Which workspaces a dock claims. An empty list makes it the catch-all.
    pub fn set_environment_workspaces(&mut self, name: &str, workspaces: Vec<i32>) -> Result<()> {
        let environment = self.named_mut(name)?;
        let mut claimed: Vec<i32> = workspaces.into_iter().filter(|index| *index >= 0).collect();
        claimed.sort_unstable();
        claimed.dedup();
        environment.workspaces = claimed;
        Ok(())
    }

    pub fn add_environment(&mut self, name: &str) -> Result<String> {
        let name = Self::usable_name(name)?;
        if self.environment_named(&name).is_some() {
            anyhow::bail!("a dock called {name} already exists");
        }
        self.environments.push(Environment {
            name: name.clone(),
            workspaces: Vec::new(),
            pinned: Vec::new(),
            widgets: Vec::new(),
            folders: Vec::new(),
        });
        Ok(name)
    }

    /// Remove a dock — unless it is the only one left.
    ///
    /// `migrated` guarantees at least one environment exists, and the whole
    /// daemon leans on it: `environment_for` ends in `environments[0]`. An
    /// empty list would panic there, so the last one cannot go.
    pub fn remove_environment(&mut self, name: &str) -> Result<()> {
        if self.environment_named(name).is_none() {
            anyhow::bail!("no dock called {name}");
        }
        if self.environments.len() == 1 {
            anyhow::bail!("the last dock cannot be removed");
        }
        self.environments
            .retain(|environment| environment.name != name);
        Ok(())
    }

    pub fn rename_environment(&mut self, from: &str, to: &str) -> Result<String> {
        let to = Self::usable_name(to)?;
        if self.environment_named(from).is_none() {
            anyhow::bail!("no dock called {from}");
        }
        if to != from && self.environment_named(&to).is_some() {
            anyhow::bail!("a dock called {to} already exists");
        }
        if let Some(environment) = self.environment_mut(from) {
            environment.name = to.clone();
        }
        Ok(to)
    }

    fn usable_name(name: &str) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("a dock needs a name");
        }
        Ok(name.to_string())
    }

    fn named_mut(&mut self, name: &str) -> Result<&mut Environment> {
        self.environment_mut(name)
            .ok_or_else(|| anyhow::anyhow!("no dock called {name}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(toml: &str) -> Config {
        toml::from_str::<Config>(toml).unwrap().migrated()
    }

    const THREE_ENVIRONMENTS: &str = r#"
        [[environments]]
        name = "Work"
        workspaces = [0, 1]

        [[environments]]
        name = "Personal"
        workspaces = [2]

        [[environments]]
        name = "Music"
        workspaces = [3]
    "#;

    #[test]
    fn cycling_lands_on_the_next_environment_in_config_order() {
        let config = config(THREE_ENVIRONMENTS);

        assert_eq!(
            config.next_environment_after("Work").map(|e| e.name.as_str()),
            Some("Personal")
        );
    }

    #[test]
    fn cycling_past_the_last_environment_comes_back_to_the_first() {
        let config = config(THREE_ENVIRONMENTS);

        assert_eq!(
            config.next_environment_after("Music").map(|e| e.name.as_str()),
            Some("Work")
        );
    }

    #[test]
    fn a_dock_with_no_workspace_of_its_own_is_still_somewhere_to_cycle_to() {
        let config = config(
            r#"
            [[environments]]
            name = "Work"
            workspaces = [0]

            [[environments]]
            name = "Everything else"
        "#,
        );

        assert_eq!(
            config.next_environment_after("Work").map(|e| e.name.as_str()),
            Some("Everything else"),
            "switching by hand needs no workspace to switch to"
        );
    }

    #[test]
    fn two_docks_sharing_one_workspace_can_still_be_cycled_between() {
        let config = config(
            r#"
            [[environments]]
            name = "Tudo"

            [[environments]]
            name = "Sistema"
        "#,
        );

        assert_eq!(
            config.next_environment_after("Tudo").map(|e| e.name.as_str()),
            Some("Sistema")
        );
        assert_eq!(
            config.next_environment_after("Sistema").map(|e| e.name.as_str()),
            Some("Tudo")
        );
    }

    #[test]
    fn one_environment_has_nowhere_to_cycle_to() {
        let config = config("[[environments]]\nname = \"Work\"\nworkspaces = [0, 1]\n");

        assert_eq!(config.next_environment_after("Work").map(|e| e.name.clone()), None);
    }

    #[test]
    fn a_migrated_phase_one_config_has_nowhere_to_cycle_to() {
        let config = config("pinned = [\"code\"]");
        let only = config.environments[0].name.clone();

        assert!(config.next_environment_after(&only).is_none());
    }

    #[test]
    fn a_dock_chosen_by_hand_is_shown_whatever_the_workspace_says() {
        let config = config(THREE_ENVIRONMENTS);

        assert_eq!(config.environment_shown(Some("Music"), 0).name, "Music");
    }

    #[test]
    fn with_nothing_chosen_the_workspace_decides_as_it_always_did() {
        let config = config(THREE_ENVIRONMENTS);

        assert_eq!(config.environment_shown(None, 3).name, "Music");
    }

    #[test]
    fn a_chosen_dock_that_was_renamed_away_falls_back_to_the_workspace() {
        let config = config(THREE_ENVIRONMENTS);

        assert_eq!(config.environment_shown(Some("Deleted"), 2).name, "Personal");
    }

    #[test]
    fn a_workspace_an_environment_asked_for_is_claimed() {
        let config = config(THREE_ENVIRONMENTS);

        assert!(config.workspace_is_claimed(2));
    }

    #[test]
    fn a_workspace_only_a_catch_all_covers_was_never_claimed() {
        let config = config(
            r#"
            [[environments]]
            name = "Work"
            workspaces = [0]

            [[environments]]
            name = "Everything else"
        "#,
        );

        assert!(config.workspace_is_claimed(0));
        assert!(
            !config.workspace_is_claimed(5),
            "a catch-all covers a workspace without asking for it"
        );
    }

    #[test]
    fn a_phase_one_config_becomes_a_single_catch_all_environment() {
        let migrated = config("pinned = [\"code\", \"discord\"]");

        assert_eq!(migrated.environments.len(), 1);
        assert_eq!(migrated.environments[0].name, DEFAULT_ENVIRONMENT);
        assert_eq!(migrated.environments[0].pinned, vec!["code", "discord"]);
        assert!(migrated.environments[0].is_catch_all());
    }

    #[test]
    fn an_absent_config_still_yields_one_usable_environment() {
        let fresh = Config::default().migrated();

        assert_eq!(fresh.environments.len(), 1);
        assert!(fresh.environments[0].pinned.is_empty());
    }

    #[test]
    fn a_migrated_config_no_longer_writes_the_old_top_level_key() {
        let migrated = config("pinned = [\"code\"]");

        let written = toml::to_string_pretty(&migrated).unwrap();

        let before_any_table: Vec<&str> = written
            .lines()
            .take_while(|line| !line.trim_start().starts_with('['))
            .collect();

        assert!(
            before_any_table
                .iter()
                .all(|line| !line.trim_start().starts_with("pinned")),
            "a top-level pinned key survived migration: {before_any_table:?}"
        );
        assert!(written.contains("pinned = [\"code\"]"));
    }

    fn two_environments() -> Config {
        config(
            "[[environments]]\n\
             name = \"Work\"\n\
             workspaces = [0, 1]\n\
             pinned = [\"code\"]\n\
             \n\
             [[environments]]\n\
             name = \"Personal\"\n\
             workspaces = [2, 3]\n\
             pinned = [\"discord\"]\n",
        )
    }

    #[test]
    fn widget_settings_have_usable_defaults_when_the_block_is_absent() {
        let settings = config("[[environments]]\nname = \"A\"\n").widgets;

        assert_eq!(settings.timer.minutes, 10);
        assert_eq!(settings.water.goal, 8);
        assert!(settings.date_is_unset());
    }

    #[test]
    fn a_widget_block_only_overrides_the_keys_it_names() {
        let settings = config("[widgets.timer]\nminutes = 25\n").widgets;

        assert_eq!(settings.timer.minutes, 25);
        assert_eq!(settings.water.goal, 8);
    }

    #[test]
    fn widget_settings_survive_a_round_trip_through_toml() {
        let original = config(
            "[widgets.countdown]\ndate = \"2026-12-25\"\nlabel = \"Christmas\"\n",
        );

        let written = toml::to_string_pretty(&original).unwrap();
        let reloaded = toml::from_str::<Config>(&written).unwrap().migrated();

        assert_eq!(reloaded.widgets, original.widgets);
    }

    #[test]
    fn a_config_with_no_appearance_block_still_has_a_theme() {
        let appearance = config("[[environments]]\nname = \"A\"\n").appearance();

        assert_eq!(appearance.theme, DEFAULT_THEME);
        assert_eq!(appearance.icon_size, 48);
    }

    #[test]
    fn a_theme_name_is_matched_regardless_of_how_it_was_typed() {
        let appearance = config("[appearance]\ntheme = \"  MidNight \"\n").appearance();

        assert_eq!(appearance.theme, "midnight");
    }

    #[test]
    fn an_absurd_icon_size_is_brought_back_into_range() {
        assert_eq!(
            config("[appearance]\nicon_size = 4000\n").appearance().icon_size,
            MAX_ICON_SIZE
        );
        assert_eq!(
            config("[appearance]\nicon_size = 2\n").appearance().icon_size,
            MIN_ICON_SIZE
        );
    }

    #[test]
    fn auto_hide_is_off_unless_it_is_asked_for() {
        assert!(!config("[appearance]\ntheme = \"native\"\n").appearance().auto_hide);
        assert!(config("[appearance]\nauto_hide = true\n").appearance().auto_hide);
    }

    #[test]
    fn magnification_is_kept_inside_a_range_that_still_looks_like_a_dock() {
        assert_eq!(config("[appearance]\nmagnification = 99.0\n").appearance().magnification, 2.5);
        assert_eq!(config("[appearance]\nmagnification = 0.2\n").appearance().magnification, 1.0);
    }

    #[test]
    fn magnification_of_one_is_how_it_is_turned_off() {
        assert_eq!(config("[appearance]\nmagnification = 1.0\n").appearance().magnification, 1.0);
    }

    #[test]
    fn a_nonsense_magnification_turns_the_lens_off_rather_than_breaking_layout() {
        assert_eq!(config("[appearance]\nmagnification = nan\n").appearance().magnification, 1.0);
    }

    #[test]
    fn appearance_survives_a_round_trip_through_toml() {
        let original = config("[appearance]\ntheme = \"paper\"\nicon_size = 40\n");

        let written = toml::to_string_pretty(&original).unwrap();
        let reloaded = toml::from_str::<Config>(&written).unwrap().migrated();

        assert_eq!(reloaded.appearance(), original.appearance());
    }

    #[test]
    fn a_workspace_resolves_to_the_environment_that_claims_it() {
        let config = two_environments();

        assert_eq!(config.environment_for(0).name, "Work");
        assert_eq!(config.environment_for(1).name, "Work");
        assert_eq!(config.environment_for(2).name, "Personal");
        assert_eq!(config.environment_for(3).name, "Personal");
    }

    #[test]
    fn an_unclaimed_workspace_falls_back_to_the_catch_all_environment() {
        let config = config(
            "[[environments]]\n\
             name = \"Work\"\n\
             workspaces = [0]\n\
             \n\
             [[environments]]\n\
             name = \"Everything else\"\n",
        );

        assert_eq!(config.environment_for(7).name, "Everything else");
    }

    #[test]
    fn an_unclaimed_workspace_with_no_catch_all_falls_back_to_the_first() {
        let config = two_environments();

        assert_eq!(config.environment_for(9).name, "Work");
    }

    #[test]
    fn pinning_lands_in_the_dock_that_is_on_screen() {
        let mut config = two_environments();

        config.pin("Personal", "spotify");

        assert_eq!(
            config.environment_named("Personal").unwrap().pinned,
            vec!["discord", "spotify"]
        );
        assert_eq!(config.environment_named("Work").unwrap().pinned, vec!["code"]);
    }

    #[test]
    fn unpinning_only_touches_the_dock_that_is_on_screen() {
        let mut config = config(
            "[[environments]]\n\
             name = \"Work\"\n\
             workspaces = [0]\n\
             pinned = [\"code\"]\n\
             \n\
             [[environments]]\n\
             name = \"Personal\"\n\
             workspaces = [1]\n\
             pinned = [\"code\"]\n",
        );

        config.unpin("Work", "code");

        assert!(config.environment_named("Work").unwrap().pinned.is_empty());
        assert_eq!(config.environment_named("Personal").unwrap().pinned, vec!["code"]);
    }

    #[test]
    fn pinning_the_same_app_twice_in_one_environment_keeps_one_entry() {
        let mut config = two_environments();

        config.pin("Work", "code");

        assert_eq!(config.environment_named("Work").unwrap().pinned, vec!["code"]);
    }

    /// A directory of this test's own, so nothing has to touch the real
    /// `XDG_CONFIG_HOME` — tests share a process, and an env var one of them
    /// sets is an env var all the others see.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(what: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "doca-config-test-{what}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self) -> PathBuf {
            self.0.join("doca/config.toml")
        }

        fn leftovers(&self) -> Vec<String> {
            let Ok(entries) = std::fs::read_dir(self.0.join("doca")) else {
                return Vec::new();
            };
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .filter(|name| name != "config.toml")
                .collect()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn two_writes_in_a_row_do_not_lose_the_first() {
        let scratch = Scratch::new("two-writes");
        let path = scratch.file();

        let mut config = two_environments();
        config.set_appearance(Appearance {
            theme: "paper".to_string(),
            ..Appearance::default()
        });
        config.save_to(&path).unwrap();

        // The second writer starts from what the first one left behind, which
        // is the whole point of the daemon holding one config and saving it
        // whole: a write that rebuilt the file from defaults would silently
        // undo the theme that had just been set.
        let mut second = Config::load_from(&path);
        second.add_environment("Music").unwrap();
        second.save_to(&path).unwrap();

        let reloaded = Config::load_from(&path);
        assert_eq!(reloaded.appearance().theme, "paper", "the first write survived");
        assert!(
            reloaded.environment_named("Music").is_some(),
            "the second write landed"
        );
        assert_eq!(reloaded.environments.len(), 3);
    }

    #[test]
    fn an_appearance_nobody_should_have_asked_for_is_sanitised_before_it_reaches_disk() {
        let scratch = Scratch::new("sanitised");
        let path = scratch.file();

        let mut config = two_environments();
        config.set_appearance(Appearance {
            theme: "  MidNight ".to_string(),
            icon_size: 4000,
            magnification: f64::NAN,
            show_trash: true,
            auto_hide: true,
        });
        config.save_to(&path).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            !written.contains("4000") && !written.contains("nan"),
            "the raw values were written out: {written}"
        );

        let reloaded = Config::load_from(&path).appearance();
        assert_eq!(reloaded.theme, "midnight");
        assert_eq!(reloaded.icon_size, MAX_ICON_SIZE);
        assert_eq!(reloaded.magnification, MIN_MAGNIFICATION);
    }

    #[test]
    fn a_saved_config_leaves_nothing_beside_itself() {
        let scratch = Scratch::new("atomic");
        let path = scratch.file();

        two_environments().save_to(&path).unwrap();
        two_environments().save_to(&path).unwrap();

        assert!(path.exists());
        assert!(
            scratch.leftovers().is_empty(),
            "a half-written file was left behind: {:?}",
            scratch.leftovers()
        );
    }

    #[test]
    fn a_write_over_a_config_that_is_already_there_replaces_it_whole() {
        let scratch = Scratch::new("replace");
        let path = scratch.file();

        let mut first = two_environments();
        first.add_environment("Music").unwrap();
        first.save_to(&path).unwrap();

        let mut second = Config::load_from(&path);
        second.remove_environment("Music").unwrap();
        second.save_to(&path).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            !written.contains("Music"),
            "the old file showed through the new one: {written}"
        );
    }

    #[test]
    fn a_reorder_cannot_add_a_pin_that_was_never_there() {
        let mut config = two_environments();

        let refused = config.reorder_pinned("Work", vec!["gimp".to_string()]);

        assert!(refused.is_err());
        assert_eq!(config.environment_named("Work").unwrap().pinned, vec!["code"]);
    }

    #[test]
    fn a_reorder_that_forgets_a_pin_keeps_it_rather_than_dropping_it() {
        let mut config = config(
            "[[environments]]\n\
             name = \"Work\"\n\
             pinned = [\"code\", \"discord\", \"firefox\"]\n",
        );

        config
            .reorder_pinned("Work", vec!["firefox".to_string()])
            .unwrap();

        assert_eq!(
            config.environment_named("Work").unwrap().pinned,
            vec!["firefox", "code", "discord"],
            "a stale list from a window must not unpin what it had not heard about"
        );
    }

    #[test]
    fn the_last_dock_cannot_be_removed() {
        let mut config = config("[[environments]]\nname = \"Only\"\n");

        assert!(config.remove_environment("Only").is_err());
        assert_eq!(config.environments.len(), 1, "something still has to be on screen");
    }

    #[test]
    fn a_dock_cannot_take_a_name_another_one_already_has() {
        let mut config = two_environments();

        assert!(config.add_environment("Work").is_err());
        assert!(config.rename_environment("Personal", "Work").is_err());
        assert!(config.add_environment("   ").is_err(), "a dock needs a name");
    }

    #[test]
    fn renaming_a_dock_to_itself_is_allowed_and_changes_nothing() {
        let mut config = two_environments();

        assert_eq!(config.rename_environment("Work", " Work ").unwrap(), "Work");
        assert_eq!(config.environments.len(), 2);
    }

    #[test]
    fn claimed_workspaces_are_tidied_rather_than_taken_as_typed() {
        let mut config = two_environments();

        config
            .set_environment_workspaces("Work", vec![3, 1, 3, -2, 1])
            .unwrap();

        assert_eq!(
            config.environment_named("Work").unwrap().workspaces,
            vec![1, 3],
            "duplicates and workspaces that cannot exist are no one's business downstream"
        );
    }

    #[test]
    fn a_dock_can_be_made_a_catch_all_by_claiming_nothing() {
        let mut config = two_environments();

        config.set_environment_workspaces("Work", Vec::new()).unwrap();

        assert!(config.environment_named("Work").unwrap().is_catch_all());
    }

    #[test]
    fn a_widget_list_keeps_its_order_and_drops_repeats() {
        let mut config = two_environments();

        config
            .set_environment_widgets(
                "Work",
                vec!["clock".into(), "cpu".into(), "clock".into(), "  ".into()],
            )
            .unwrap();

        assert_eq!(
            config.environment_named("Work").unwrap().widgets,
            vec!["clock", "cpu"]
        );
    }

    #[test]
    fn writing_a_setting_on_a_dock_that_is_gone_says_so() {
        let mut config = two_environments();

        assert!(config.set_environment_widgets("Gone", vec![]).is_err());
        assert!(config.set_environment_workspaces("Gone", vec![1]).is_err());
        assert!(config.reorder_pinned("Gone", vec![]).is_err());
        assert!(config.rename_environment("Gone", "Here").is_err());
        assert!(config.remove_environment("Gone").is_err());
    }

    #[test]
    fn a_widget_setting_is_written_by_name() {
        let mut config = two_environments();

        config
            .set_widget_setting("timer", "minutes", Setting::Count(25))
            .unwrap();
        config
            .set_widget_setting("countdown", "date", Setting::Text("2026-12-25".into()))
            .unwrap();

        assert_eq!(config.widgets.timer.minutes, 25);
        assert_eq!(config.widgets.countdown.date, "2026-12-25");
    }

    #[test]
    fn a_widget_setting_nobody_has_is_refused_rather_than_ignored() {
        let mut config = two_environments();

        assert!(config
            .set_widget_setting("timer", "seconds", Setting::Count(30))
            .is_err());
        assert!(config
            .set_widget_setting("clock", "minutes", Setting::Count(30))
            .is_err());
    }

    #[test]
    fn a_widget_setting_outside_its_range_is_brought_back_in() {
        let mut config = two_environments();

        config
            .set_widget_setting("timer", "minutes", Setting::Count(0))
            .unwrap();
        config
            .set_widget_setting("water", "goal", Setting::Count(9_000))
            .unwrap();

        assert_eq!(config.widgets.timer.minutes, MIN_TIMER_MINUTES);
        assert_eq!(config.widgets.water.goal, MAX_WATER_GOAL);
    }

    #[test]
    fn a_count_typed_as_text_is_still_a_count() {
        let mut config = two_environments();

        config
            .set_widget_setting("timer", "minutes", Setting::Text(" 45 ".into()))
            .unwrap();

        assert_eq!(config.widgets.timer.minutes, 45);
        assert!(config
            .set_widget_setting("timer", "minutes", Setting::Text("soon".into()))
            .is_err());
    }

    #[test]
    fn a_round_trip_through_toml_preserves_every_environment() {
        let original = two_environments();

        let written = toml::to_string_pretty(&original).unwrap();
        let reloaded = toml::from_str::<Config>(&written).unwrap().migrated();

        assert_eq!(reloaded.environments, original.environments);
    }
}
