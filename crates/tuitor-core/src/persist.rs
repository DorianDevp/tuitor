//! Breakpoints and watches are remembered per workspace, outside the repository
//! (`~/.local/state/tuitor/<workspace>.json`), so they never show up in `git status`.
//! The projects opened recently are listed there too, for the open-project picker, and
//! the flow of recent runs (`flows/<workspace>.json`), which FLOW predicts from.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::breakpoints::Breakpoint;
use crate::flow::Run;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Persisted {
    #[serde(default)]
    pub breakpoints: Vec<Breakpoint>,
    #[serde(default)]
    pub watches: Vec<String>,
}

const RECENT_PROJECTS: usize = 30;
/// Runs remembered per workspace; older ones are dropped.
const RUNS_KEPT: usize = 30;

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

/// The runs remembered for a workspace, named like its [`state_file`].
pub fn flows_file(workspace: &Path) -> Option<PathBuf> {
    let file = state_file(workspace)?;
    Some(file.parent()?.join("flows").join(file.file_name()?))
}

/// Runs remembered at `path`, oldest first.
pub fn load_runs(path: &Path) -> Result<Vec<Run>> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(err).with_context(|| format!("reading {}", path.display())),
    }
}

/// Saves `run` at `path` in place of the run with its number, or as the newest, and returns
/// the runs. The file is read again first, so runs another tuitor recorded meanwhile stay.
pub fn remember_run(path: &Path, run: &Run) -> Result<Vec<Run>> {
    // Runs that cannot be read are replaced: they only feed predictions.
    let mut runs = load_runs(path).unwrap_or_default();
    match runs.iter_mut().find(|r| r.number == run.number) {
        Some(slot) => *slot = run.clone(),
        None => runs.push(run.clone()),
    }
    let excess = runs.len().saturating_sub(RUNS_KEPT);
    runs.drain(..excess);
    write_json(path, &runs)?;
    Ok(runs)
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
    fn runs_round_trip_replace_their_own_record_and_keep_the_latest() {
        use crate::flow::{Location, Observed, ObservedFile, Stop};

        let dir = std::env::temp_dir().join(format!("tuitor-runs-{}", std::process::id()));
        let file = flows_file(Path::new("/home/me/shop")).expect("state dir");
        assert_eq!(file.parent().and_then(|d| d.file_name()).and_then(|n| n.to_str()), Some("flows"));
        let file = dir.join("flows.json");
        assert!(load_runs(&file).expect("missing is fine").is_empty());

        let top = Location { path: "/w/src/Service.php".into(), line: 19 };
        let observed = Observed {
            stops: vec![Stop { top: Some(top.clone()), hit: true }],
            files: vec![ObservedFile {
                path: top.path.clone(),
                library: false,
                functions: vec![("Service->assign".into(), 1)],
                first_seen: [("Service->assign".into(), 1)].into(),
                lines: [19].into(),
                stops: vec![1],
            }],
            edges: vec![],
        };
        let run = Run::new(1, "2026-10-20 01:12".into(), observed);
        remember_run(&file, &run).expect("save");
        assert_eq!(load_runs(&file).expect("load"), std::slice::from_ref(&run));

        let mut longer = run.clone();
        longer.observed.stops.push(Stop { top: None, hit: false });
        remember_run(&file, &longer).expect("update");
        assert_eq!(load_runs(&file).expect("load"), [longer]);

        for number in 2..40 {
            remember_run(&file, &Run { number, ..run.clone() }).expect("save");
        }
        let runs = load_runs(&file).expect("load");
        assert_eq!((runs.len(), runs[0].number, runs[RUNS_KEPT - 1].number), (RUNS_KEPT, 10, 39));
        assert_eq!(runs[0].first_hit, Some(top));
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
