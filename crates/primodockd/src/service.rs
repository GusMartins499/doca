use std::sync::{Arc, Mutex};

use primodock_ipc::{
    Appearance, DockItem, EnvironmentInfo, FolderEntry, WidgetState, WindowInfo,
};
use zbus::object_server::SignalEmitter;

use crate::config::{Config, Environment};
use crate::desktop::DesktopIndex;
use crate::folder;
use crate::model;
use crate::trash;
use crate::widgets::WidgetHandle;
use crate::x11::XHandle;

pub struct DockService {
    pub x11: XHandle,
    pub widgets: WidgetHandle,
    pub index: Arc<DesktopIndex>,
    pub config: Arc<Mutex<Config>>,
    /// The dock put on screen by hand, overriding what the workspace asks for.
    pub chosen: Chosen,
}

/// The environment a key chose, until a workspace claims one of its own.
#[derive(Clone, Default)]
pub struct Chosen(Arc<Mutex<Option<String>>>);

impl Chosen {
    pub fn get(&self) -> Option<String> {
        self.0.lock().ok().and_then(|name| name.clone())
    }

    pub fn set(&self, name: &str) {
        if let Ok(mut chosen) = self.0.lock() {
            *chosen = Some(name.to_string());
        }
    }

    pub fn clear(&self) {
        if let Ok(mut chosen) = self.0.lock() {
            *chosen = None;
        }
    }
}

fn failed(e: impl std::fmt::Display) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(e.to_string())
}

impl DockService {
    async fn workspace(&self) -> zbus::fdo::Result<i32> {
        self.x11.current_workspace().await.map_err(failed)
    }

    fn with_config<T>(&self, read: impl FnOnce(&Config) -> T) -> zbus::fdo::Result<T> {
        let config = self.config.lock().map_err(|_| {
            zbus::fdo::Error::Failed("configuration lock was poisoned".to_string())
        })?;
        Ok(read(&config))
    }

    /// The environment on screen right now.
    async fn shown(&self) -> zbus::fdo::Result<Environment> {
        let workspace = self.workspace().await?;
        let chosen = self.chosen.get();
        self.with_config(|config| config.environment_shown(chosen.as_deref(), workspace).clone())
    }

    async fn items(&self) -> zbus::fdo::Result<Vec<DockItem>> {
        let windows = self.x11.list_windows().await.map_err(failed)?;
        let environment = self.shown().await?;
        let visible = model::windows_in(&environment, &windows);
        let mut items = model::build(&self.index, &environment.pinned, &visible);
        for path in &environment.folders {
            items.push(folder::item(&folder::expand(path)));
        }
        if self.with_config(|config| config.appearance().show_trash)? {
            items.push(trash::item(trash::count_in(&trash::trash_files_dir())));
        }
        Ok(items)
    }

    fn update_config(&self, change: impl FnOnce(&mut Config)) -> zbus::fdo::Result<()> {
        let mut config = self.config.lock().map_err(|_| {
            zbus::fdo::Error::Failed("configuration lock was poisoned".to_string())
        })?;
        change(&mut config);
        config.save().map_err(failed)
    }

    fn launch_path(&self, path: &str) -> zbus::fdo::Result<()> {
        std::process::Command::new("xdg-open")
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(failed)?;
        Ok(())
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
        if id == trash::ID {
            return self.launch_path(&trash::trash_files_dir().to_string_lossy());
        }
        if let Some(path) = folder::path_of(id) {
            return self.launch_path(&path.to_string_lossy());
        }
        let windows = self.x11.list_windows().await.map_err(failed)?;
        let environment = self.shown().await?;
        let visible = model::windows_in(&environment, &windows);
        let mine = model::windows_of(&self.index, id, &visible);

        let Some(first) = mine.first() else {
            return self.spawn(id);
        };

        match mine.iter().find(|window| window.active) {
            Some(focused) => self.x11.minimize_window(focused.id).await.map_err(failed),
            None => self.x11.activate_window(first.id).await.map_err(failed),
        }
    }

    async fn launch_item(&self, id: &str) -> zbus::fdo::Result<()> {
        self.spawn(id)
    }

