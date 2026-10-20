//! PATHS: the call paths a language server finds to the SOURCE line, each with the lines
//! worth a breakpoint, for the user to take all or some of, each as a breakpoint set of its own.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Result, bail};
use tuitor_code::{CallPath, Found, Query, Update};
use tuitor_core::Engine;
use tuitor_core::breakpoints::{SET_COLORS, SetId};

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
    /// The paths `enter` makes sets of; as many of the first as there are sets for.
    pub marked: Vec<bool>,
    /// The sets the paths marked go to, in order: see [`free_sets`].
    pub sets: Vec<SetId>,
}

impl Paths {
    pub fn new(tab: TabId, place: String, query: Query, sets: Vec<SetId>) -> Paths {
        Paths {
            tab,
            place,
            started: Instant::now(),
            query: Some(query),
            progress: String::new(),
            found: None,
            selected: 0,
            marked: Vec::new(),
            sets,
        }
    }

    /// Takes in what the query said.
    pub fn poll(&mut self) {
        let Some(query) = &mut self.query else { return };
        for update in query.updates() {
            match update {
                Update::Progress(text) => self.progress = text,
                Update::Done(found) => {
                    let paths = found.as_ref().map_or(0, |found| found.paths.len());
                    self.marked = (0..paths).map(|i| i < self.sets.len()).collect();
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

    /// Marks the selected path or unmarks it; refuses to mark more paths than there are sets
    /// for.
    pub fn toggle(&mut self) -> Result<(), String> {
        let marked = self.marked.iter().filter(|m| **m).count();
        let Some(mark) = self.marked.get_mut(self.selected) else { return Ok(()) };
        if !*mark && marked >= self.sets.len() {
            return Err(match self.sets.len() {
                0 => format!("there are {SET_COLORS} sets, one per colour, all in use: D in BREAKPOINTS deletes one"),
                free => format!(
                    "{free} of the {SET_COLORS} set colours {} free and taken by the paths marked: unmark one first",
                    if free == 1 { "is" } else { "are" }
                ),
            });
        }
        *mark = !*mark;
        Ok(())
    }

    /// The paths marked, each with the set it goes to.
    pub fn chosen(&self) -> Vec<(&CallPath, SetId)> {
        let marked = self.paths().iter().zip(&self.marked).filter(|(_, marked)| **marked);
        marked.map(|(path, _)| path).zip(self.sets.iter().copied()).collect()
    }

    /// The set the path at `index` goes to, if it is marked.
    pub fn set_of(&self, index: usize) -> Option<SetId> {
        if !self.marked.get(index).copied().unwrap_or(false) {
            return None;
        }
        let before = self.marked[..index].iter().filter(|m| **m).count();
        self.sets.get(before).copied()
    }
}

/// The sets paths can go to, in the order they are taken: sets with no breakpoint, then the
/// colours no set has yet, in the order `new_set` gives them.
pub fn free_sets(engine: &Engine) -> Vec<SetId> {
    let bps = engine.breakpoints();
    let empty = bps.sets().iter().copied().filter(|set| bps.members(*set).next().is_none());
    empty.chain((0..SET_COLORS).filter(|set| !bps.sets().contains(set))).collect()
}

/// What taking paths did.
#[derive(Debug, Default, PartialEq)]
pub struct Taken {
    /// The set of each path, in order.
    pub sets: Vec<SetId>,
    pub added: usize,
    /// Lines that had a breakpoint already, which joined the path's set.
    pub existing: usize,
}

/// Makes each of `paths` a set of breakpoints on its lines, in the set given with it, and the
/// first one the active set: the paths come nearest an entry point first, and the first is the
/// one to stop on. A line several paths share is one breakpoint, in each of their sets.
pub fn take(engine: &mut Engine, paths: &[(&CallPath, SetId)]) -> Result<Taken> {
    let mut taken = Taken::default();
    let mut lines: Vec<(PathBuf, u32)> = Vec::new();
    for (path, set) in paths {
        let set = if engine.breakpoints().sets().contains(set) { *set } else { engine.new_set()? };
        for stop in &path.stops {
            let (_, new) = engine.put_in_set(&stop.path, stop.line, set)?;
            let line = (stop.path.clone(), stop.line);
            if !lines.contains(&line) {
                taken.added += usize::from(new);
                taken.existing += usize::from(!new);
                lines.push(line);
            }
        }
        taken.sets.push(set);
    }
    match taken.sets.first() {
        Some(first) => engine.activate_set(*first)?,
        None => bail!("no path marked"),
    }
    Ok(taken)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tuitor_code::{Found, Root, Step, Stop, Why};
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
    fn each_path_is_a_set_and_a_line_they_share_one_breakpoint_in_both() {
        let options = EngineOptions { adapters: Adapters::default(), workspace: "/w".into(), state_file: None };
        let (mut engine, _) = Engine::new(options);
        // Rose, the set there always is, has a breakpoint: the paths go to new colours.
        engine.toggle_breakpoint(Path::new("/w/b.php"), 9);
        assert_eq!(free_sets(&engine), [1, 2, 3, 4]);
        let first = path(&[("a.php", 3), ("b.php", 9)]);
        let second = path(&[("a.php", 3), ("c.php", 5)]);
        let taken = take(&mut engine, &[(&first, 1), (&second, 2)]).expect("taken");
        assert_eq!(taken, Taken { sets: vec![1, 2], added: 2, existing: 1 });
        let bps = engine.breakpoints();
        let sets: Vec<(String, u32, Vec<SetId>)> =
            bps.iter().map(|bp| (bp.path.display().to_string(), bp.line, bp.sets.clone())).collect();
        assert_eq!(
            sets,
            [("/w/a.php".into(), 3, vec![1, 2]), ("/w/b.php".into(), 9, vec![0, 1]), ("/w/c.php".into(), 5, vec![2])]
        );
        // The first path stops; the other records.
        assert_eq!(bps.active_set(), 1);
        assert_eq!(free_sets(&engine), [3, 4]);
    }

    /// PATHS with `n` paths found and `sets` to put them in, as `poll` leaves it.
    fn found(n: usize, sets: Vec<SetId>) -> Paths {
        let target = Step { name: "f".into(), path: "/w/a.php".into(), line: 1, observed: false };
        let paths = (0..n).map(|_| path(&[("a.php", 3)])).collect();
        let found = Found { target, paths, notes: Vec::new(), requests: 0 };
        let marked = (0..n).map(|i| i < sets.len()).collect();
        let (started, progress) = (Instant::now(), String::new());
        Paths {
            tab: 1,
            place: "a.php:3".into(),
            started,
            query: None,
            progress,
            found: Some(Ok(found)),
            selected: 0,
            marked,
            sets,
        }
    }

    #[test]
    fn no_more_paths_are_marked_than_there_are_sets_for() {
        let mut paths = found(4, vec![3, 4]);
        assert_eq!(paths.marked, [true, true, false, false]);
        assert_eq!((paths.set_of(1), paths.set_of(2)), (Some(4), None));
        paths.selected = 2;
        let refused = paths.toggle().expect_err("full");
        assert_eq!(refused, "2 of the 5 set colours are free and taken by the paths marked: unmark one first");
        paths.selected = 0;
        paths.toggle().expect("unmarked");
        paths.selected = 2;
        paths.toggle().expect("marked");
        // The paths marked take the sets in order.
        let chosen: Vec<SetId> = paths.chosen().into_iter().map(|(_, set)| set).collect();
        assert_eq!((chosen, paths.set_of(2)), (vec![3, 4], Some(4)));
        let mut full = found(2, Vec::new());
        assert!(full.toggle().is_err_and(|why| why.contains("all in use: D in BREAKPOINTS deletes one")));
    }

    #[test]
    fn an_empty_set_is_taken_before_a_new_colour() {
        let options = EngineOptions { adapters: Adapters::default(), workspace: "/w".into(), state_file: None };
        let (mut engine, _) = Engine::new(options);
        assert_eq!(free_sets(&engine), [0, 1, 2, 3, 4]);
        let only = path(&[("a.php", 3)]);
        assert_eq!(take(&mut engine, &[(&only, 0)]).expect("taken").sets, [0]);
        assert_eq!(engine.breakpoints().sets(), [0]);
    }
}
