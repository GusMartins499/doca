use primodock_ipc::{DockItem, WindowInfo};

use crate::desktop::DesktopIndex;

const UNMATCHED_PREFIX: &str = "window:";

pub fn unmatched_id(window: u32) -> String {
    format!("{UNMATCHED_PREFIX}{window}")
}

pub fn window_of_unmatched_id(id: &str) -> Option<u32> {
    id.strip_prefix(UNMATCHED_PREFIX)?.parse().ok()
}

pub fn build(index: &DesktopIndex, pinned: &[String], windows: &[WindowInfo]) -> Vec<DockItem> {
    let mut items: Vec<DockItem> = pinned
        .iter()
        .map(|id| {
            let entry = index.get(id);
            DockItem {
                id: id.clone(),
                name: entry.map(|e| e.name.clone()).unwrap_or_else(|| id.clone()),
                icon: entry
                    .map(|e| e.icon.clone())
                    .unwrap_or_else(|| "application-x-executable".to_string()),
                pinned: true,
                windows: Vec::new(),
                active: false,
            }
        })
        .collect();

    for window in windows {
        let matched = index.match_window(&window.app_id);
        let id = matched
            .map(|entry| entry.id.clone())
            .unwrap_or_else(|| unmatched_id(window.id));

        if let Some(item) = items.iter_mut().find(|item| item.id == id) {
            item.windows.push(window.id);
            item.active |= window.active;
            continue;
        }

        items.push(DockItem {
            id,
            name: matched
                .map(|entry| entry.name.clone())
                .unwrap_or_else(|| display_name(window)),
            icon: matched
                .map(|entry| entry.icon.clone())
                .unwrap_or_else(|| "application-x-executable".to_string()),
            pinned: false,
            windows: vec![window.id],
            active: window.active,
        });
    }

    items
}

fn display_name(window: &WindowInfo) -> String {
    if window.app_id.is_empty() {
        window.title.clone()
    } else {
        window.app_id.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::DesktopEntry;
    use std::path::PathBuf;

    fn entry(id: &str, name: &str) -> DesktopEntry {
        DesktopEntry {
            id: id.to_string(),
            name: name.to_string(),
            icon: format!("{id}-icon"),
            exec: format!("/usr/bin/{id}"),
            startup_wm_class: None,
            no_display: false,
            path: PathBuf::from("/tmp"),
        }
    }

    fn index() -> DesktopIndex {
        DesktopIndex::from_entries(vec![
            entry("code", "Visual Studio Code"),
            entry("brave-browser", "Brave"),
            entry("discord", "Discord"),
        ])
    }

    fn window(id: u32, app_id: &str, active: bool) -> WindowInfo {
        WindowInfo {
            id,
            title: format!("window {id}"),
            app_id: app_id.to_string(),
            workspace: 0,
            active,
        }
    }

    #[test]
    fn a_pinned_app_with_no_windows_still_appears_in_the_dock() {
        let items = build(&index(), &["code".into()], &[]);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Visual Studio Code");
        assert!(items[0].pinned);
        assert!(items[0].windows.is_empty());
    }

    #[test]
    fn several_windows_of_one_app_collapse_onto_a_single_item() {
        let windows = vec![
            window(1, "code", false),
            window(2, "code", false),
            window(3, "code", false),
        ];

        let items = build(&index(), &["code".into()], &windows);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].windows, vec![1, 2, 3]);
    }

    #[test]
    fn an_item_is_active_when_any_of_its_windows_is() {
        let windows = vec![window(1, "code", false), window(2, "code", true)];

        let items = build(&index(), &["code".into()], &windows);

        assert!(items[0].active);
    }

    #[test]
    fn a_running_app_that_is_not_pinned_appears_after_the_pinned_ones() {
        let pinned = vec!["code".to_string(), "brave-browser".to_string()];
        let windows = vec![window(1, "discord", false)];

        let items = build(&index(), &pinned, &windows);

        assert_eq!(
            items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec!["code", "brave-browser", "discord"]
        );
        assert!(!items[2].pinned);
    }

    #[test]
    fn pinned_order_is_the_configured_order_not_the_window_order() {
        let pinned = vec!["discord".to_string(), "code".to_string()];
        let windows = vec![window(1, "code", false), window(2, "discord", false)];

        let items = build(&index(), &pinned, &windows);

        assert_eq!(
            items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec!["discord", "code"]
        );
    }

    #[test]
    fn a_window_matching_no_desktop_entry_still_gets_an_item() {
        let windows = vec![window(42, "some-unpackaged-thing", false)];

        let items = build(&index(), &[], &windows);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "window:42");
        assert_eq!(items[0].name, "some-unpackaged-thing");
        assert_eq!(window_of_unmatched_id(&items[0].id), Some(42));
    }

    #[test]
    fn two_unmatched_windows_stay_separate_items() {
        let windows = vec![window(1, "", false), window(2, "", false)];

        let items = build(&index(), &[], &windows);

        assert_eq!(items.len(), 2);
    }

    #[test]
    fn a_pinned_app_that_is_running_is_one_item_not_two() {
        let windows = vec![window(1, "code", true)];

        let items = build(&index(), &["code".into()], &windows);

        assert_eq!(items.len(), 1);
        assert!(items[0].pinned);
        assert_eq!(items[0].windows, vec![1]);
    }
}
