use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_ENVIRONMENT: &str = "Default";

// The look of the dock is a contract between three crates, so its names and
// limits live in the one they all depend on.
pub use doca_ipc::{
    DEFAULT_THEME, MAX_ICON_SIZE, MAX_MAGNIFICATION, MAX_TIMER_MINUTES, MAX_WATER_GOAL,
    MIN_ICON_SIZE, MIN_MAGNIFICATION, MIN_TIMER_MINUTES, MIN_WATER_GOAL,
};

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
    /// The icon theme to use instead of the system's. Empty follows the system.
    ///
    /// Empty rather than `Option` on purpose: these three cross the bus inside
    /// `Appearance`, and an absent value there would mean a nullable field in
    /// the signature for no gain — "follow the system" and "no override" are
    /// the same thing, and the empty string says it.
    #[serde(default)]
    pub icon_theme: String,
    #[serde(default)]
    pub gtk_theme: String,
    #[serde(default)]
    pub cursor_theme: String,
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
            icon_theme: String::new(),
            gtk_theme: String::new(),
            cursor_theme: String::new(),
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
            icon_theme: self.icon_theme.trim().to_string(),
            gtk_theme: self.gtk_theme.trim().to_string(),
            cursor_theme: self.cursor_theme.trim().to_string(),
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
    /// The paper it is written on, one of `doca_ipc::note_colour::ALL`.
    #[serde(default = "default_note_colour")]
    pub colour: String,
}

fn default_note_colour() -> String {
    doca_ipc::note_colour::DEFAULT.to_string()
}

impl Default for NoteSettings {
    fn default() -> Self {
        Self {
            text: "a note lives here".to_string(),
            colour: default_note_colour(),
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
    /// Millilitres for the day.
    #[serde(default = "default_water_goal")]
    pub goal: u32,
    /// Millilitres in one go — the bottle on the desk, not an abstract glass.
    #[serde(default = "default_water_bottle")]
    pub bottle: u32,
}

fn default_water_goal() -> u32 {
    2000
}

fn default_water_bottle() -> u32 {
    500
}

impl Default for WaterSettings {
    fn default() -> Self {
        Self {
            goal: default_water_goal(),
            bottle: default_water_bottle(),
        }
    }
}

impl WaterSettings {
    /// A goal written when this counted glasses, read as the millilitres it
    /// always meant.
    ///
    /// The old range was 1 to 64 glasses and the new one is 500 to 6000
    /// millilitres: they do not overlap, which is the whole of why this is
    /// safe to run on every load. A number below the new floor cannot be a
    /// goal somebody set in the new unit, so it can only be an old one — and
    /// once carried over it is above the floor and this never sees it again.
    ///
    /// Eight glasses was the old default and 2000ml is the new one, so a
    /// config nobody ever edited comes out meaning exactly what it meant.
    fn carried_over(mut self) -> Self {
        if self.goal < MIN_WATER_GOAL {
            self.goal = (self.goal * doca_ipc::GLASS).clamp(MIN_WATER_GOAL, MAX_WATER_GOAL);
        }
        self
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
                colour: doca_ipc::note_colour::resolve(self.note.colour.trim()).to_string(),
            },
            timer: TimerSettings {
                minutes: self.timer.minutes.clamp(MIN_TIMER_MINUTES, MAX_TIMER_MINUTES),
            },
            water: WaterSettings {
                goal: self.water.goal.clamp(MIN_WATER_GOAL, MAX_WATER_GOAL),
                bottle: self
                    .water
                    .bottle
                    .clamp(doca_ipc::MIN_WATER_BOTTLE, doca_ipc::MAX_WATER_BOTTLE),
            },
        }
    }

