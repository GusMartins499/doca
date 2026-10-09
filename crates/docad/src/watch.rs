//! Noticing that the config file changed under the daemon.
//!
//! The D-Bus is the only way the daemon *writes* the config, but the file is
//! still the thing this project tells people to edit by hand, and an edit in
//! an editor used to mean killing and restarting the daemon to see it. So the
//! file is watched, and a change to it becomes a reload — which, through
//! [`crate::config::Config::reread`], ends in nothing at all when the change
//! was the daemon's own save.
//!
//! The directory is watched rather than the file. Almost nothing rewrites a
//! file in place: editors and the daemon itself write beside it and rename
//! over it, and a watch on the file would follow the old inode into the bin
//! and hear nothing after the first save.
//!
//! inotify is read on a thread of its own, the way the X11 root is in
//! `x11.rs`: a blocking read on a thread is all this needs, and it hands the
//! main loop the same kind of channel the root watcher does.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use inotify::{Inotify, WatchMask};
use tokio::sync::mpsc;

/// How long the file has to stay still before it is read.
///
/// Editors save in more than one step — truncate then write, write a backup
/// then rename, write then `chmod` — and reading between two of them reads a
/// file that is empty or half there. 150 ms is long enough to cover all of a
/// save and short enough that the bar still seems to follow the keystroke
/// that saved it.
pub const QUIET: Duration = Duration::from_millis(150);

/// The most a burst can delay a reload, however long it goes on.
///
/// Without it a program that rewrote the file every 100 ms would hold the
/// reload off for ever, and the dock would never catch up with any of it.
pub const LONGEST: Duration = Duration::from_secs(1);

/// Watch the directory `path` lives in and call `on_change` for anything
/// that happens to `path` there.
///
/// A config that is a symlink — a dotfiles checkout — is edited at the other
/// end of the link, where nothing happens in `~/.config/doca` at all, so the
/// directory the link points into is watched too. That is resolved once, here;
/// a link moved to point somewhere else is only followed after a restart.
pub fn watch_config(path: &Path, on_change: impl Fn() + Send + 'static) -> Result<()> {
    let mut places = vec![place_of(path)?];
    if let Ok(target) = std::fs::canonicalize(path) {
        let target = place_of(&target)?;
        if target != places[0] {
            places.push(target);
        }
    }

    let inotify = Inotify::init().context("cannot start inotify")?;
    for (directory, _) in &places {
        // Created if it is not there, because a directory that does not exist
        // cannot be watched, and the first save would create it anyway.
        std::fs::create_dir_all(directory)
            .with_context(|| format!("cannot create {}", directory.display()))?;
        inotify
            .watches()
            .add(
                directory,
                WatchMask::CLOSE_WRITE
                    | WatchMask::MODIFY
                    | WatchMask::MOVED_TO
                    | WatchMask::MOVED_FROM
                    | WatchMask::CREATE
                    | WatchMask::DELETE,
            )
            .with_context(|| format!("cannot watch {}", directory.display()))?;
    }
    let names: Vec<OsString> = places.into_iter().map(|(_, name)| name).collect();

    std::thread::Builder::new()
        .name("config-watch".into())
        .spawn(move || {
            if let Err(e) = watch_loop(inotify, &names, &on_change) {
                tracing::error!("config watcher stopped: {e:#}");
            }
        })?;
    Ok(())
}

/// The directory a file lives in and its name there.
fn place_of(path: &Path) -> Result<(PathBuf, OsString)> {
    let directory = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no directory to watch", path.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("{} names no file", path.display()))?;
    Ok((directory.to_path_buf(), name.to_os_string()))
}

fn watch_loop(mut inotify: Inotify, names: &[OsString], on_change: &impl Fn()) -> Result<()> {
    let mut buffer = [0u8; 4096];
    loop {
        let events = inotify.read_events_blocking(&mut buffer)?;
        // Everything else in the directory — the scratch files a save writes
        // beside the config, an editor's swap file — is not the config.
        if events
            .filter_map(|event| event.name)
            .any(|name| names.iter().any(|wanted| wanted == name))
        {
            on_change();
        }
    }
}

