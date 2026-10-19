//! Source files read from disk, cached, and re-read when they change on disk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

pub struct SourceFile {
    pub lines: Vec<String>,
    modified: Option<SystemTime>,
}

#[derive(Default)]
pub struct Sources {
    files: HashMap<PathBuf, Rc<SourceFile>>,
}

impl Sources {
    /// The file's lines, or `None` when it cannot be read (remote path, deleted file).
    pub fn get(&mut self, path: &Path) -> Option<Rc<SourceFile>> {
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if let Some(file) = self.files.get(path)
            && file.modified == modified
        {
            return Some(file.clone());
        }
        let bytes = std::fs::read(path).ok()?;
        let text = String::from_utf8_lossy(&bytes);
        let lines = text.lines().map(|l| l.replace('\t', "    ")).collect();
        let file = Rc::new(SourceFile { lines, modified });
        self.files.insert(path.to_path_buf(), file.clone());
        Some(file)
    }
}
