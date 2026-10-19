//! The open-project picker: projects opened recently, matched fuzzily, or a directory
//! typed as a path (`~/…`, `/…`, `../…`) and completed from what is on disk.

use std::path::{Path, PathBuf};

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};
use tuitor_core::config::home_relative;
use tuitor_core::project::{Kind, detect};

use crate::overlay::fuzzy;
use crate::paths::expand;

const MAX_ROWS: usize = 40;

/// A directory the input may mean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// As shown: `~/…` for recent projects, the typed form for paths.
    pub label: String,
    pub path: PathBuf,
    /// Characters of `label` the input matched.
    pub matched: Vec<u32>,
}

pub struct PickerRow {
    pub candidate: Candidate,
    pub kinds: Vec<Kind>,
    /// Open in a tab already; choosing it shows that tab.
    pub open: bool,
}

pub struct Picker {
    pub input: String,
    pub selected: usize,
    pub rows: Vec<PickerRow>,
    recent: Vec<PathBuf>,
    open: Vec<PathBuf>,
    home: Option<PathBuf>,
    /// Where a relative path starts: the current project.
    base: PathBuf,
}

impl Picker {
    pub fn new(recent: Vec<PathBuf>, open: Vec<PathBuf>, home: Option<PathBuf>, base: PathBuf) -> Picker {
        let mut picker = Picker { input: String::new(), selected: 0, rows: Vec::new(), recent, open, home, base };
        picker.refresh();
        picker
    }

    /// A path is being typed, rather than a recent project searched for.
    pub fn typing_path(&self) -> bool {
        looks_like_path(&self.input)
    }

    pub fn refresh(&mut self) {
        self.selected = 0;
        let candidates = if self.typing_path() {
            complete(&self.input, self.home.as_deref(), &self.base)
        } else {
            // Projects open already come last: choosing one only shows its tab.
            let mut recent = self.recent_matches();
            recent.sort_by_key(|candidate| self.open.contains(&candidate.path));
            recent
        };
        self.rows = candidates
            .into_iter()
            .take(MAX_ROWS)
            .map(|candidate| PickerRow {
                kinds: detect(&candidate.path),
                open: self.open.contains(&candidate.path),
                candidate,
            })
            .collect();
    }

    fn recent_matches(&self) -> Vec<Candidate> {
        let labelled = self.recent.iter().map(|path| (home_relative(path), path));
        let query = self.input.trim();
        if query.is_empty() {
            return labelled
                .map(|(label, path)| Candidate { label, path: path.clone(), matched: Vec::new() })
                .collect();
        }
        let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let mut scored: Vec<(u32, Candidate)> = labelled
            .filter_map(|(label, path)| {
                let (score, matched) = fuzzy(&mut matcher, &pattern, &label)?;
                Some((score, Candidate { label, path: path.clone(), matched }))
            })
            .collect();
        // Stable, so equal scores keep the most recent first.
        scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        scored.into_iter().map(|(_, candidate)| candidate).collect()
    }

    pub fn select(&mut self, delta: isize) {
        if !self.rows.is_empty() {
            self.selected = (self.selected as isize + delta).rem_euclid(self.rows.len() as isize) as usize;
        }
    }

    /// `tab`: the selected directory becomes the input, ready for the next level.
    pub fn complete(&mut self) {
        if let Some(row) = self.rows.get(self.selected) {
            self.input = format!("{}/", row.candidate.label.trim_end_matches('/'));
            self.refresh();
        }
    }

    pub fn choice(&self) -> Option<&Path> {
        Some(self.rows.get(self.selected)?.candidate.path.as_path())
    }
}

/// `/`, `~`, `.` and `..` start a path; anything else searches the recent projects.
fn looks_like_path(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with(['/', '~']) || matches!(text, "." | "..") || text.starts_with("./") || text.starts_with("../")
}

