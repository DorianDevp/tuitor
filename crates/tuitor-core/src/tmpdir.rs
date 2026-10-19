//! A temporary directory of its own for each adapter process tuitor starts, given to it as
//! `TMPDIR`. js-debug makes a directory in the temporary directory every time it starts
//! and never removes it, and keeps the profile of the browser it starts in there; with a
//! directory of its own, all of that goes when tuitor removes the directory.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// Longest a directory's path may be. js-debug and Chromium make Unix sockets right in it,
/// the longest Chromium's `.org.chromium.Chromium.XXXXXX/SingletonSocket` (46 bytes with
/// the separators), and a socket's path holds at most 103 bytes on macOS (107 on Linux).
const MAX_PATH: usize = 103 - 46;

/// Directories made by this process so far, which names the next: every tab's engine makes
/// them, so a session id would not do.
static MADE: AtomicU32 = AtomicU32::new(0);

/// Removed, with everything in it, when dropped.
#[derive(Debug)]
pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// `tuitor-<pid>-<n>` in the runtime directory, which is the user's own and emptied at
    /// logout, or else in the system's temporary directory. `None` when neither leaves room
    /// for the sockets or takes a directory; the adapter then uses the system's as before.
    pub(crate) fn create() -> Option<TempDir> {
        let mut bases = dirs::runtime_dir().into_iter().chain([std::env::temp_dir()]);
        bases.find_map(|base| Self::create_in(&base, &MADE))
    }

    /// The next of `made` that is free in `base`.
    fn create_in(base: &Path, made: &AtomicU32) -> Option<TempDir> {
        // A name already taken, by a tuitor that had this pid before a reboot say, is skipped:
        // only a directory made here is tuitor's to remove.
        for _ in 0..16 {
            let n = made.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("tuitor-{}-{n}", std::process::id()));
            if path.as_os_str().len() > MAX_PATH {
                return None;
            }
            match make_private(&path) {
                Ok(()) => return Some(TempDir { path }),
                Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
                Err(err) => {
                    tracing::warn!("temporary directory {}: {err}", path.display());
                    return None;
                }
            }
        }
        None
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if let Err(err) = std::fs::remove_dir_all(&self.path)
            && err.kind() != ErrorKind::NotFound
        {
            tracing::warn!("removing {}: {err}", self.path.display());
        }
    }
}

/// Makes the directory readable by its owner only; fails if anything is there already.
#[cfg(unix)]
fn make_private(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
fn make_private(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_of_its_own_goes_with_everything_in_it() {
        let base = std::env::temp_dir();
        let dir = TempDir::create_in(&base, &MADE).expect("temporary directory");
        let path = dir.path().to_path_buf();
        assert!(path.starts_with(&base) && path.is_dir());
        std::fs::create_dir_all(path.join("vscode-js-debug-x/.profile")).expect("what an adapter leaves");
        drop(dir);
        assert!(!path.exists());
    }

    #[test]
    fn a_base_too_long_for_sockets_is_passed_over() {
        let base = std::env::temp_dir().join("x".repeat(MAX_PATH));
        assert!(TempDir::create_in(&base, &MADE).is_none());
        assert!(!base.exists());
    }

    #[test]
    fn what_is_there_already_is_not_taken_over() {
        let base = std::env::temp_dir().join(format!("tuitor-tmpdir-test-{}", std::process::id()));
        std::fs::create_dir_all(&base).expect("base");
        // The next name, made by someone else first.
        let made = AtomicU32::new(7);
        let next = base.join(format!("tuitor-{}-7", std::process::id()));
        std::fs::create_dir(&next).expect("taken");
        let dir = TempDir::create_in(&base, &made).expect("another name");
        assert_ne!(dir.path(), next);
        drop(dir);
        assert!(next.is_dir(), "a directory tuitor did not make was removed");
        std::fs::remove_dir_all(base).ok();
    }
}
