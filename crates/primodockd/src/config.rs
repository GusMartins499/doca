use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub pinned: Vec<String>,
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
            return Self::default();
        };
        match toml::from_str(&contents) {
            Ok(config) => config,
            Err(e) => {
                tracing::warn!("ignoring unreadable config at {}: {e}", path.display());
                Self::default()
            }
        }
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

    pub fn pin(&mut self, id: &str) {
        if !self.pinned.iter().any(|existing| existing == id) {
            self.pinned.push(id.to_string());
        }
    }

    pub fn unpin(&mut self, id: &str) {
        self.pinned.retain(|existing| existing != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinning_the_same_app_twice_keeps_one_entry() {
        let mut config = Config::default();

        config.pin("code");
        config.pin("code");

        assert_eq!(config.pinned, vec!["code"]);
    }

    #[test]
    fn pinning_preserves_the_order_apps_were_added_in() {
        let mut config = Config::default();

        config.pin("brave-browser");
        config.pin("code");
        config.pin("discord");

        assert_eq!(config.pinned, vec!["brave-browser", "code", "discord"]);
    }

    #[test]
    fn unpinning_leaves_the_remaining_order_untouched() {
        let mut config = Config::default();
        config.pin("brave-browser");
        config.pin("code");
        config.pin("discord");

        config.unpin("code");

        assert_eq!(config.pinned, vec!["brave-browser", "discord"]);
    }

    #[test]
    fn a_config_file_that_cannot_be_parsed_falls_back_to_an_empty_dock() {
        let parsed: Result<Config, _> = toml::from_str("pinned = \"not a list\"");

        assert!(parsed.is_err());
    }
}
