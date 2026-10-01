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
use doca_ipc::{Appearance, DocaProxy};
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
