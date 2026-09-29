//! primodockd — the brain.
//!
//! Owns the window model and, later, environments, widgets and config.
//! Draws nothing and knows nothing about how the bar looks.

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

    // The watcher is a sync thread and signal emission is async, so the
    // channel is the seam between them.
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

/// How long to keep absorbing root changes before telling anyone.
///
/// Short enough that the bar still feels immediate, long enough to collapse
/// a burst into one update.
const COALESCE_WINDOW: Duration = Duration::from_millis(50);

/// Collapses a burst of root-window changes into a single verdict.
///
/// One workspace switch makes the window manager touch `_NET_ACTIVE_WINDOW`
/// several times in a few milliseconds. Forwarding each one would have the
/// bar rebuild itself a dozen times for one user action, which on a laptop
/// is paid for in battery. So the first change opens a short window, and
/// everything that lands inside it is folded into the same notification.
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
