//! Breakpoints and watches are remembered per workspace, outside the repository
//! (`~/.local/state/tuitor/<workspace>.json`), so they never show up in `git status`.

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

/// State file for a workspace: the absolute path with separators escaped.
pub fn state_file(workspace: &Path) -> Option<PathBuf> {
    let name: String = workspace
        .to_string_lossy()
        .trim_start_matches('/')
        .chars()
        .map(|c| if c == '/' || c == '\\' || c == ':' { '%' } else { c })
        .collect();
    Some(dirs::state_dir().or_else(dirs::data_local_dir)?.join("tuitor").join(format!("{name}.json")))
}

pub fn load(path: &Path) -> Result<Persisted> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Persisted::default()),
        Err(err) => Err(err).with_context(|| format!("reading {}", path.display())),
    }
}

pub fn save(path: &Path, state: &Persisted) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(state)?).with_context(|| format!("writing {}", tmp.display()))?;
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
}