    /// The keys a caller is allowed to name, and nothing else.
    ///
    /// A typo in a widget id or a key is told to the caller rather than
    /// written to disk — a silently ignored setting is the kind of thing the
    /// user reads as "the dock is broken".
    pub fn set(&mut self, widget: &str, key: &str, value: Setting) -> Result<()> {
        use doca_ipc::widget_key as k;

        match (widget, key) {
            (k::COUNTDOWN, k::DATE) => self.countdown.date = value.into_text()?,
            (k::COUNTDOWN, k::LABEL) => self.countdown.label = value.into_text()?,
            (k::NOTE, k::TEXT) => self.note.text = value.into_text()?,
            (k::NOTE, k::COLOUR) => self.note.colour = value.into_text()?,
            (k::TIMER, k::MINUTES) => self.timer.minutes = value.into_count()?,
            (k::WATER, k::GOAL) => self.water.goal = value.into_count()?,
            (k::WATER, k::BOTTLE) => self.water.bottle = value.into_count()?,
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
        self.widgets.water = std::mem::take(&mut self.widgets.water).carried_over();
        self
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&config_path())
    }

    /// Write the config out, or leave the old one untouched.
    ///
    /// The file is the only record of what the dock looks like, so the write
    /// goes through [`crate::atomic::write`] — which is also what the state
    /// file uses, and says there why.
    ///
    /// And it is written *onto* the file that is there rather than over it:
    /// this is the file the project tells people to edit by hand, so the
    /// comments and the order they put in it outlive a widget counting a
    /// glass of water. [`crate::merge::keeping`] is the whole of that, and
    /// says there what it costs.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        let fresh = toml::to_string_pretty(self)?;
        let existing = std::fs::read_to_string(path).unwrap_or_default();
        crate::atomic::write(path, &crate::merge::keeping(&existing, &fresh))
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

    /// Pin an app to a dock, at the end of what it already pins.
    ///
    /// Naming a dock that does not exist is an error rather than a no-op: a
    /// window editing a dock that was renamed under it would otherwise get a
    /// success for a write that went nowhere, and show a pin the file does not
    /// have.
    pub fn pin(&mut self, name: &str, id: &str) -> Result<()> {
        let id = Self::usable_id(id)?;
        let environment = self.named_mut(name)?;
        if !environment.pinned.contains(&id) {
            environment.pinned.push(id);
        }
        Ok(())
    }

