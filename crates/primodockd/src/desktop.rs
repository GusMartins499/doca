use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopEntry {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub exec: String,
    pub startup_wm_class: Option<String>,
    pub no_display: bool,
    pub path: PathBuf,
}

fn normalize(value: &str) -> String {
    value.trim().to_lowercase().replace(' ', "-")
}

fn last_segment(id: &str) -> &str {
    id.rsplit('.').next().unwrap_or(id)
}

fn strip_field_codes(exec: &str) -> String {
    exec.split_whitespace()
        .filter(|token| !matches!(*token, "%U" | "%u" | "%F" | "%f" | "%i" | "%c" | "%k"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn exec_binary(exec: &str) -> Option<String> {
    let first = exec.split_whitespace().next()?;
    let trimmed = first.trim_matches('"');
    Path::new(trimmed)
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
}

pub fn parse_entry(id: &str, contents: &str, path: PathBuf) -> Option<DesktopEntry> {
    let mut in_group = false;
    let mut fields: HashMap<String, String> = HashMap::new();

    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            continue;
        }
        if !in_group || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            fields
                .entry(key.trim().to_string())
                .or_insert_with(|| value.trim().to_string());
        }
    }

    if fields.get("Type").map(String::as_str) != Some("Application") {
        return None;
    }
    if fields.get("Hidden").map(String::as_str) == Some("true") {
        return None;
    }

    let name = fields.get("Name")?.clone();
    let exec = fields.get("Exec").map(|e| strip_field_codes(e))?;

    Some(DesktopEntry {
        id: id.to_string(),
        name,
        icon: fields
            .get("Icon")
            .cloned()
            .unwrap_or_else(|| "application-x-executable".to_string()),
        exec,
        startup_wm_class: fields.get("StartupWMClass").cloned(),
        no_display: fields.get("NoDisplay").map(String::as_str) == Some("true"),
        path,
    })
}

#[derive(Debug, Default)]
pub struct DesktopIndex {
    entries: Vec<DesktopEntry>,
}

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(&home).join(".local/share/applications"));
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    for dir in data_dirs.split(':') {
        if !dir.is_empty() {
            dirs.push(PathBuf::from(dir).join("applications"));
        }
    }
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(&home).join(".local/share/flatpak/exports/share/applications"));
    }
    dirs
}

impl DesktopIndex {
    pub fn from_entries(entries: Vec<DesktopEntry>) -> Self {
        Self { entries }
    }