/// Directories `typed` may mean: itself when it is one, then the directories beside its
/// last part whose names match that part. Hidden ones only when the part starts with a dot.
pub fn complete(typed: &str, home: Option<&Path>, base: &Path) -> Vec<Candidate> {
    let typed = typed.trim_start();
    let mut found = Vec::new();
    let itself = expand(typed, home, base);
    if itself.is_dir() {
        found.push(Candidate { label: typed.to_owned(), path: canonical(&itself), matched: Vec::new() });
    }
    // "~", "." and ".." name a directory; completing starts after their slash.
    let Some(slash) = typed.rfind('/') else { return found };
    let (parent, part) = typed.split_at(slash + 1);
    let Ok(entries) = std::fs::read_dir(expand(parent, home, base)) else { return found };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.') || part.starts_with('.'))
        .collect();
    names.sort();
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(part, CaseMatching::Smart, Normalization::Smart);
    let offset = parent.chars().count() as u32;
    let mut matches: Vec<(u32, Candidate)> = names
        .into_iter()
        .filter_map(|name| {
            let (score, matched) =
                if part.is_empty() { (0, Vec::new()) } else { fuzzy(&mut matcher, &pattern, &name)? };
            let path = canonical(&expand(&format!("{parent}{name}"), home, base));
            let matched = matched.into_iter().map(|i| i + offset).collect();
            Some((score, Candidate { label: format!("{parent}{name}"), path, matched }))
        })
        .collect();
    matches.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    for (_, candidate) in matches {
        if !found.iter().any(|f: &Candidate| f.path == candidate.path) {
            found.push(candidate);
        }
    }
    found
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `<tmp>/<test>/shop` with api, apps, web and .git directories and a file.
    fn shop(test: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("tuitor-picker-{test}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        for dir in ["shop/api", "shop/apps", "shop/web", "shop/.git"] {
            std::fs::create_dir_all(root.join(dir)).expect("dir");
        }
        std::fs::write(root.join("shop/notes.txt"), "").expect("file");
        canonical(&root.join("shop"))
    }

    fn labels(found: &[Candidate]) -> Vec<&str> {
        found.iter().map(|c| c.label.as_str()).collect()
    }

    #[test]
    fn paths_complete_from_the_directories_on_disk() {
        let shop = shop("complete");
        let typed = format!("{}/", shop.display());
        let found = complete(&typed, None, Path::new("/"));
        let listed: Vec<String> = ["", "api", "apps", "web"].iter().map(|n| format!("{typed}{n}")).collect();
        assert_eq!(labels(&found), listed);
        assert_eq!(found[0].path, shop);

        let found = complete(&format!("{typed}ap"), None, Path::new("/"));
        assert_eq!(labels(&found), [format!("{typed}api"), format!("{typed}apps")]);
        let offset = typed.chars().count() as u32;
        assert_eq!(found[0].matched, [offset, offset + 1]);

        // The directory typed comes first and is not listed twice.
        let found = complete(&format!("{typed}api"), None, Path::new("/"));
        assert_eq!(labels(&found), [format!("{typed}api")]);
        assert_eq!(found[0].path, shop.join("api"));
        std::fs::remove_dir_all(shop.parent().expect("root")).ok();
    }

    #[test]
    fn tilde_dot_dot_and_hidden_directories() {
        let shop = shop("tilde");
        let home = shop.as_path();
        assert_eq!(labels(&complete("~/we", Some(home), Path::new("/"))), ["~/web"]);
        assert_eq!(complete("~", Some(home), Path::new("/"))[0].path, shop);
        let found = complete("../web", None, &shop.join("api"));
        assert_eq!((labels(&found), found[0].path.clone()), (vec!["../web"], shop.join("web")));
        assert!(!labels(&complete("~/", Some(home), Path::new("/"))).contains(&"~/.git"));
        assert_eq!(labels(&complete("~/.g", Some(home), Path::new("/"))), ["~/.git"]);
        assert!(complete("~/nothing/x", Some(home), Path::new("/")).is_empty());
        std::fs::remove_dir_all(shop.parent().expect("root")).ok();
    }

    #[test]
    fn recent_projects_are_matched_until_a_path_is_typed() {
        let shop = shop("recent");
        let recent = vec![shop.join("web"), shop.join("api")];
        let mut picker = Picker::new(recent, vec![shop.join("web")], None, shop.clone());
        // The open one, though more recent, comes last.
        assert_eq!(picker.choice(), Some(shop.join("api").as_path()));
        assert!(!picker.rows[0].open && picker.rows[1].open);

        picker.input = "ap".into();
        picker.refresh();
        assert_eq!(picker.choice(), Some(shop.join("api").as_path()));

        // Completing a recent project turns the input into its path.
        picker.complete();
        assert!(picker.typing_path());
        assert_eq!(picker.input, format!("{}/", shop.join("api").display()));
        assert_eq!(picker.choice(), Some(shop.join("api").as_path()));
        std::fs::remove_dir_all(shop.parent().expect("root")).ok();
    }

    #[test]
    fn what_starts_a_path() {
        for path in ["/srv", "~", "~/x", ".", "..", "./x", "../x"] {
            assert!(looks_like_path(path), "{path}");
        }
        for query in ["api", "web/src", ".config"] {
            assert!(!looks_like_path(query), "{query}");
        }
    }
}