/// Wait out a burst of changes, taking every one of them with it.
///
/// Called with the first change already in hand. Returns once [`QUIET`] has
/// passed with nothing new, or [`LONGEST`] after it was called, whichever
/// comes first, and leaves nothing behind in `changes` that happened before
/// then — so a save in two steps is one reload, not two.
pub async fn quiet(changes: &mut mpsc::UnboundedReceiver<()>) {
    let deadline = tokio::time::Instant::now() + LONGEST;
    loop {
        let until = (tokio::time::Instant::now() + QUIET).min(deadline);
        match tokio::time::timeout_at(until, changes.recv()).await {
            Ok(Some(())) => continue,
            Ok(None) | Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::Instant;

    /// Truncate then write, or write then rename: two events, one save.
    #[tokio::test(start_paused = true)]
    async fn a_save_in_two_steps_is_read_once() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tx.send(()).unwrap();
        tx.send(()).unwrap();

        rx.recv().await.unwrap();
        let started = Instant::now();
        quiet(&mut rx).await;

        assert!(rx.try_recv().is_err(), "the second step was left for another reload");
        assert_eq!(started.elapsed(), QUIET);
    }

    /// The second step can come a while after the first — an editor writing
    /// a backup before it touches the real file. Each change starts the wait
    /// again, so the file is read once it has stopped moving.
    #[tokio::test(start_paused = true)]
    async fn a_step_that_lands_inside_the_wait_starts_it_again() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tx.send(()).unwrap();
        let later = tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            later.send(()).unwrap();
        });

        rx.recv().await.unwrap();
        let started = Instant::now();
        quiet(&mut rx).await;

        assert_eq!(started.elapsed(), Duration::from_millis(100) + QUIET);
        assert!(rx.try_recv().is_err());
    }

    /// A save after the file has gone quiet is a save of its own, and has to
    /// be left for the next reload rather than swallowed by this one.
    #[tokio::test(start_paused = true)]
    async fn a_change_after_the_quiet_is_kept_for_the_next_reload() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tx.send(()).unwrap();
        let later = tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(QUIET + Duration::from_millis(50)).await;
            later.send(()).unwrap();
        });

        rx.recv().await.unwrap();
        quiet(&mut rx).await;

        assert!(
            rx.recv().await.is_some(),
            "the later save was eaten by the earlier reload"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_file_that_never_stops_changing_is_still_read() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tx.send(()).unwrap();
        let writer = tx.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if writer.send(()).is_err() {
                    break;
                }
            }
        });

        rx.recv().await.unwrap();
        let started = Instant::now();
        quiet(&mut rx).await;

        assert_eq!(started.elapsed(), LONGEST);
    }

    #[test]
    fn a_config_is_watched_from_the_directory_it_lives_in() {
        let (directory, name) = place_of(Path::new("/home/me/.config/doca/config.toml")).unwrap();

        assert_eq!(directory, Path::new("/home/me/.config/doca"));
        assert_eq!(name, "config.toml");
    }

    /// The real thing, end to end on a scratch directory: a save the way the
    /// daemon saves — beside the file, then renamed over it — is heard, and
    /// the scratch file it wrote first is not mistaken for the config.
    #[test]
    fn a_save_by_rename_is_heard_and_the_scratch_file_is_not() {
        let directory = std::env::temp_dir().join(format!("doca-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("config.toml");

        let (tx, rx) = std::sync::mpsc::channel();
        watch_config(&path, move || {
            let _ = tx.send(());
        })
        .unwrap();

        std::fs::write(directory.join("unrelated.toml"), "a = 1").unwrap();
        assert!(
            rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "a file that is not the config woke the watcher"
        );

        crate::atomic::write(&path, "b = 2").unwrap();
        assert!(
            rx.recv_timeout(Duration::from_secs(2)).is_ok(),
            "a save by rename went unheard"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }
}
