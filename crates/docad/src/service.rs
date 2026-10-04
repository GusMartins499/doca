use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use doca_ipc::{
    Appearance, Application, DockItem, EnvironmentInfo, FolderEntry, WidgetSettings, WidgetState,
    WindowInfo,
};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

use crate::config::{Config, Environment};
use crate::patch;
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

/// Something the caller asked for that the config will not accept.
///
/// Told apart from `Failed` on purpose: a GUI can put "a dock called Work
/// already exists" next to the field the user typed it in, where "the disk is
/// full" belongs somewhere else entirely.
fn rejected(e: impl std::fmt::Display) -> zbus::fdo::Error {
    zbus::fdo::Error::InvalidArgs(e.to_string())
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

    /// Change the config and leave disk agreeing with memory, or neither.
    ///
    /// The lock is held across the change *and* the save, which is what keeps
    /// two writes landing together from losing one of them: without it both
    /// could read, both change their own copy, and the second save would write
    /// a file that never saw the first change. Nothing is awaited in here, so
    /// holding it costs no one anything.
    fn update_config<T>(
        &self,
        change: impl FnOnce(&mut Config) -> anyhow::Result<T>,
    ) -> zbus::fdo::Result<T> {
        let mut config = self.config.lock().map_err(|_| {
            zbus::fdo::Error::Failed("configuration lock was poisoned".to_string())
        })?;
        let mut working = config.clone();
        let outcome = change(&mut working).map_err(rejected)?;
        working.save().map_err(failed)?;
        *config = working;
        Ok(outcome)
    }

    /// A write, and the one announcement every reader is waiting for.
    ///
    /// The signal goes out only after the change is on disk, so anyone who
    /// re-reads on hearing it reads the new file and not the old one.
    async fn commit<T>(
        &self,
        emitter: &SignalEmitter<'_>,
        change: impl FnOnce(&mut Config) -> anyhow::Result<T>,
    ) -> zbus::fdo::Result<T> {
        let before = self.widget_shape()?;
        let outcome = self.update_config(change)?;
        let after = self.widget_shape()?;
        // Compared rather than always settled, so moving a slider in the
        // Appearance tab does not re-poll twelve widgets. Compared *here*
        // rather than named by each method, so a write added later cannot
        // forget to do it: the only way to change what widgets exist or what
        // they are set to is to go through this.
        if after != before {
            if let Err(e) = self.widgets.settle(after.0, after.1).await {
                // The config is saved and announced; a hub that did not hear
                // is a bar showing yesterday's setting, not a failed write.
                tracing::error!("the widgets did not hear the change: {e:#}");
            }
        }
        Self::config_changed(emitter).await?;
        Ok(outcome)
    }

    /// What widgets there are and what they are set to — the pair the hub is
    /// built from, and the only thing worth comparing across a write.
    fn widget_shape(&self) -> zbus::fdo::Result<(Vec<String>, crate::config::WidgetSettings)> {
        self.with_config(|config| (config.all_widgets(), config.widgets.clone()))
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

#[zbus::interface(name = "io.github.gusmartins499.Doca1")]
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

    async fn pin_item(
        &self,
        id: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let name = self.shown().await?.name;
        self.commit(&emitter, |config| config.pin(&name, id)).await
    }

    async fn unpin_item(
        &self,
        id: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let name = self.shown().await?.name;
        self.commit(&emitter, |config| config.unpin(&name, id))
            .await
    }

    async fn appearance(&self) -> zbus::fdo::Result<Appearance> {
        self.with_config(appearance_of)
    }

    /// Change the look of the dock, one named key at a time.
    async fn set_appearance(
        &self,
        changes: HashMap<String, OwnedValue>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let named: Vec<(&str, &Value)> = changes
            .iter()
            .map(|(key, value)| (key.as_str(), &**value))
            .collect();

        self.commit(&emitter, move |config| {
            let patched = patch::appearance(&config.appearance, named)?;
            config.set_appearance(patched);
            Ok(())
        })
        .await
    }

    async fn widget_settings(&self) -> zbus::fdo::Result<WidgetSettings> {
        self.with_config(widget_settings_of)
    }

    /// Change one setting of one widget.
    ///
    /// The value is written, announced, and handed to the widget that is
    /// already running — see `commit`.
    async fn set_widget_setting(
        &self,
        widget: &str,
        key: &str,
        value: OwnedValue,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.commit(&emitter, |config| {
            config.set_widget_setting(widget, key, patch::setting(&value)?)
        })
        .await
    }

    async fn set_environment_widgets(
        &self,
        name: &str,
        widgets: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.commit(&emitter, |config| {
            config.set_environment_widgets(name, widgets)
        })
        .await
    }

    async fn set_environment_workspaces(
        &self,
        name: &str,
        workspaces: Vec<i32>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.commit(&emitter, |config| {
            config.set_environment_workspaces(name, workspaces)
        })
        .await
    }

    async fn reorder_pinned(
        &self,
        name: &str,
        order: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.commit(&emitter, |config| config.reorder_pinned(name, order))
            .await
    }

    async fn reorder_environments(
        &self,
        order: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.commit(&emitter, |config| config.reorder_environments(order))
            .await
    }

    async fn add_environment(
        &self,
        name: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<String> {
        self.commit(&emitter, |config| config.add_environment(name))
            .await
    }

    async fn remove_environment(
        &self,
        name: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.commit(&emitter, |config| config.remove_environment(name))
            .await?;
        // A dock chosen by hand that no longer exists would otherwise keep the
        // bar pointing at nothing until the next workspace change.
        if self.chosen.get().as_deref() == Some(name) {
            self.chosen.clear();
        }
        Ok(())
    }

    async fn rename_environment(
        &self,
        from: &str,
        to: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<String> {
        let renamed = self
            .commit(&emitter, |config| config.rename_environment(from, to))
            .await?;
        if self.chosen.get().as_deref() == Some(from) {
            self.chosen.set(&renamed);
        }
        Ok(renamed)
    }

    async fn list_applications(&self) -> zbus::fdo::Result<Vec<Application>> {
        Ok(self
            .index
            .listed()
            .into_iter()
            .map(|entry| Application {
                id: entry.id.clone(),
                name: entry.name.clone(),
                icon: entry.icon.clone(),
            })
            .collect())
    }

    /// Pin to the dock named, which need not be the one on screen.
    async fn pin_in(
        &self,
        name: &str,
        id: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        // A pin on an id no `.desktop` file answers to draws nothing: it would
        // sit in the config looking like a bug in the dock rather than a typo
        // in the call. The bar's own `PinItem` needs no such check — the id it
        // sends came out of this index.
        if self.index.get(id).is_none() {
            return Err(rejected(format!("no application called {id}")));
        }
        self.commit(&emitter, |config| config.pin(name, id)).await
    }

    async fn unpin_in(
        &self,
        name: &str,
        id: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        // No index check: unpinning is how an id the index no longer knows —
        // an app that was uninstalled — gets out of the config.
        self.commit(&emitter, |config| config.unpin(name, id)).await
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
                    pinned: environment.pinned.clone(),
                    widgets: environment.widgets.clone(),
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
    pub async fn config_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

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

/// The look of the dock as the bus reports it: sanitised, never raw.
/// The daemon's settings in the flat shape the bus carries, already clamped.
fn widget_settings_of(config: &Config) -> WidgetSettings {
    let settings = config.widgets.sanitised();
    WidgetSettings {
        countdown_date: settings.countdown.date,
        countdown_label: settings.countdown.label,
        note_text: settings.note.text,
        timer_minutes: settings.timer.minutes,
        water_goal: settings.water.goal,
    }
}

fn appearance_of(config: &Config) -> Appearance {
    let appearance = config.appearance();
    Appearance {
        theme: appearance.theme,
        icon_size: appearance.icon_size,
        magnification: appearance.magnification,
        auto_hide: appearance.auto_hide,
        show_trash: appearance.show_trash,
        icon_theme: appearance.icon_theme,
        gtk_theme: appearance.gtk_theme,
        cursor_theme: appearance.cursor_theme,
    }
}
