//! Writing a file whole, or leaving the one that is there untouched.
//!
//! Two files are written while the daemon runs — the config the user chose
//! and the state the day accumulated — and both are the only record of what
//! they hold. A session that dies mid-write would otherwise leave half a file
//! behind, which does not parse, so the next start falls back to defaults and
//! whatever was in it is gone.
//!
//! Writing beside the real file and renaming over it makes the swap atomic: a
//! reader sees the old file or the new one, never a torn one. This lives in
//! one place because getting it subtly wrong in the second caller — flushing
//! after the rename, or writing the temporary file somewhere else — would
//! look exactly like getting it right until the machine lost power.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};

/// A name no other writer is using, next to the file it will become.
///
/// Same directory, because `rename` is only atomic within one filesystem.
fn temporary_name(path: &Path) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stem = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    format!(
        ".{stem}.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Put these bytes at this path, creating the directory above it if need be.
///
/// Bytes rather than text since the cover cache joined the config and the
/// state: a picture half-written is as broken as a half-written TOML file,
/// and the bar would be the one to find out.
///
/// The rename is the commit, so everything that can fail has to fail first:
/// the bytes are written and flushed to the disk before the old file is
/// replaced. A failure at any point leaves the old file exactly as it was and
/// takes the scratch file with it.
pub fn write(path: &Path, body: impl AsRef<[u8]>) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no directory to write into", path.display()))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("cannot create {}", parent.display()))?;

    let scratch = parent.join(temporary_name(path));

    let written = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&scratch)?;
        file.write_all(body.as_ref())?;
        file.sync_all()
    })();

    if let Err(e) = written {
        let _ = std::fs::remove_file(&scratch);
        return Err(
            anyhow::Error::new(e).context(format!("cannot write beside {}", path.display()))
        );
    }

    if let Err(e) = std::fs::rename(&scratch, path) {
        let _ = std::fs::remove_file(&scratch);
        return Err(
            anyhow::Error::new(e).context(format!("cannot replace {}", path.display()))
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of this test's own, cleaned up after it.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("doca-atomic-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self) -> std::path::PathBuf {
            self.0.join("thing.toml")
        }

        /// Anything in the directory that is not the file itself: a scratch
        /// file that was left behind.
        fn leftovers(&self) -> Vec<String> {
            std::fs::read_dir(&self.0)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name != "thing.toml")
                .collect()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_file_written_where_no_directory_existed_is_still_written() {
        let scratch = Scratch::new("deep");
        let path = scratch.0.join("a/b/c/thing.toml");

        write(&path, "hello = 1").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello = 1");
    }

    #[test]
    fn a_write_leaves_nothing_beside_the_file_it_wrote() {
        let scratch = Scratch::new("tidy");

        write(&scratch.file(), "first = 1").unwrap();
        write(&scratch.file(), "second = 2").unwrap();

        assert!(
            scratch.leftovers().is_empty(),
            "a scratch file was left behind: {:?}",
            scratch.leftovers()
        );
    }

    /// The point of the rename: the new file replaces the old one whole,
    /// rather than being laid over the front of it.
    #[test]
    fn a_shorter_file_does_not_leave_the_tail_of_the_longer_one_showing() {
        let scratch = Scratch::new("replace");

        write(&scratch.file(), "a_very_long_first_write = 1").unwrap();
        write(&scratch.file(), "short = 2").unwrap();

        assert_eq!(std::fs::read_to_string(scratch.file()).unwrap(), "short = 2");
    }

    /// Two writers in the same process must not pick the same scratch name,
    /// or one would rename the other's half-written file into place.
    #[test]
    fn two_writes_never_share_a_scratch_name() {
        let path = std::path::Path::new("/tmp/doca/config.toml");

        assert_ne!(temporary_name(path), temporary_name(path));
    }

    #[test]
    fn a_path_with_no_directory_above_it_is_refused_rather_than_panicking() {
        assert!(write(std::path::Path::new("/"), "nothing").is_err());
    }
}
