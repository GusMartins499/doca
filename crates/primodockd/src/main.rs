mod service;
mod x11;

use std::time::Duration;

use anyhow::{Context, Result};
use primodock_ipc::{BUS_NAME, OBJECT_PATH};
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

use crate::service::DockService;
use crate::x11::{spawn_worker, watch_root, RootChange, X11Backend};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("primodockd=info")),
        )
        .init();

    let backend = X11Backend::connect()?;
    tracing::info!(
        workspaces = backend.workspace_count()?,
        windows = backend.list_windows()?.len(),
        "connected to X11"
    );
    let x11 = spawn_worker(backend)?;

    let connection = zbus::connection::Builder::session()
        .context("cannot reach the session bus")?
        .name(BUS_NAME)
        .context("another primodockd is already running")?
        .serve_at(OBJECT_PATH, DockService { x11: x11.clone() })?
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
                let (windows, workspace) = coalesce(first, &mut rx).await;
                if windows {
                    DockService::windows_changed(emitter.signal_emitter()).await?;
                }
                if workspace {
                    let index = x11.current_workspace().await.unwrap_or(0);
                    DockService::workspace_changed(emitter.signal_emitter(), index).await?;
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
                break;
            }
        }
    }

    Ok(())
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

