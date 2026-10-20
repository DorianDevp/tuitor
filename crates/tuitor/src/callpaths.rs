//! PATHS: the call paths a language server finds to the SOURCE line, each with the lines
//! worth a breakpoint, for the user to take all or some of, each as a breakpoint set of its own.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Result, bail};
use tuitor_code::{CallPath, Found, Query, Update};
use tuitor_core::Engine;
use tuitor_core::breakpoints::SetId;
use tuitor_core::palette;

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
    /// For each path, the set `enter` makes of it, given when it is marked: none is at first.
    pub marked: Vec<Option<SetId>>,
    /// The tab's sets: those with no breakpoint are taken first, then new colours.
    pub sets: Vec<SetId>,
    /// The tab's sets with no breakpoint, see [`free_sets`].
    pub free: Vec<SetId>,
}

impl Paths {
    pub fn new(tab: TabId, place: String, query: Query, engine: &Engine) -> Paths {
        Paths {
            tab,
            place,
            started: Instant::now(),
            query: Some(query),
            progress: String::new(),
            found: None,
            selected: 0,
            marked: Vec::new(),
            sets: engine.breakpoints().sets().to_vec(),
            free: free_sets(engine),
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
                    self.marked = vec![None; paths];
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

    /// Marks the selected path, in the colour of the set it is to be, or unmarks it, which
    /// frees the colour: an empty set's, else one apart from those of the sets there are and
    /// of the paths marked ([`palette::next`]).
    pub fn toggle(&mut self) {
        let given: Vec<SetId> = self.marked.iter().flatten().copied().collect();
        let Some(mark) = self.marked.get_mut(self.selected) else { return };
        *mark = match mark {
            Some(_) => None,
            None => Some(self.free.iter().copied().find(|set| !given.contains(set)).unwrap_or_else(|| {
                let taken: Vec<SetId> = self.sets.iter().chain(&given).copied().collect();
                palette::next(&taken)
            })),
        };
    }

    /// The paths marked, each with the set it goes to.
    pub fn chosen(&self) -> Vec<(&CallPath, SetId)> {
        self.paths().iter().zip(&self.marked).filter_map(|(path, set)| Some((path, (*set)?))).collect()
    }

    /// The set the path at `index` goes to, if it is marked.
    pub fn set_of(&self, index: usize) -> Option<SetId> {
        self.marked.get(index).copied().flatten()
    }
}

/// The sets with no breakpoint, which paths are put in before new sets are made.
pub fn free_sets(engine: &Engine) -> Vec<SetId> {
    let bps = engine.breakpoints();
    bps.sets().iter().copied().filter(|set| bps.members(*set).next().is_none()).collect()
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
        if !engine.breakpoints().sets().contains(set) {
            engine.new_set_in(*set)?;
        }
        for stop in &path.stops {
            let (_, new) = engine.put_in_set(&stop.path, stop.line, *set)?;
            let line = (stop.path.clone(), stop.line);
            if !lines.contains(&line) {
                taken.added += usize::from(new);
                taken.existing += usize::from(!new);
                lines.push(line);
            }
        }
        taken.sets.push(*set);
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

    fn engine() -> Engine {
        let options = EngineOptions { adapters: Adapters::default(), workspace: "/w".into(), state_file: None };
        Engine::new(options).0
    }

    #[test]
    fn each_path_is_a_set_and_a_line_they_share_one_breakpoint_in_both() {
        let mut engine = engine();
        // The set there always is has a breakpoint: the paths go to new colours.
        let first_set = engine.breakpoints().active_set();
        engine.toggle_breakpoint(Path::new("/w/b.php"), 9);
        assert!(free_sets(&engine).is_empty());
        let (one, two) = (palette::next(&[first_set]), palette::next(&[first_set, palette::next(&[first_set])]));
        let first = path(&[("a.php", 3), ("b.php", 9)]);
        let second = path(&[("a.php", 3), ("c.php", 5)]);
        let taken = take(&mut engine, &[(&first, one), (&second, two)]).expect("taken");
        assert_eq!(taken, Taken { sets: vec![one, two], added: 2, existing: 1 });
        let bps = engine.breakpoints();
        let sets: Vec<(String, u32, Vec<SetId>)> =
            bps.iter().map(|bp| (bp.path.display().to_string(), bp.line, bp.sets.clone())).collect();
        assert_eq!(
            sets,
            [
                ("/w/a.php".into(), 3, vec![one, two]),
                ("/w/b.php".into(), 9, vec![first_set, one]),
                ("/w/c.php".into(), 5, vec![two])
            ]
        );
        // The first path stops; the other records.
        assert_eq!((bps.sets(), bps.active_set()), (&[first_set, one, two][..], one));
    }

    /// PATHS with `n` paths found for `engine`, as `poll` leaves it.
    fn found(n: usize, engine: &Engine) -> Paths {
        let target = Step { name: "f".into(), path: "/w/a.php".into(), line: 1, observed: false };
        let paths = (0..n).map(|_| path(&[("a.php", 3)])).collect();
        let found = Found { target, paths, notes: Vec::new(), requests: 0 };
        let (started, progress) = (Instant::now(), String::new());
        Paths {
            tab: 1,
            place: "a.php:3".into(),
            started,
            query: None,
            progress,
            found: Some(Ok(found)),
            selected: 0,
            marked: vec![None; n],
            sets: engine.breakpoints().sets().to_vec(),
            free: free_sets(engine),
        }
    }

    #[test]
    fn nothing_is_marked_at_first_and_a_path_marked_has_its_colour_until_unmarked() {
        let engine = engine();
        let empty = engine.breakpoints().active_set();
        let mut paths = found(12, &engine);
        assert!(paths.chosen().is_empty() && paths.set_of(0).is_none());
        // The first path marked takes the empty set; the others colours apart from it, as
        // many as there are paths.
        for i in [3, 0, 5, 1, 2, 4, 6, 7, 8, 9, 10, 11] {
            paths.selected = i;
            paths.toggle();
        }
        assert_eq!(paths.set_of(3), Some(empty));
        assert_eq!(paths.set_of(0), Some(palette::next(&[empty])));
        let mut colours: Vec<SetId> = paths.marked.iter().flatten().copied().collect();
        colours.sort();
        colours.dedup();
        assert_eq!(colours.len(), 12);
        // Unmarking frees the colour; the others keep theirs, and the path marked again gets
        // one no other has.
        let kept = paths.set_of(1);
        paths.selected = 5;
        paths.toggle();
        assert_eq!((paths.set_of(5), paths.set_of(1), paths.chosen().len()), (None, kept, 11));
        paths.toggle();
        let again = paths.set_of(5).expect("marked");
        assert_eq!(paths.marked.iter().flatten().filter(|set| **set == again).count(), 1);
        // In the order of the paths.
        let chosen: Vec<SetId> = paths.chosen().into_iter().map(|(_, set)| set).collect();
        assert_eq!(chosen[..2], [palette::next(&[empty]), kept.expect("marked")]);

        // A colour freed is the one the next path marked gets, when the others are as they were.
        let mut paths = found(3, &engine);
        for i in [0, 1] {
            paths.selected = i;
            paths.toggle();
        }
        let freed = paths.set_of(1);
        paths.toggle();
        paths.selected = 2;
        paths.toggle();
        assert_eq!((paths.set_of(1), paths.set_of(2)), (None, freed));
    }

    #[test]
    fn an_empty_set_is_taken_before_a_new_colour() {
        let mut engine = engine();
        let empty = engine.breakpoints().active_set();
        assert_eq!(free_sets(&engine), [empty]);
        let only = path(&[("a.php", 3)]);
        assert_eq!(take(&mut engine, &[(&only, empty)]).expect("taken").sets, [empty]);
        assert_eq!(engine.breakpoints().sets(), [empty]);
    }
}
