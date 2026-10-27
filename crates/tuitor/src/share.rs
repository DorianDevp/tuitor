//! Breakpoints to share: `/bp export` writes a project's breakpoints to a file, paths from the
//! project's directory, each with what its line read; `/bp import` reads such a file into a set
//! of its own, and each breakpoint finds its line in the files as they are here, as it does
//! when a file changes. A colleague's breakpoints, or one's own from another checkout, land on
//! their lines though the lines moved.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tuitor_core::follow::Anchor;

use crate::app::{App, Outcome};
use crate::sets::name;

/// The file `/bp export` writes when none is named, in the project's directory.
const DEFAULT_FILE: &str = "tuitor-breakpoints.json";

#[derive(Debug, Serialize, Deserialize)]
struct Shared {
    tuitor: u32,
    breakpoints: Vec<SharedBreakpoint>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SharedBreakpoint {
    /// From the project's directory.
    file: PathBuf,
    line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    anchor: Option<Anchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    log_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hit_condition: Option<String>,
}

impl App {
    /// `/bp export [file]`: the tab's breakpoints, those of its project's files, to `file`.
    pub(crate) fn bp_export(&mut self, file: Option<String>) -> Outcome {
        let tab = self.tab();
        let dir = tab.engine.workspace().to_path_buf();
        let breakpoints: Vec<SharedBreakpoint> = tab
            .engine
            .breakpoints()
            .iter()
            .filter_map(|bp| {
                Some(SharedBreakpoint {
                    file: bp.path.strip_prefix(&dir).ok()?.to_path_buf(),
                    line: bp.line,
                    anchor: bp.anchor.clone(),
                    condition: bp.condition.clone(),
                    log_message: bp.log_message.clone(),
                    hit_condition: bp.hit_condition.clone(),
                })
            })
            .collect();
        if breakpoints.is_empty() {
            return Err("no breakpoint in this project's files to export".into());
        }
        let path = target(&dir, file.as_deref().unwrap_or(DEFAULT_FILE));
        let text = serde_json::to_string_pretty(&Shared { tuitor: 1, breakpoints }).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| format!("writing {}: {e}", path.display()))?;
        Ok(format!("exported the breakpoints to {}", path.display()))
    }

    /// `/bp import <file>`: the breakpoints of `file` in a new set, each on its line as the
    /// files are here; the set is not active, so they record.
    pub(crate) fn bp_import(&mut self, file: Option<String>) -> Outcome {
        let dir = self.tab().engine.workspace().to_path_buf();
        let path = target(&dir, file.as_deref().unwrap_or(DEFAULT_FILE));
        let text = std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
        let shared: Shared = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let engine = &mut self.tab_mut().engine;
        let set = engine.new_set();
        let (mut made, mut missing) = (0, 0);
        for shared in shared.breakpoints {
            let file = dir.join(&shared.file);
            if !file.is_file() {
                missing += 1;
                continue;
            }
            let Ok((id, _)) = engine.put_in_set(&file, shared.line, set) else { continue };
            let _ = engine.update_breakpoint(id, |bp| {
                bp.anchor = shared.anchor.or(bp.anchor.take());
                bp.condition = shared.condition;
                bp.log_message = shared.log_message;
                bp.hit_condition = shared.hit_condition;
            });
            // The anchor brought along finds the line in this checkout's file.
            engine.follow_now(&file);
            made += 1;
        }
        let lost = engine.breakpoints().members(set).filter(|b| b.lost).count();
        let set_name = name(engine.breakpoints(), set);
        let mut said = format!("{set_name}: imported {made} breakpoints, recording");
        if lost > 0 {
            said.push_str(&format!("; {lost} found no line like theirs (⊘)"));
        }
        if missing > 0 {
            said.push_str(&format!("; {missing} in files this project does not have"));
        }
        Ok(said)
    }
}

/// `file` from the project's directory, unless it is absolute.
fn target(dir: &Path, file: &str) -> PathBuf {
    let file = Path::new(file.trim());
    if file.is_absolute() { file.to_path_buf() } else { dir.join(file) }
}