    async fn open_with(&self, id: &str, paths: Vec<String>) -> zbus::fdo::Result<()> {
        let entry = self
            .index
            .get(id)
            .ok_or_else(|| zbus::fdo::Error::Failed(format!("no desktop entry for {id}")))?;
        let quoted: Vec<String> = paths
            .iter()
            .filter(|path| std::path::Path::new(path).exists())
            .map(|path| format!("'{}'", path.replace('\'', "'\\''")))
            .collect();
        if quoted.is_empty() {
            return Err(zbus::fdo::Error::Failed("no readable path dropped".into()));
        }
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("exec {} {}", entry.exec, quoted.join(" ")))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(failed)?;
        Ok(())
    }

    async fn pin_item(&self, id: &str) -> zbus::fdo::Result<()> {
        let name = self.shown().await?.name;
        self.update_config(|config| config.pin(&name, id))
    }

    async fn unpin_item(&self, id: &str) -> zbus::fdo::Result<()> {
        let name = self.shown().await?.name;
        self.update_config(|config| config.unpin(&name, id))
    }

    async fn appearance(&self) -> zbus::fdo::Result<Appearance> {
        self.with_config(|config| {
            let appearance = config.appearance();
            Appearance {
                theme: appearance.theme,
                icon_size: appearance.icon_size,
                magnification: appearance.magnification,
                auto_hide: appearance.auto_hide,
            }
        })
    }

    async fn list_widgets(&self) -> zbus::fdo::Result<Vec<WidgetState>> {
        let wanted = self.shown().await?.widgets;
        let available = self.widgets.list().await.map_err(failed)?;
        Ok(wanted
            .iter()
            .filter_map(|id| available.iter().find(|state| &state.id == id).cloned())
            .collect())
    }

    async fn invoke_widget(&self, id: &str, action: &str) -> zbus::fdo::Result<()> {
        self.widgets.invoke(id, action).await.map_err(failed)
    }

    async fn list_folder(&self, id: &str) -> zbus::fdo::Result<Vec<FolderEntry>> {
        let path = folder::path_of(id)
            .ok_or_else(|| zbus::fdo::Error::Failed(format!("{id} is not a folder")))?;
        Ok(folder::list(&path))
    }

    async fn open_path(&self, path: &str) -> zbus::fdo::Result<()> {
        self.launch_path(path)
    }

    async fn item_windows(&self, id: &str) -> zbus::fdo::Result<Vec<WindowInfo>> {
        let windows = self.x11.list_windows().await.map_err(failed)?;
        let environment = self.shown().await?;
        let visible = model::windows_in(&environment, &windows);
        Ok(model::windows_of(&self.index, id, &visible)
            .into_iter()
            .cloned()
            .collect())
    }

    async fn list_environments(&self) -> zbus::fdo::Result<Vec<EnvironmentInfo>> {
        let current = self.shown().await?.name;
        self.with_config(|config| {
            config
                .environments
                .iter()
                .map(|environment| EnvironmentInfo {
                    name: environment.name.clone(),
                    workspaces: environment.workspaces.clone(),
                    current: environment.name == current,
                })
                .collect()
        })
    }

    async fn current_environment(&self) -> zbus::fdo::Result<String> {
        Ok(self.shown().await?.name)
    }

    /// Put an environment on screen. The workspace does not move.
    async fn set_environment(
        &self,
        name: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let known = self.with_config(|config| {
            config
                .environment_named(name)
                .map(|environment| environment.name.clone())
        })?;
        let Some(known) = known else {
            return Err(zbus::fdo::Error::Failed(format!("no environment {name}")));
        };
        self.chosen.set(&known);
        Self::environment_changed(&emitter, &known).await?;
        Ok(())
    }

    /// Put the next environment on screen. The workspace does not move.
    async fn cycle_environment(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<String> {
        let current = self.shown().await?.name;
        let next = self.with_config(|config| {
            config
                .next_environment_after(&current)
                .map(|environment| environment.name.clone())
        })?;

        let Some(next) = next else {
            return Err(zbus::fdo::Error::Failed(
                "no other environment to cycle to".to_string(),
            ));
        };
        self.chosen.set(&next);
        Self::environment_changed(&emitter, &next).await?;
        Ok(next)
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
    pub async fn environment_changed(emitter: &SignalEmitter<'_>, name: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn widget_changed(
        emitter: &SignalEmitter<'_>,
        state: WidgetState,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn items_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn windows_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn workspace_changed(emitter: &SignalEmitter<'_>, index: i32) -> zbus::Result<()>;
}
