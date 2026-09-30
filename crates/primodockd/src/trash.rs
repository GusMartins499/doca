use std::path::{Path, PathBuf};

use primodock_ipc::DockItem;

pub const ID: &str = "trash";

pub fn trash_files_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Trash/files")
}

pub fn count_in(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| entries.flatten().count())
        .unwrap_or(0)
}

pub fn icon_for(count: usize) -> &'static str {
    if count == 0 {
        "user-trash"
    } else {
        "user-trash-full"
    }
}

pub fn name_for(count: usize) -> String {
    match count {
        0 => "Trash, empty".to_string(),
        1 => "Trash, 1 item".to_string(),
        many => format!("Trash, {many} items"),
    }
}

pub fn item(count: usize) -> DockItem {
    DockItem {
        id: ID.to_string(),
        name: name_for(count),
        icon: icon_for(count).to_string(),
        pinned: true,
        windows: Vec::new(),
        active: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_trash_and_a_full_one_do_not_look_alike() {
        assert_ne!(icon_for(0), icon_for(1));
    }

    #[test]
    fn the_tooltip_says_how_much_is_in_there() {
        assert_eq!(name_for(0), "Trash, empty");
        assert_eq!(name_for(1), "Trash, 1 item");
        assert_eq!(name_for(7), "Trash, 7 items");
    }

    #[test]
    fn counting_a_directory_that_is_not_there_is_not_an_error() {
        assert_eq!(count_in(Path::new("/nonexistent/Trash/files")), 0);
    }

    #[test]
    fn the_trash_item_is_never_shown_as_running() {
        let item = item(3);

        assert!(item.windows.is_empty());
        assert!(!item.active);
        assert!(item.pinned);
    }

    #[test]
    fn counting_sees_every_kind_of_entry_not_just_files() {
        let dir = std::env::temp_dir().join(format!("primodock-trash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a-folder")).unwrap();
        std::fs::write(dir.join("a-file"), b"x").unwrap();

        assert_eq!(count_in(&dir), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
