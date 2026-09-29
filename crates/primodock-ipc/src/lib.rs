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

#[zbus::proxy(
    interface = "dev.oprimo.PrimoDock1",
    default_service = "dev.oprimo.PrimoDock",
    default_path = "/dev/oprimo/PrimoDock"
)]
pub trait PrimoDock {
    fn list_windows(&self) -> zbus::Result<Vec<WindowInfo>>;
    fn activate_window(&self, id: u32) -> zbus::Result<()>;
    fn current_workspace(&self) -> zbus::Result<i32>;
    fn workspace_count(&self) -> zbus::Result<i32>;
    fn set_workspace(&self, index: i32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn windows_changed(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn workspace_changed(&self, index: i32) -> zbus::Result<()>;
}
