//! PATHS: the call paths a language server finds to the SOURCE line, each with the lines
//! worth a breakpoint, for the user to take all or some of.

use std::path::PathBuf;
use std::time::Instant;

use tuitor_code::{CallPath, Found, Query, Update};
use tuitor_core::Engine;
use tuitor_core::breakpoints::BreakpointId;

use crate::tab::TabId;

pub struct Paths {
    /// The tab whose project was asked, and whose engine gets the breakpoints.
    pub tab: TabId,
    /// `file:line` asked about.
    pub place: String,
    pub started: Instant,
    query: Option<Query>,
    /// What the query said last while it runs.
    pub progress: String,
    pub found: Option<Result<Found, String>>,
    pub selected: usize,
    /// The paths whose breakpoints `enter` adds; all of them at first.
    pub marked: Vec<bool>,
}

impl Paths {
    pub fn new(tab: TabId, place: String, query: Query) -> Paths {
        Paths {
            tab,
            place,
            started: Instant::now(),
            query: Some(query),
            progress: String::new(),
            found: None,
            selected: 0,
            marked: Vec::new(),
        }
    }

    /// Takes in what the query said.
    pub fn poll(&mut self) {
        let Some(query) = &mut self.query else { return };
        for update in query.updates() {
            match update {
                Update::Progress(text) => self.progress = text,
                Update::Done(found) => {
                    self.marked = vec![true; found.as_ref().map_or(0, |found| found.paths.len())];
                    self.found = Some(found);
                    self.query = None;
                }
            }
        }
    }

    pub fn paths(&self) -> &[CallPath] {
        match &self.found {
            Some(Ok(found)) => &found.paths,
            _ => &[],
        }
    }

    pub fn select(&mut self, delta: isize) {
        let len = self.paths().len();
        if len > 0 {
            self.selected = self.selected.saturating_add_signed(delta).min(len - 1);
        }
    }

    pub fn toggle(&mut self) {
        if let Some(marked) = self.marked.get_mut(self.selected) {
            *marked = !*marked;
        }
    }

    pub fn chosen(&self) -> Vec<&CallPath> {
        self.paths().iter().zip(&self.marked).filter(|(_, marked)| **marked).map(|(path, _)| path).collect()
    }
}

/// What taking paths did.
#[derive(Debug, Default, PartialEq)]
pub struct Taken {
    /// The breakpoints of each path taken, in its order: a line several paths share has one
    /// breakpoint, in each of their lists.
    pub paths: Vec<Vec<BreakpointId>>,
    pub added: usize,
    /// Lines that had a breakpoint already.
    pub existing: usize,
}

/// Puts breakpoints on the lines of `paths`, reusing one a line has. The one place that turns
/// paths into breakpoints: where each path is to become a set of its own.
pub fn take(engine: &mut Engine, paths: &[&CallPath]) -> Taken {
    let mut taken = Taken::default();
    let mut lines: Vec<(PathBuf, u32)> = Vec::new();
    for path in paths {
        let places: Vec<(PathBuf, u32)> = path.stops.iter().map(|stop| (stop.path.clone(), stop.line)).collect();
        let mut ids = Vec::new();
        for ((id, new), place) in engine.add_breakpoints(&places).into_iter().zip(places) {
            if !ids.contains(&id) {
                ids.push(id);
            }
            if !lines.contains(&place) {
                taken.added += usize::from(new);
                taken.existing += usize::from(!new);
                lines.push(place);
            }
        }
        taken.paths.push(ids);
    }
    taken
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tuitor_code::{Root, Step, Stop, Why};
    use tuitor_core::EngineOptions;
    use tuitor_core::config::Adapters;

    use super::*;

    fn path(lines: &[(&str, u32)]) -> CallPath {
        let step = Step { name: "f".into(), path: "/w/a.php".into(), line: 1, observed: false };
        let stops = lines
            .iter()
            .map(|(file, line)| Stop { path: Path::new("/w").join(file), line: *line, why: Why::Entry, step: 0 })
            .collect();
        CallPath { steps: vec![step], root: Root::Uncalled, stops }
    }

    #[test]
    fn a_line_on_several_paths_is_one_breakpoint_and_one_there_is_kept() {
        let options = EngineOptions { adapters: Adapters::default(), workspace: "/w".into(), state_file: None };
        let (mut engine, _) = Engine::new(options);
        engine.toggle_breakpoint(Path::new("/w/b.php"), 9);
        let first = path(&[("a.php", 3), ("b.php", 9)]);
        let second = path(&[("a.php", 3), ("c.php", 5)]);
        let taken = take(&mut engine, &[&first, &second]);
        // #1 was there; a.php:3 is #2 on both paths.
        assert_eq!(taken, Taken { paths: vec![vec![2, 1], vec![2, 3]], added: 2, existing: 1 });
        let lines: Vec<(String, u32)> =
            engine.breakpoints().iter().map(|bp| (bp.path.display().to_string(), bp.line)).collect();
        assert_eq!(lines, [("/w/a.php".into(), 3), ("/w/b.php".into(), 9), ("/w/c.php".into(), 5)]);
    }
}
