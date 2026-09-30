use std::path::{Path, PathBuf};

use primodock_ipc::{DockItem, FolderEntry};

pub const PREFIX: &str = "folder:";
pub const MAX_ENTRIES: usize = 60;

pub fn id_for(path: &str) -> String {
    format!("{PREFIX}{}", expand(path).to_string_lossy())
}

pub fn path_of(id: &str) -> Option<PathBuf> {
    id.strip_prefix(PREFIX).map(PathBuf::from)
}

pub fn expand(path: &str) -> PathBuf {
    let Some(rest) = path.strip_prefix("~/") else {
        return PathBuf::from(path);
    };
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(rest),
        None => PathBuf::from(path),
    }
}

pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

pub fn item(path: &Path) -> DockItem {
    DockItem {
        id: id_for(&path.to_string_lossy()),
        name: display_name(path),
        icon: "folder".to_string(),
        pinned: true,
        windows: Vec::new(),
        active: false,
    }
}

pub fn list(path: &Path) -> Vec<FolderEntry> {
    let Ok(read) = std::fs::read_dir(path) else {
        return Vec::new();
    };

    let mut entries: Vec<FolderEntry> = read
        .flatten()
        .filter(|entry| {
            !entry
                .file_name()
                .to_string_lossy()
                .starts_with('.')
        })
        .map(|entry| {
            let path = entry.path();
            FolderEntry {
                name: display_name(&path),
                path: path.to_string_lossy().into_owned(),
                is_dir: path.is_dir(),
            }
        })
        .collect();

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries.truncate(MAX_ENTRIES);
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("primodock-folder-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_id_round_trips_back_to_the_path_it_came_from() {
        let id = id_for("/home/someone/Downloads");

        assert_eq!(
            path_of(&id),
            Some(PathBuf::from("/home/someone/Downloads"))
        );
    }

    #[test]
    fn an_id_that_is_not_a_folder_yields_no_path() {
        assert_eq!(path_of("code"), None);
        assert_eq!(path_of("window:42"), None);
    }

    #[test]
    fn a_tilde_is_expanded_so_configs_can_be_written_by_hand() {
        std::env::set_var("HOME", "/home/someone");

        assert_eq!(expand("~/Downloads"), PathBuf::from("/home/someone/Downloads"));
        assert_eq!(expand("/tmp/x"), PathBuf::from("/tmp/x"));
    }

    #[test]
    fn a_tilde_in_the_middle_of_a_path_is_left_alone() {
        assert_eq!(expand("/tmp/a~b"), PathBuf::from("/tmp/a~b"));
    }

    #[test]
    fn folders_are_listed_before_files() {
        let dir = scratch("order");
        std::fs::create_dir(dir.join("zzz-folder")).unwrap();
        std::fs::write(dir.join("aaa-file"), b"x").unwrap();

        let entries = list(&dir);

        assert_eq!(entries[0].name, "zzz-folder");
        assert_eq!(entries[1].name, "aaa-file");
    }

    #[test]
    fn entries_of_the_same_kind_are_ordered_by_name_ignoring_case() {
        let dir = scratch("case");
        for name in ["banana", "Apple", "cherry"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }

        let names: Vec<String> = list(&dir).into_iter().map(|e| e.name).collect();

        assert_eq!(names, vec!["Apple", "banana", "cherry"]);
    }

    #[test]
    fn hidden_entries_are_left_out() {
        let dir = scratch("hidden");
        std::fs::write(dir.join(".hidden"), b"x").unwrap();
        std::fs::write(dir.join("visible"), b"x").unwrap();

        let names: Vec<String> = list(&dir).into_iter().map(|e| e.name).collect();

        assert_eq!(names, vec!["visible"]);
    }

    #[test]
    fn a_folder_with_thousands_of_files_does_not_become_an_endless_menu() {
        let dir = scratch("many");
        for n in 0..(MAX_ENTRIES + 25) {
            std::fs::write(dir.join(format!("file-{n:04}")), b"x").unwrap();
        }

        assert_eq!(list(&dir).len(), MAX_ENTRIES);
    }

    #[test]
    fn a_folder_that_is_not_there_lists_as_empty_rather_than_failing() {
        assert!(list(Path::new("/nonexistent/folder")).is_empty());
    }

    #[test]
    fn the_dock_item_is_named_after_the_folder_not_its_whole_path() {
        let item = item(Path::new("/home/someone/Downloads"));

        assert_eq!(item.name, "Downloads");
        assert!(item.windows.is_empty());
    }
}
