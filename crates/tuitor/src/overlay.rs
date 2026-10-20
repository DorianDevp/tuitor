//! Popups that take over the keyboard: the command palette, the fuzzy finder, the
//! open-project picker, DOCTOR and INSTALL.

use std::path::{Path, PathBuf};

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use tuitor_core::breakpoints::{BreakpointId, Breakpoints};

use crate::command::{self, Arg, COMMANDS, Group, Spec};
use crate::doctor::Doctor;
use crate::keymap::Keymap;
use crate::picker::Picker;
use crate::search::Prompt;

const MAX_FILES: usize = 100_000;
const MAX_HITS: usize = 40;

pub enum Overlay {
    Palette(Palette),
    Finder(Finder),
    Picker(Picker),
    Doctor(Doctor),
    /// INSTALL, which shows [`App::installer`]: an installation goes on with it closed.
    Install,
    /// The search being typed; it shows on the status line, over SOURCE.
    Search(Prompt),
}

/// Score and matched character positions, or `None` when the pattern does not match.
pub(crate) fn fuzzy(matcher: &mut Matcher, pattern: &Pattern, text: &str) -> Option<(u32, Vec<u32>)> {
    let mut buf = Vec::new();
    let haystack = Utf32Str::new(text, &mut buf);
    let mut indices = Vec::new();
    let score = pattern.indices(haystack, matcher, &mut indices)?;
    indices.sort_unstable();
    indices.dedup();
    Some((score, indices))
}

// ---- palette ------------------------------------------------------------------------------

pub struct PaletteItem {
    pub spec: &'static Spec,
    pub key: Option<String>,
    pub matched: Vec<u32>,
}

pub struct Palette {
    pub input: String,
    pub selected: usize,
    pub items: Vec<PaletteItem>,
    pub recent: Vec<String>,
}

impl Palette {
    pub fn new(input: String, recent: Vec<String>) -> Palette {
        Palette { input, selected: 0, items: Vec::new(), recent }
    }

    /// Text after the slash; once a full command and a space are typed, only that command matches.
    pub fn refresh(&mut self, keymap: &Keymap) {
        let typed = self.input.trim_start_matches('/');
        self.selected = 0;
        if let Some((spec, _)) = command::split(typed).filter(|_| typed.contains(' ') || command::find(typed).is_some())
        {
            self.items = vec![PaletteItem { spec, key: keymap.any_key_for(spec.name), matched: Vec::new() }];
            return;
        }
        let shown = |spec: &Spec| spec.group != Group::Navigate || !typed.is_empty();
        if typed.is_empty() {
            self.items = COMMANDS
                .iter()
                .filter(|s| shown(s))
                .map(|spec| PaletteItem { spec, key: keymap.any_key_for(spec.name), matched: Vec::new() })
                .collect();
            return;
        }
        let mut matcher = Matcher::new(Config::DEFAULT);
        let pattern = Pattern::parse(typed, CaseMatching::Ignore, Normalization::Smart);
        let mut scored: Vec<(u32, PaletteItem)> = COMMANDS
            .iter()
            .filter_map(|spec| {
                let (score, matched) = fuzzy(&mut matcher, &pattern, spec.name)?;
                Some((score, PaletteItem { spec, key: keymap.any_key_for(spec.name), matched }))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.spec.name.len().cmp(&b.1.spec.name.len())));
        self.items = scored.into_iter().map(|(_, item)| item).collect();
    }

    pub fn select(&mut self, delta: isize) {
        if !self.items.is_empty() {
            self.selected = (self.selected as isize + delta).rem_euclid(self.items.len() as isize) as usize;
        }
    }

    pub fn complete(&mut self) {
        if let Some(item) = self.items.get(self.selected) {
            let space = if item.spec.arg == Arg::None { "" } else { " " };
            self.input = format!("/{}{space}", item.spec.name);
        }
    }

    /// The line to run, or `None` when an argument is still needed (the input is then prepared).
    pub fn accept(&mut self) -> Option<String> {
        if let Some((spec, arg)) = command::split(&self.input) {
            if arg.is_none() && matches!(spec.arg, Arg::Required(_)) {
                self.input = format!("/{} ", spec.name);
                return None;
            }
            return Some(self.input.clone());
        }
        let spec = self.items.get(self.selected)?.spec;
        if matches!(spec.arg, Arg::Required(_)) {
            self.input = format!("/{} ", spec.name);
            self.items.truncate(0);
            self.items.push(PaletteItem { spec, key: None, matched: Vec::new() });
            return None;
        }
        Some(spec.name.to_owned())
    }
}

// ---- finder -------------------------------------------------------------------------------

pub struct FileHit {
    pub rel: String,
    pub matched: Vec<u32>,
}

pub struct BpHit {
    pub id: BreakpointId,
    pub label: String,
    pub matched: Vec<u32>,
}

