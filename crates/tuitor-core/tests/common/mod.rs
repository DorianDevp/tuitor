//! What the end-to-end tests share.

use std::path::{Path, PathBuf};

/// A directory of a test's own in the system's temporary directory, removed with everything
/// in it when dropped. A failing test panics, and the panic drops it on the way out, so a
/// failed run leaves nothing behind either.
pub struct TestDir {
    path: PathBuf,
}

impl TestDir {
    /// `tuitor-<name>-<pid>`, by its canonical path, as frames and breakpoints have them.
    pub fn new(name: &str) -> TestDir {
        let path = std::env::temp_dir().join(format!("tuitor-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&path).expect("test directory");
        TestDir { path: std::fs::canonicalize(path).expect("test directory path") }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}
