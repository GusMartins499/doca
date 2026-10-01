//! The window's end of the bus.
//!
//! Every control writes through here and nothing writes to the config file:
//! the daemon validates, saves and announces, so the window never has to know
//! where the TOML lives or what the limits are. What comes back from
//! `appearance` is already sanitised, which is why a control can show it
//! without second-guessing it.

use std::collections::HashMap;
use std::rc::Rc;

use anyhow::Result;
use doca_ipc::{Appearance, Application, DocaProxy, EnvironmentInfo, WidgetSettings};

use crate::docks::Action;
use crate::widgets::Wrote;
use zbus::zvariant::{OwnedValue, Value};

#[derive(Clone)]
pub struct Link {
    proxy: Rc<DocaProxy<'static>>,
}

impl Link {
    pub async fn open() -> Result<Self> {
        let connection = zbus::Connection::session().await?;
        let proxy = DocaProxy::new(&connection).await?;
        // Not a handshake for its own sake: this is what tells a window
        // opened with no daemon behind it to say so instead of drawing
        // controls that would fail on the first click.
        proxy.appearance().await?;
        Ok(Self {
            proxy: Rc::new(proxy),
        })
    }

    pub fn proxy(&self) -> Rc<DocaProxy<'static>> {
        self.proxy.clone()
    }

    pub async fn appearance(&self) -> Option<Appearance> {
        match self.proxy.appearance().await {
            Ok(appearance) => Some(appearance),
            Err(e) => {
                tracing::warn!("cannot read the appearance: {e}");
                None
            }
        }
    }

    pub async fn environments(&self) -> Option<Vec<EnvironmentInfo>> {
        self.read("the docks", self.proxy.list_environments().await)
    }

    pub async fn widget_settings(&self) -> Option<WidgetSettings> {
        self.read("the widget settings", self.proxy.widget_settings().await)
    }

    pub async fn applications(&self) -> Option<Vec<Application>> {
        self.read("the installed applications", self.proxy.list_applications().await)
    }

    fn read<T>(&self, what: &str, answer: zbus::Result<T>) -> Option<T> {
        match answer {
            Ok(value) => Some(value),
            Err(e) => {
                tracing::warn!("cannot read {what}: {e}");
                None
            }
        }
    }

    /// Carry out one change to the docks, and say why if it did not happen.
    ///
    /// Unlike the look, these can be refused for a reason the user has to see:
    /// a name already taken, a dock that was the last one. The daemon sends
    /// that sentence back and the window puts it on screen — which is why this
    /// one waits for the answer where `set` does not.
    pub async fn apply(&self, action: Action) -> Result<(), String> {
        let answer = match action {
            Action::Add(name) => self.proxy.add_environment(&name).await.map(|_| ()),
            Action::Remove(name) => self.proxy.remove_environment(&name).await,
            Action::Rename { from, to } => {
                self.proxy.rename_environment(&from, &to).await.map(|_| ())
            }
            Action::Workspaces { name, workspaces } => {
                self.proxy.set_environment_workspaces(&name, workspaces).await
            }
            Action::Pin { name, id } => self.proxy.pin_in(&name, &id).await,
            Action::Unpin { name, id } => self.proxy.unpin_in(&name, &id).await,
            Action::Reorder { name, order } => self.proxy.reorder_pinned(&name, order).await,
            Action::Widgets { name, widgets } => {
                self.proxy.set_environment_widgets(&name, widgets).await
            }
        };
        answer.map_err(|e| said(&e))
    }

    /// Write one widget setting, and say why if the daemon would not have it.
    ///
    /// Waits for the answer, like the dock changes and unlike the look: the
    /// daemon refuses a setting it does not know, and the only way that can
    /// happen is a key this window and the daemon disagree about — which the
    /// user cannot fix but should not be left guessing about either.
    pub async fn put(&self, wrote: Wrote) -> Result<(), String> {
        let (widget, key, value) = match wrote {
            Wrote::Text { widget, key, text } => (widget, key, Value::from(text)),
            Wrote::Count { widget, key, count } => (widget, key, Value::from(count)),
        };
        let value = OwnedValue::try_from(value)
            .map_err(|e| format!("{widget}.{key} could not be put on the bus: {e}"))?;

        self.proxy
            .set_widget_setting(widget, key, value)
            .await
            .map_err(|e| said(&e))
    }

    /// Change one key of the look, and forget about it.
    ///
    /// Nothing waits for the answer: the control the user is holding has
    /// already moved, and the dock changing is the confirmation. A refusal is
    /// logged rather than shown, because the daemon only refuses what this
    /// window cannot send — the ranges come from the same contract the daemon
    /// validates against.
    pub fn set(&self, key: &'static str, value: Value<'static>) {
        let proxy = self.proxy.clone();
        glib::spawn_future_local(async move {
            let Ok(value) = OwnedValue::try_from(value) else {
                tracing::error!("{key} could not be put on the bus");
                return;
            };
            let changes = HashMap::from([(key.to_string(), value)]);
            if let Err(e) = proxy.set_appearance(changes).await {
                tracing::error!("{key} was refused: {e}");
            }
        });
    }
}

/// What the daemon said, without the D-Bus error name in front of it.
///
/// `zbus`'s own Display puts the interface name first, which is true and no
/// use to anyone reading a settings window.
fn said(e: &zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(_, Some(message), _) => message.clone(),
        other => other.to_string(),
    }
}
