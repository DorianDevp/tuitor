//! Source files read from disk, cached, and re-read when they change on disk; and the
//! texts adapters give for sources they have themselves.

use std::cell::OnceCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use tuitor_core::session::SessionId;

use crate::outline::{self, Symbol};
use crate::tab::TabId;
use crate::ui::syntax::Lang;

pub struct SourceFile {
    pub lines: Vec<String>,
    modified: Option<SystemTime>,
    lang: Lang,
    /// Read when first asked for, once per version of the file.
    outline: OnceCell<Vec<Symbol>>,
}

impl SourceFile {
    fn new(text: &str, modified: Option<SystemTime>, lang: Lang) -> SourceFile {
        let lines = text.lines().map(|l| l.replace('\t', "    ")).collect();
        SourceFile { lines, modified, lang, outline: OnceCell::new() }
    }

    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// The functions and methods in the file.
    pub fn outline(&self) -> &[Symbol] {
        self.outline.get_or_init(|| outline::outline(&self.lines, self.lang))
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
        let file = Rc::new(SourceFile::new(&String::from_utf8_lossy(&bytes), modified, Lang::from_path(path)));
        self.files.insert(path.to_path_buf(), file.clone());
        Some(file)
    }

    /// The lines of `text`, which the adapter gave for `source`; split once, as it does not
    /// change.
    pub fn adapter(&mut self, source: AdapterSource, text: &str) -> Rc<SourceFile> {
        self.adapter.entry(source).or_insert_with(|| Rc::new(SourceFile::new(text, None, Lang::Plain))).clone()
    }
}