    pub fn load() -> Self {
        let mut seen: HashMap<String, DesktopEntry> = HashMap::new();
        for dir in search_dirs() {
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            for file in read.flatten() {
                let path = file.path();
                if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                    continue;
                }
                let Some(id) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
                    continue;
                };
                if seen.contains_key(&id) {
                    continue;
                }
                let Ok(contents) = std::fs::read_to_string(&path) else {
                    continue;
                };
                if let Some(entry) = parse_entry(&id, &contents, path) {
                    seen.insert(id, entry);
                }
            }
        }
        Self {
            entries: seen.into_values().collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&DesktopEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    pub fn match_window(&self, app_id: &str) -> Option<&DesktopEntry> {
        if app_id.is_empty() {
            return None;
        }
        let wanted = normalize(app_id);

        self.by_startup_wm_class(app_id)
            .or_else(|| self.by_exact_id(&wanted))
            .or_else(|| self.by_trailing_id_segment(&wanted))
            .or_else(|| self.by_exec_binary(&wanted))
    }

    fn by_startup_wm_class(&self, app_id: &str) -> Option<&DesktopEntry> {
        self.entries.iter().find(|entry| {
            entry
                .startup_wm_class
                .as_deref()
                .is_some_and(|class| class.eq_ignore_ascii_case(app_id))
        })
    }

    fn by_exact_id(&self, wanted: &str) -> Option<&DesktopEntry> {
        self.entries
            .iter()
            .find(|entry| normalize(&entry.id) == wanted)
    }

    fn by_trailing_id_segment(&self, wanted: &str) -> Option<&DesktopEntry> {
        self.entries
            .iter()
            .find(|entry| normalize(last_segment(&entry.id)) == wanted)
    }

    fn by_exec_binary(&self, wanted: &str) -> Option<&DesktopEntry> {
        self.entries
            .iter()
            .find(|entry| exec_binary(&entry.exec).as_deref() == Some(wanted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, exec: &str, startup_wm_class: Option<&str>) -> DesktopEntry {
        DesktopEntry {
            id: id.to_string(),
            name: id.to_string(),
            icon: id.to_string(),
            exec: exec.to_string(),
            startup_wm_class: startup_wm_class.map(str::to_string),
            no_display: false,
            path: PathBuf::from(format!("/usr/share/applications/{id}.desktop")),
        }
    }

    fn index() -> DesktopIndex {
        DesktopIndex::from_entries(vec![
            entry("code", "/usr/share/code/code", None),
            entry("brave-browser", "/usr/bin/brave-browser", None),
            entry("org.gnome.gedit", "gedit", Some("Gedit")),
            entry("com.anthropic.Claude", "/opt/Claude/claude", None),
            entry("dev.warp.Warp", "/opt/warpdotdev/warp-terminal", None),
            entry("docker-desktop", "/opt/docker-desktop/bin/docker-desktop", None),
            entry("libreoffice-calc", "/usr/bin/libreoffice --calc", None),
            entry("spotify_spotify", "/snap/bin/spotify", Some("Spotify")),
        ])
    }

    #[test]
    fn matches_a_window_by_its_declared_startup_wm_class() {
        assert_eq!(index().match_window("Gedit").unwrap().id, "org.gnome.gedit");
    }

    #[test]
    fn matches_a_window_whose_class_differs_from_the_id_only_by_case() {
        assert_eq!(
            index().match_window("Brave-browser").unwrap().id,
            "brave-browser"
        );
    }

    #[test]
    fn matches_a_reverse_dns_class_against_the_same_reverse_dns_id() {
        assert_eq!(
            index().match_window("com.anthropic.Claude").unwrap().id,
            "com.anthropic.Claude"
        );
        assert_eq!(
            index().match_window("dev.warp.Warp").unwrap().id,
            "dev.warp.Warp"
        );
    }

    #[test]
    fn matches_a_class_carrying_a_space_against_a_hyphenated_id() {
        assert_eq!(
            index().match_window("Docker Desktop").unwrap().id,
            "docker-desktop"
        );
    }

    #[test]
    fn matches_a_plain_class_against_the_trailing_segment_of_a_reverse_dns_id() {
        let index = DesktopIndex::from_entries(vec![entry("org.gnome.Nautilus", "nautilus", None)]);

        assert_eq!(
            index.match_window("nautilus").unwrap().id,
            "org.gnome.Nautilus"
        );
    }

    #[test]
    fn falls_back_to_the_executable_name_when_nothing_else_matches() {
        let index = DesktopIndex::from_entries(vec![entry(
            "some-vendor-id",
            "/usr/bin/obsidian --flag",
            None,
        )]);

        assert_eq!(index.match_window("obsidian").unwrap().id, "some-vendor-id");
    }

    #[test]
    fn a_window_matching_nothing_returns_no_entry() {
        assert!(index().match_window("totally-unknown-app").is_none());
        assert!(index().match_window("").is_none());
    }

    #[test]
    fn startup_wm_class_wins_over_an_id_that_would_also_match() {
        let index = DesktopIndex::from_entries(vec![
            entry("spotify", "/usr/bin/spotify", None),
            entry("spotify_spotify", "/snap/bin/spotify", Some("Spotify")),
        ]);

        assert_eq!(
            index.match_window("Spotify").unwrap().id,
            "spotify_spotify"
        );
    }

    #[test]
    fn parses_the_fields_a_dock_needs_and_drops_exec_field_codes() {
        let parsed = parse_entry(
            "code",
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Visual Studio Code\n\
             Icon=vscode\n\
             Exec=/usr/share/code/code --unity-launch %F\n\
             StartupWMClass=Code\n",
            PathBuf::from("/usr/share/applications/code.desktop"),
        )
        .unwrap();

        assert_eq!(parsed.name, "Visual Studio Code");
        assert_eq!(parsed.icon, "vscode");
        assert_eq!(parsed.exec, "/usr/share/code/code --unity-launch");
        assert_eq!(parsed.startup_wm_class.as_deref(), Some("Code"));
    }

    #[test]
    fn keeps_only_the_desktop_entry_group_and_ignores_actions() {
        let parsed = parse_entry(
            "app",
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Real Name\n\
             Exec=/usr/bin/app\n\
             \n\
             [Desktop Action new-window]\n\
             Name=New Window\n\
             Exec=/usr/bin/app --new-window\n",
            PathBuf::from("/tmp/app.desktop"),
        )
        .unwrap();

        assert_eq!(parsed.name, "Real Name");
        assert_eq!(parsed.exec, "/usr/bin/app");
    }

    #[test]
    fn entries_that_are_not_applications_are_not_indexed() {
        assert!(parse_entry(
            "link",
            "[Desktop Entry]\nType=Link\nName=A Link\nURL=https://example.com\n",
            PathBuf::from("/tmp/link.desktop")
        )
        .is_none());
    }
}
