use std::sync::{Arc, Mutex};

use primodock_ipc::{DockItem, WindowInfo};
use zbus::object_server::SignalEmitter;

use crate::config::Config;
use crate::desktop::DesktopIndex;
use crate::model;
use crate::x11::XHandle;

pub struct DockService {
    pub x11: XHandle,
    pub index: Arc<DesktopIndex>,
    pub config: Arc<Mutex<Config>>,
}

fn failed(e: impl std::fmt::Display) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(e.to_string())
}

impl DockService {
    fn pinned(&self) -> Vec<String> {
        self.config
            .lock()
            .map(|config| config.pinned.clone())
            .unwrap_or_default()
    }

    async fn items(&self) -> zbus::fdo::Result<Vec<DockItem>> {
        let windows = self.x11.list_windows().await.map_err(failed)?;
        Ok(model::build(&self.index, &self.pinned(), &windows))
    }

    fn update_config(&self, change: impl FnOnce(&mut Config)) -> zbus::fdo::Result<()> {
        let mut config = self.config.lock().map_err(|_| {
            zbus::fdo::Error::Failed("configuration lock was poisoned".to_string())
        })?;
        change(&mut config);
        config.save().map_err(failed)
    }

    fn spawn(&self, id: &str) -> zbus::fdo::Result<()> {
        let entry = self
            .index
            .get(id)
            .ok_or_else(|| zbus::fdo::Error::Failed(format!("no desktop entry for {id}")))?;
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("exec {}", entry.exec))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(failed)?;
        Ok(())
    }
}

#[zbus::interface(name = "dev.oprimo.PrimoDock1")]
impl DockService {
    async fn list_items(&self) -> zbus::fdo::Result<Vec<DockItem>> {
        self.items().await
    }

    async fn activate_item(&self, id: &str) -> zbus::fdo::Result<()> {
        if let Some(window) = model::window_of_unmatched_id(id) {
            return self.x11.activate_window(window).await.map_err(failed);
        }

        let items = self.items().await?;
        let Some(item) = items.into_iter().find(|item| item.id == id) else {
            return Err(zbus::fdo::Error::Failed(format!("no dock item {id}")));
        };

        let Some(&first) = item.windows.first() else {
            return self.spawn(id);
        };

        if item.active {
            self.x11.minimize_window(first).await.map_err(failed)
        } else {
            self.x11.activate_window(first).await.map_err(failed)
        }
    }

    async fn launch_item(&self, id: &str) -> zbus::fdo::Result<()> {
        self.spawn(id)
    }

    async fn pin_item(&self, id: &str) -> zbus::fdo::Result<()> {
        self.update_config(|config| config.pin(id))
    }

    async fn unpin_item(&self, id: &str) -> zbus::fdo::Result<()> {
        self.update_config(|config| config.unpin(id))
    }

    async fn close_window(&self, id: u32) -> zbus::fdo::Result<()> {
        self.x11.close_window(id).await.map_err(failed)
    }

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
    pub async fn items_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn windows_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn workspace_changed(emitter: &SignalEmitter<'_>, index: i32) -> zbus::Result<()>;
}
