use serde::{Deserialize, Serialize};
use zbus::zvariant::Type;

pub const BUS_NAME: &str = "dev.oprimo.PrimoDock";
pub const OBJECT_PATH: &str = "/dev/oprimo/PrimoDock";
pub const INTERFACE: &str = "dev.oprimo.PrimoDock1";

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Appearance {
    pub theme: String,
    pub icon_size: i32,
    pub magnification: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct FolderEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[zbus::proxy(
    interface = "dev.oprimo.PrimoDock1",
    default_service = "dev.oprimo.PrimoDock",
    default_path = "/dev/oprimo/PrimoDock"
)]
pub trait PrimoDock {
    fn list_environments(&self) -> zbus::Result<Vec<EnvironmentInfo>>;
    fn current_environment(&self) -> zbus::Result<String>;
    fn set_environment(&self, name: &str) -> zbus::Result<()>;
    fn cycle_environment(&self) -> zbus::Result<String>;

    fn appearance(&self) -> zbus::Result<Appearance>;

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
