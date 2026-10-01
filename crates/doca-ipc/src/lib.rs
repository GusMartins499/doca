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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EnvironmentInfo {
    pub name: String,
    pub workspaces: Vec<i32>,
    pub current: bool,
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
}

/// The keys `SetAppearance` understands, by the name they carry on the wire.
pub mod appearance_key {
    pub const THEME: &str = "theme";
    pub const ICON_SIZE: &str = "icon_size";
    pub const MAGNIFICATION: &str = "magnification";
    pub const AUTO_HIDE: &str = "auto_hide";
    pub const SHOW_TRASH: &str = "show_trash";

    pub const ALL: [&str; 5] = [THEME, ICON_SIZE, MAGNIFICATION, AUTO_HIDE, SHOW_TRASH];
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

    fn set_widget_setting(
        &self,
        widget: &str,
        key: &str,
        value: OwnedValue,
    ) -> zbus::Result<()>;

    fn set_environment_widgets(&self, name: &str, widgets: Vec<String>) -> zbus::Result<()>;
    fn set_environment_workspaces(&self, name: &str, workspaces: Vec<i32>) -> zbus::Result<()>;
    fn reorder_pinned(&self, name: &str, order: Vec<String>) -> zbus::Result<()>;
    fn add_environment(&self, name: &str) -> zbus::Result<String>;
    fn remove_environment(&self, name: &str) -> zbus::Result<()>;
    fn rename_environment(&self, from: &str, to: &str) -> zbus::Result<String>;

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
