//! Source files read from disk, cached, and re-read when they change on disk; and the
//! texts adapters give for sources they have themselves.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use tuitor_core::session::SessionId;

use crate::tab::TabId;

pub struct SourceFile {
    pub lines: Vec<String>,
    modified: Option<SystemTime>,
}

impl SourceFile {
    fn new(text: &str, modified: Option<SystemTime>) -> SourceFile {
        SourceFile { lines: text.lines().map(|l| l.replace('\t', "    ")).collect(), modified }
    }
}

/// An adapter's source: its tab, session and reference.
pub type AdapterSource = (TabId, SessionId, i64);

#[derive(Default)]
pub struct Sources {
    files: HashMap<PathBuf, Rc<SourceFile>>,
    adapter: HashMap<AdapterSource, Rc<SourceFile>>,
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
        let file = Rc::new(SourceFile::new(&String::from_utf8_lossy(&bytes), modified));
        self.files.insert(path.to_path_buf(), file.clone());
        Some(file)
    }

    /// The lines of `text`, which the adapter gave for `source`; split once, as it does not
    /// change.
    pub fn adapter(&mut self, source: AdapterSource, text: &str) -> Rc<SourceFile> {
        self.adapter.entry(source).or_insert_with(|| Rc::new(SourceFile::new(text, None))).clone()
    }
}
