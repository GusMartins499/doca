use primodock_ipc::WindowInfo;
use zbus::object_server::SignalEmitter;

use crate::x11::XHandle;

pub struct DockService {
    pub x11: XHandle,
}

fn failed(e: impl std::fmt::Display) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(e.to_string())
}

#[zbus::interface(name = "dev.oprimo.PrimoDock1")]
impl DockService {
    async fn list_windows(&self) -> zbus::fdo::Result<Vec<WindowInfo>> {
        self.x11.list_windows().await.map_err(failed)
    }

    async fn activate_window(&self, id: u32) -> zbus::fdo::Result<()> {
        self.x11.activate_window(id).await.map_err(failed)
    }

    async fn current_workspace(&self) -> zbus::fdo::Result<i32> {
        self.x11.current_workspace().await.map_err(failed)
    }

    async fn workspace_count(&self) -> zbus::fdo::Result<i32> {
        self.x11.workspace_count().await.map_err(failed)
    }

    async fn set_workspace(&self, index: i32) -> zbus::fdo::Result<()> {
        self.x11.set_workspace(index).await.map_err(failed)
    }

    #[zbus(signal)]
    pub async fn windows_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn workspace_changed(emitter: &SignalEmitter<'_>, index: i32) -> zbus::Result<()>;
}
