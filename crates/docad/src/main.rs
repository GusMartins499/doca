mod config;
mod desktop;
mod folder;
mod model;
mod patch;
mod service;
mod trash;
mod widgets;
mod x11;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use doca_ipc::{BUS_NAME, OBJECT_PATH};
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::desktop::DesktopIndex;
use crate::service::{Chosen, DockService};
use crate::x11::{spawn_worker, watch_root, RootChange, X11Backend};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("docad=info")),
        )
        .init();

    let backend = X11Backend::connect()?;
    tracing::info!(
        workspaces = backend.workspace_count()?,
        windows = backend.list_windows()?.len(),
        "connected to X11"
    );
    let x11 = spawn_worker(backend)?;

    let index = Arc::new(DesktopIndex::load());
    let config = Arc::new(Mutex::new(Config::load()));
    tracing::info!(
        entries = index.len(),
        environments = config.lock().map(|c| c.environments.len()).unwrap_or(0),
        "desktop index loaded"
    );

    let (widget_ids, widget_settings) = config
        .lock()
        .map(|c| (c.all_widgets(), c.widgets.clone()))
        .unwrap_or_default();
    let (widgets, widget_changes) =
        crate::widgets::spawn_hub(crate::widgets::build(&widget_ids, &widget_settings))?;
    tracing::info!(widgets = widget_ids.len(), "widgets started");

    let chosen = Chosen::default();

    let connection = zbus::connection::Builder::session()
        .context("cannot reach the session bus")?
        .name(BUS_NAME)
        .context("another docad is already running")?
        .serve_at(
            OBJECT_PATH,
            DockService {
                x11: x11.clone(),
                widgets: widgets.clone(),
                index: index.clone(),
                config: config.clone(),
                chosen: chosen.clone(),
            },
        )?
        .build()
        .await?;

    let (tx, mut rx) = mpsc::unbounded_channel::<RootChange>();
    watch_root(move |change| {
        let _ = tx.send(change);
    })?;

    let emitter = connection
        .object_server()
        .interface::<_, DockService>(OBJECT_PATH)
        .await?;

    tracing::info!(bus = BUS_NAME, "listening");

    loop {
        tokio::select! {
            Some(first) = rx.recv() => {
                let announce = announcements_for(coalesce(first, &mut rx).await);
                if announce.windows {
                    DockService::windows_changed(emitter.signal_emitter()).await?;
                }
                if announce.workspace {
                    let index = x11.current_workspace().await.unwrap_or(0);
                    // Moving onto a workspace an environment asked for is a
                    // choice of its own, and it replaces the one a key made.
                    // A workspace only a catch-all covers asks for nothing, so
                    // a dock chosen by hand stays on screen across it.
                    if config
                        .lock()
                        .map(|c| c.workspace_is_claimed(index))
                        .unwrap_or(false)
                    {
                        chosen.clear();
                    }
                    DockService::workspace_changed(emitter.signal_emitter(), index).await?;
                }
                if announce.environment {
                    let index = x11.current_workspace().await.unwrap_or(0);
                    let picked = chosen.get();
                    let name = config
                        .lock()
                        .map(|c| c.environment_shown(picked.as_deref(), index).name.clone())
                        .unwrap_or_default();
                    DockService::environment_changed(emitter.signal_emitter(), &name).await?;
                }
                if announce.items {
                    DockService::items_changed(emitter.signal_emitter()).await?;
                }
            }
            Ok(state) = widget_changes.recv() => {
                DockService::widget_changed(emitter.signal_emitter(), state).await?;
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
                break;
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Announcements {
    pub windows: bool,
    pub workspace: bool,
    pub environment: bool,
    pub items: bool,
}

pub fn announcements_for((windows, workspace): (bool, bool)) -> Announcements {
    Announcements {
        windows,
        workspace,
        environment: workspace,
        items: windows || workspace,
    }
}

const COALESCE_WINDOW: Duration = Duration::from_millis(50);

async fn coalesce(
    first: RootChange,
    rx: &mut mpsc::UnboundedReceiver<RootChange>,
) -> (bool, bool) {
    let mut windows = false;
    let mut workspace = false;
    let mut fold = |change| match change {
        RootChange::Windows => windows = true,
        RootChange::Workspace => workspace = true,
    };

    fold(first);
    let deadline = tokio::time::Instant::now() + COALESCE_WINDOW;
    while let Ok(Some(change)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        fold(change);
    }
    (windows, workspace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_change_announces_the_environment_and_the_items_it_changed() {
        let announce = announcements_for((false, true));

        assert!(announce.workspace);
        assert!(
            announce.environment,
            "switching workspace switches environment, and the bar has to hear about it"
        );
        assert!(
            announce.items,
            "each environment pins its own apps, so the row changes too"
        );
    }

    #[test]
    fn a_window_change_does_not_pretend_the_environment_moved() {
        let announce = announcements_for((true, false));

        assert!(announce.windows);
        assert!(announce.items);
        assert!(!announce.environment);
        assert!(!announce.workspace);
    }

    #[test]
    fn nothing_changing_announces_nothing() {
        let announce = announcements_for((false, false));

        assert_eq!(
            announce,
            Announcements {
                windows: false,
                workspace: false,
                environment: false,
                items: false
            }
        );
    }

    #[tokio::test]
    async fn a_burst_of_root_changes_becomes_a_single_notification() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tx.send(RootChange::Windows).unwrap();
        tx.send(RootChange::Windows).unwrap();
        tx.send(RootChange::Windows).unwrap();
        tx.send(RootChange::Workspace).unwrap();

        let first = rx.recv().await.unwrap();
        let (windows, workspace) = coalesce(first, &mut rx).await;

        assert!(windows);
        assert!(workspace);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_change_of_one_kind_does_not_announce_the_other() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tx.send(RootChange::Workspace).unwrap();

        let first = rx.recv().await.unwrap();
        let (windows, workspace) = coalesce(first, &mut rx).await;

        assert!(!windows);
        assert!(workspace);
    }
}

