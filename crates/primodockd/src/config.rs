use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_ENVIRONMENT: &str = "Default";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    pub name: String,
    #[serde(default)]
    pub workspaces: Vec<i32>,
    #[serde(default)]
    pub pinned: Vec<String>,
    #[serde(default)]
    pub widgets: Vec<String>,
}

impl Environment {
    pub fn claims(&self, workspace: i32) -> bool {
        self.workspaces.contains(&workspace)
    }

    pub fn is_catch_all(&self) -> bool {
        self.workspaces.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
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
    base.join("primodock/config.toml")
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        let Ok(contents) = std::fs::read_to_string(&path) else {
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
            });
        }
        self.pinned.clear();
        self
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        std::fs::write(&path, toml::to_string_pretty(self)?)
            .with_context(|| format!("cannot write {}", path.display()))?;
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

    pub fn pin(&mut self, workspace: i32, id: &str) {
        let name = self.environment_for(workspace).name.clone();
        if let Some(environment) = self.environment_mut(&name) {
            if !environment.pinned.iter().any(|existing| existing == id) {
                environment.pinned.push(id.to_string());
            }
        }
    }

    pub fn unpin(&mut self, workspace: i32, id: &str) {
        let name = self.environment_for(workspace).name.clone();
        if let Some(environment) = self.environment_mut(&name) {
            environment.pinned.retain(|existing| existing != id);
        }
    }

    fn environment_mut(&mut self, name: &str) -> Option<&mut Environment> {
        self.environments
            .iter_mut()
            .find(|environment| environment.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(toml: &str) -> Config {
        toml::from_str::<Config>(toml).unwrap().migrated()
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

        let first_line = written.lines().find(|line| !line.trim().is_empty()).unwrap();
        assert_eq!(first_line, "[[environments]]");
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
    fn pinning_lands_in_the_environment_that_owns_the_workspace() {
        let mut config = two_environments();

        config.pin(2, "spotify");

        assert_eq!(
            config.environment_named("Personal").unwrap().pinned,
            vec!["discord", "spotify"]
        );
        assert_eq!(config.environment_named("Work").unwrap().pinned, vec!["code"]);
    }

    #[test]
    fn unpinning_only_touches_the_environment_that_owns_the_workspace() {
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

        config.unpin(0, "code");

        assert!(config.environment_named("Work").unwrap().pinned.is_empty());
        assert_eq!(config.environment_named("Personal").unwrap().pinned, vec!["code"]);
    }

    #[test]
    fn pinning_the_same_app_twice_in_one_environment_keeps_one_entry() {
        let mut config = two_environments();

        config.pin(0, "code");

        assert_eq!(config.environment_named("Work").unwrap().pinned, vec!["code"]);
    }

    #[test]
    fn a_round_trip_through_toml_preserves_every_environment() {
        let original = two_environments();

        let written = toml::to_string_pretty(&original).unwrap();
        let reloaded = toml::from_str::<Config>(&written).unwrap().migrated();

        assert_eq!(reloaded.environments, original.environments);
    }
}
