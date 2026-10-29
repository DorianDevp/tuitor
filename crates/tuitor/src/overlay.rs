//! Popups that take over the keyboard: the command palette, the fuzzy finder, the
//! open-project picker, DOCTOR, INSTALL, PATHS and the list of breakpoint sets.

use std::path::{Path, PathBuf};

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use tuitor_core::breakpoints::{BreakpointId, Breakpoints};
use tuitor_core::config::project_relative;

use crate::callpaths::Paths;
use crate::command::{self, Arg, COMMANDS, Group, Spec};
use crate::doctor::Doctor;
use crate::keymap::Keymap;
use crate::picker::Picker;
use crate::search::Prompt;
use crate::sets::SetPick;

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
    /// The call trees through a line; dropped, it stops asking.
    Paths(Box<Paths>),
    /// The breakpoint sets, to make one active.
    Sets(SetPick),
    /// A value in full, wrapped: a variable's, a watch's, a line of OUTPUT.
    Value(ValueView),
}

/// A value too long for its pane, shown whole.
#[derive(Debug)]
pub struct ValueView {
    /// Where it is from: `$order`, `OUTPUT`.
    pub title: String,
    /// Its lines: a JSON value laid out, else the text as it is.
    pub lines: Vec<String>,
    pub scroll: usize,
}

impl ValueView {
    pub fn new(title: String, text: &str) -> ValueView {
        ValueView { title, lines: layout(text), scroll: 0 }
    }
}

/// `text` laid out to read: JSON as JSON (also when it is a quoted string holding JSON, as
/// adapters show a string), else as it is.
fn layout(text: &str) -> Vec<String> {
    let json = |text: &str| {
        serde_json::from_str::<serde_json::Value>(text.trim()).ok().and_then(|value| match value {
            serde_json::Value::String(inner) => serde_json::from_str::<serde_json::Value>(inner.trim()).ok(),
            value @ (serde_json::Value::Object(_) | serde_json::Value::Array(_)) => Some(value),
            _ => None,
        })
    };
    let pretty = |value: serde_json::Value| serde_json::to_string_pretty(&value).ok();
    // An evaluation in OUTPUT reads `expression = value`: the expression, then its value laid out.
    let laid = match json(text).and_then(pretty) {
        Some(laid) => laid,
        None => match text.rsplit_once(" = ").and_then(|(expr, value)| Some((expr, pretty(json(value)?)?))) {
            Some((expr, value)) => format!("{expr} =\n{value}"),
            None => text.to_owned(),
        },
    };
    laid.lines().map(str::to_owned).collect()
}

#[cfg(test)]
mod layout_tests {
    use super::layout;

    #[test]
    fn a_value_reads_laid_out_when_it_is_json_or_an_evaluation_of_json() {
        assert_eq!(layout("plain text"), ["plain text"]);
        assert_eq!(layout(r#"{"a":[1]}"#), ["{", r#"  "a": ["#, "    1", "  ]", "}"]);
        let evaluated = layout(r#"json_encode($x) = "{\"id\":[\"get\"]}""#);
        assert_eq!(evaluated[0], "json_encode($x) =");
        assert_eq!(evaluated[1..], ["{", r#"  "id": ["#, r#"    "get""#, "  ]", "}"]);
    }
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
        // A number alone goes to that line.
        if command::line_number(typed).is_ok()
            && let Some(spec) = COMMANDS.iter().find(|spec| spec.name == "line")
        {
            self.items = vec![PaletteItem { spec, key: keymap.any_key_for(spec.name), matched: Vec::new() }];
            return;
        }
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
        if command::line_number(&self.input).is_ok() {
            return Some(self.input.clone());
        }
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
            self.items.clear();
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
            .map(|b| (b.id, format!("{}:{}", project_relative(&b.path, workspace), b.line)))
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
    fn palette_takes_a_number_for_a_line() {
        let keymap = Keymap::defaults();
        let mut palette = Palette::new("42".into(), Vec::new());
        palette.refresh(&keymap);
        assert_eq!(palette.items.iter().map(|i| i.spec.name).collect::<Vec<_>>(), ["line"]);
        assert_eq!(palette.accept().as_deref(), Some("42"));
    }

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
