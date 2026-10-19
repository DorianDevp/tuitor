//! Breakpoints and watches are remembered per workspace, outside the repository
//! (`~/.local/state/tuitor/<workspace>.json`), so they never show up in `git status`.
//! The projects opened recently are listed there too, for the open-project picker.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::breakpoints::Breakpoint;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Persisted {
    #[serde(default)]
    pub breakpoints: Vec<Breakpoint>,
    #[serde(default)]
    pub watches: Vec<String>,
}

const RECENT_PROJECTS: usize = 30;

fn state_dir() -> Option<PathBuf> {
    Some(dirs::state_dir().or_else(dirs::data_local_dir)?.join("tuitor"))
}

/// State file for a workspace: the absolute path with separators escaped.
pub fn state_file(workspace: &Path) -> Option<PathBuf> {
    let name: String = workspace
        .to_string_lossy()
        .trim_start_matches('/')
        .chars()
        .map(|c| if c == '/' || c == '\\' || c == ':' { '%' } else { c })
        .collect();
    Some(state_dir()?.join(format!("{name}.json")))
}

/// Projects opened recently, newest first.
pub fn recent_file() -> Option<PathBuf> {
    Some(state_dir()?.join("recent-projects.json"))
}

pub fn load_recent(path: &Path) -> Result<Vec<PathBuf>> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(err).with_context(|| format!("reading {}", path.display())),
    }
}

/// Moves `project` to the front of the list at `path` and returns the list. The file is
/// read again first, so projects another tuitor opened meanwhile stay in it.
pub fn remember_recent(path: &Path, project: &Path) -> Result<Vec<PathBuf>> {
    // A list that cannot be read is replaced: it only saves typing.
    let mut recent = load_recent(path).unwrap_or_default();
    recent.retain(|p| p != project);
    recent.insert(0, project.to_path_buf());
    recent.truncate(RECENT_PROJECTS);
    write_json(path, &recent)?;
    Ok(recent)
}

pub fn load(path: &Path) -> Result<Persisted> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Persisted::default()),
        Err(err) => Err(err).with_context(|| format!("reading {}", path.display())),
    }
}

pub fn save(path: &Path, state: &Persisted) -> Result<()> {
    write_json(path, state)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_file_escapes_workspace_path() {
        let file = state_file(Path::new("/home/me/shop")).expect("state dir");
        assert_eq!(file.file_name().and_then(|n| n.to_str()), Some("home%me%shop.json"));
    }

    #[test]
    fn round_trips_and_tolerates_missing_file() {
        let dir = std::env::temp_dir().join(format!("tuitor-persist-{}", std::process::id()));
        let file = dir.join("state.json");
        assert!(load(&file).expect("missing is fine").breakpoints.is_empty());
        let state = Persisted { breakpoints: vec![], watches: vec!["$total".into()] };
        save(&file, &state).expect("save");
        assert_eq!(load(&file).expect("load").watches, ["$total"]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn recent_projects_are_newest_first_without_duplicates() {
        let dir = std::env::temp_dir().join(format!("tuitor-recent-{}", std::process::id()));
        let file = dir.join("recent-projects.json");
        assert!(load_recent(&file).expect("missing is fine").is_empty());
        for project in ["/w/api", "/w/web", "/w/api"] {
            remember_recent(&file, Path::new(project)).expect("remember");
        }
        assert_eq!(load_recent(&file).expect("load"), [PathBuf::from("/w/api"), PathBuf::from("/w/web")]);
        for n in 0..40 {
            remember_recent(&file, Path::new(&format!("/w/{n}"))).expect("remember");
        }
        let recent = load_recent(&file).expect("load");
        assert_eq!((recent.len(), recent[0].as_path()), (RECENT_PROJECTS, Path::new("/w/39")));
        std::fs::remove_dir_all(dir).ok();
    }
}