pub enum Choice {
    File { path: PathBuf, line: Option<u32> },
    Breakpoint(BreakpointId),
}

pub struct Finder {
    pub input: String,
    pub selected: usize,
    pub files: Vec<FileHit>,
    pub bps: Vec<BpHit>,
    pub total_files: usize,
    workspace: PathBuf,
}

impl Finder {
    pub fn new(input: String) -> Finder {
        Finder { input, selected: 0, files: Vec::new(), bps: Vec::new(), total_files: 0, workspace: PathBuf::new() }
    }

    /// `order:29` searches for "order" and remembers line 29.
    pub fn query(&self) -> (&str, Option<u32>) {
        match self.input.rsplit_once(':') {
            Some((text, line)) if line.chars().all(|c| c.is_ascii_digit()) => (text, line.parse().ok()),
            _ => (&self.input, None),
        }
    }

    pub fn refresh(&mut self, files: &[String], breakpoints: &Breakpoints, workspace: &Path) {
        self.workspace = workspace.to_path_buf();
        self.total_files = files.len();
        self.selected = 0;
        let (text, _) = self.query();
        let text = text.trim().to_owned();

        let labels: Vec<(BreakpointId, String)> = breakpoints
            .iter()
            .map(|b| {
                let rel = b.path.strip_prefix(workspace).unwrap_or(&b.path);
                (b.id, format!("{}:{}", rel.display(), b.line))
            })
            .collect();

        if text.is_empty() {
            self.bps = labels.into_iter().map(|(id, label)| BpHit { id, label, matched: Vec::new() }).collect();
            self.files =
                files.iter().take(MAX_HITS).map(|rel| FileHit { rel: rel.clone(), matched: Vec::new() }).collect();
            return;
        }

        let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
        let pattern = Pattern::parse(&text, CaseMatching::Smart, Normalization::Smart);
        let mut hits: Vec<(u32, FileHit)> = files
            .iter()
            .filter_map(|rel| {
                let (score, matched) = fuzzy(&mut matcher, &pattern, rel)?;
                Some((score, FileHit { rel: rel.clone(), matched }))
            })
            .collect();
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.rel.len().cmp(&b.1.rel.len())));
        hits.truncate(MAX_HITS);
        self.files = hits.into_iter().map(|(_, h)| h).collect();

        let mut bp_hits: Vec<(u32, BpHit)> = labels
            .into_iter()
            .filter_map(|(id, label)| {
                let (score, matched) = fuzzy(&mut matcher, &pattern, &label)?;
                Some((score, BpHit { id, label, matched }))
            })
            .collect();
        bp_hits.sort_by_key(|h| std::cmp::Reverse(h.0));
        self.bps = bp_hits.into_iter().map(|(_, h)| h).collect();
    }

    pub fn len(&self) -> usize {
        self.files.len() + self.bps.len()
    }

    pub fn select(&mut self, delta: isize) {
        let len = self.len();
        if len > 0 {
            self.selected = (self.selected as isize + delta).rem_euclid(len as isize) as usize;
        }
    }

    pub fn choice(&self) -> Option<Choice> {
        let (_, line) = self.query();
        if let Some(hit) = self.files.get(self.selected) {
            return Some(Choice::File { path: self.workspace.join(&hit.rel), line });
        }
        let hit = self.bps.get(self.selected - self.files.len())?;
        Some(Choice::Breakpoint(hit.id))
    }
}

/// Files under the workspace, honouring .gitignore, as relative paths.
pub fn workspace_files(root: &Path) -> Vec<String> {
    let mut files: Vec<String> = ignore::WalkBuilder::new(root)
        .hidden(true)
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|e| e.path().strip_prefix(root).ok().map(|p| p.to_string_lossy().into_owned()))
        .take(MAX_FILES)
        .collect();
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_completes_and_asks_for_arguments() {
        let keymap = Keymap::defaults();
        let mut palette = Palette::new("/wat".into(), Vec::new());
        palette.refresh(&keymap);
        assert_eq!(palette.items[0].spec.name, "watch add");
        assert_eq!(palette.accept(), Some("watch add".into()));

        let mut palette = Palette::new("/eval".into(), Vec::new());
        palette.refresh(&keymap);
        assert_eq!(palette.accept(), None);
        assert_eq!(palette.input, "/eval ");
        palette.input.push_str("$total * 2");
        assert_eq!(palette.accept(), Some("/eval $total * 2".into()));
    }

    #[test]
    fn finder_query_with_line() {
        let mut finder = Finder::new("ordserv:141".into());
        let files = vec!["src/Service/OrderService.php".to_owned(), "src/Entity/Order.php".to_owned()];
        finder.refresh(&files, &Breakpoints::default(), Path::new("/w"));
        assert_eq!(finder.files[0].rel, "src/Service/OrderService.php");
        assert!(matches!(finder.choice(), Some(Choice::File { line: Some(141), .. })));
    }
}