    pub fn unpin(&mut self, name: &str, id: &str) -> Result<()> {
        let id = Self::usable_id(id)?;
        let environment = self.named_mut(name)?;
        environment.pinned.retain(|existing| *existing != id);
        Ok(())
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

    /// Put the docks themselves in a new order — and only in a new order.
    ///
    /// The order of this list *is* the cycle: `next_environment_after` walks
    /// it and wraps round, so this is the only thing that decides which dock a
    /// key takes you to next. It was also the one piece of the config a window
    /// could not reach, which left the cycle order editable by hand and
    /// nothing else.
    ///
    /// The same contract as [`Self::reorder_pinned`], for the same reason: a
    /// reorder that could also add or drop a dock would be a sloppier
    /// `add_environment`/`remove_environment`, and a window sending a list it
    /// had drawn before a rename would quietly delete a dock. Names given are
    /// taken in the order given, anything left out keeps its place at the end,
    /// and anything invented is refused.
    pub fn reorder_environments(&mut self, order: Vec<String>) -> Result<()> {
        if let Some(unknown) = order
            .iter()
            .find(|name| self.environment_named(name).is_none())
        {
            anyhow::bail!("no dock called {unknown}");
        }

        let mut reordered: Vec<Environment> = Vec::with_capacity(self.environments.len());
        for name in order {
            if reordered.iter().any(|dock| dock.name == name) {
                continue;
            }
            if let Some(at) = self.environments.iter().position(|dock| dock.name == name) {
                reordered.push(self.environments.remove(at));
            }
        }
        reordered.append(&mut self.environments);
        self.environments = reordered;
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

    /// An app id with nothing but whitespace in it pins nothing, and would
    /// sit in the file for ever looking like a pin that failed to draw.
    fn usable_id(id: &str) -> Result<String> {
        let id = id.trim();
        if id.is_empty() {
            anyhow::bail!("a pin needs an application");
        }
        Ok(id.to_string())
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

    /// The other half of `doca_ipc::widget_key::ALL`: a window builds its
    /// controls from that list, so every pair in it has to be a pair this
    /// accepts. A key renamed on one side alone fails here.
    #[test]
    fn every_setting_a_window_is_offered_is_a_setting_that_can_be_written() {
        for (widget, key) in doca_ipc::widget_key::ALL {
            let mut settings = WidgetSettings::default();

            let written = settings
                .set(widget, key, Setting::Text("1".to_string()))
                .or_else(|_| settings.set(widget, key, Setting::Count(1)));

            assert!(written.is_ok(), "{widget}.{key} was refused");
        }
    }

    #[test]
    fn widget_settings_have_usable_defaults_when_the_block_is_absent() {
        let settings = config("[[environments]]\nname = \"A\"\n").widgets;

        assert_eq!(settings.timer.minutes, 10);
        assert_eq!(settings.water.goal, 2000);
        assert_eq!(settings.water.bottle, 500);
        assert!(settings.date_is_unset());
    }

    /// The unit changed under people who already had a goal, so the goal has
    /// to change with it rather than be clamped into nonsense — a saved `8`
    /// read as millilitres would be a goal of eight millilitres.
    #[test]
    fn a_goal_written_in_glasses_is_read_as_the_millilitres_it_meant() {
        let carried = config("[widgets.water]\ngoal = 8\n").widgets;

        assert_eq!(carried.water.goal, 2000, "eight glasses is two litres");
    }

    /// And the half of it that matters more: it cannot happen twice. The old
    /// range stopped at 64 and the new one starts at 500, so a goal that has
    /// already been carried over can never be mistaken for one that has not.
    #[test]
    fn carrying_a_goal_over_a_second_time_does_nothing() {
        let once = config("[widgets.water]\ngoal = 8\n").widgets.water;

        let twice = once.clone().carried_over();

        assert_eq!(twice.goal, once.goal);
        for glasses in 1..=64u32 {
            let carried = WaterSettings { goal: glasses, bottle: 500 }.carried_over();
            assert_eq!(
                carried.clone().carried_over().goal,
                carried.goal,
                "{glasses} glasses moved twice"
            );
            assert!(carried.goal >= MIN_WATER_GOAL);
        }
    }

    /// A goal somebody set in the new unit is left exactly alone, including
    /// the smallest one the range allows — which is the number the carry-over
    /// has to stop at.
    #[test]
    fn a_goal_already_in_millilitres_is_not_touched() {
        for goal in [MIN_WATER_GOAL, 1500, 2000, MAX_WATER_GOAL] {
            let settings = WaterSettings { goal, bottle: 500 };

            assert_eq!(settings.carried_over().goal, goal);
        }
    }

    #[test]
    fn a_widget_block_only_overrides_the_keys_it_names() {
        let settings = config("[widgets.timer]\nminutes = 25\n").widgets;

        assert_eq!(settings.timer.minutes, 25);
        assert_eq!(settings.water.goal, 2000);
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

        config.pin("Personal", "spotify").unwrap();

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

        config.unpin("Work", "code").unwrap();

        assert!(config.environment_named("Work").unwrap().pinned.is_empty());
        assert_eq!(config.environment_named("Personal").unwrap().pinned, vec!["code"]);
    }

    #[test]
    fn pinning_to_a_dock_that_does_not_exist_is_refused_rather_than_ignored() {
        let mut config = two_environments();

        let refused = config.pin("Renamed yesterday", "code");

        assert!(
            refused.is_err(),
            "a window editing a dock that moved would get a success for nothing"
        );
        assert_eq!(config.environment_named("Work").unwrap().pinned, vec!["code"]);
    }

    #[test]
    fn a_pin_with_no_application_in_it_is_refused() {
        let mut config = two_environments();

        assert!(config.pin("Work", "   ").is_err());
    }

    #[test]
    fn pinning_the_same_app_twice_in_one_environment_keeps_one_entry() {
        let mut config = two_environments();

        config.pin("Work", "code").unwrap();

        assert_eq!(config.environment_named("Work").unwrap().pinned, vec!["code"]);
    }

    /// A directory of this test's own, so nothing has to touch the real
    /// `XDG_CONFIG_HOME` — tests share a process, and an env var one of them
    /// sets is an env var all the others see.
    use std::sync::atomic::{AtomicU64, Ordering};

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
            icon_theme: "  Papirus-Dark  ".to_string(),
            gtk_theme: String::new(),
            cursor_theme: String::new(),
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
        assert_eq!(
            reloaded.icon_theme, "Papirus-Dark",
            "a theme name is looked up verbatim, so stray spaces would simply miss"
        );
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

    fn three_docks() -> Config {
        config(
            "[[environments]]\nname = \"Work\"\n\n\
             [[environments]]\nname = \"Home\"\n\n\
             [[environments]]\nname = \"Games\"\n",
        )
    }

    fn order(config: &Config) -> Vec<String> {
        config
            .environments
            .iter()
            .map(|dock| dock.name.clone())
            .collect()
    }

    /// The order of the list is the order of the cycle, so this is the one
    /// thing that decides where a key takes you next.
    #[test]
    fn docks_take_the_order_they_were_given() {
        let mut config = three_docks();

        config
            .reorder_environments(["Games", "Work", "Home"].map(str::to_string).to_vec())
            .unwrap();

        assert_eq!(order(&config), ["Games", "Work", "Home"]);
    }

    #[test]
    fn reordering_docks_keeps_everything_each_one_held() {
        let mut config = config(
            "[[environments]]\nname = \"Work\"\npinned = [\"code\"]\nworkspaces = [0, 1]\n\n\
             [[environments]]\nname = \"Home\"\nwidgets = [\"clock\"]\n",
        );

        config
            .reorder_environments(vec!["Home".to_string(), "Work".to_string()])
            .unwrap();

        let work = config.environment_named("Work").unwrap();
        assert_eq!(work.pinned, vec!["code"]);
        assert_eq!(work.workspaces, vec![0, 1]);
        assert_eq!(config.environment_named("Home").unwrap().widgets, vec!["clock"]);
    }

    #[test]
    fn a_reorder_cannot_invent_a_dock() {
        let mut config = three_docks();

        let refused = config.reorder_environments(vec!["Studio".to_string()]);

        assert!(refused.is_err());
        assert_eq!(order(&config), ["Work", "Home", "Games"], "a refusal moved something");
    }

    /// The reason the set is fixed here: a window that drew its list before a
    /// rename would otherwise delete the dock it had not heard about.
    #[test]
    fn a_reorder_that_forgets_a_dock_keeps_it_rather_than_dropping_it() {
        let mut config = three_docks();

        config
            .reorder_environments(vec!["Games".to_string()])
            .unwrap();

        assert_eq!(order(&config), ["Games", "Work", "Home"]);
    }

    #[test]
    fn a_dock_named_twice_is_placed_once() {
        let mut config = three_docks();

        config
            .reorder_environments(["Home", "Home", "Work"].map(str::to_string).to_vec())
            .unwrap();

        assert_eq!(order(&config), ["Home", "Work", "Games"]);
    }

    /// The cycle reads this list, so the two have to agree after a reorder.
    #[test]
    fn the_cycle_follows_the_order_it_was_put_in() {
        let mut config = three_docks();

        config
            .reorder_environments(["Games", "Work", "Home"].map(str::to_string).to_vec())
            .unwrap();

        assert_eq!(
            config.next_environment_after("Games").map(|dock| dock.name.as_str()),
            Some("Work")
        );
        assert_eq!(
            config.next_environment_after("Home").map(|dock| dock.name.as_str()),
            Some("Games"),
            "the cycle wraps round the new order, not the old one"
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
