//! PATHS: the call trees a language server finds through the SOURCE line, from where execution
//! starts down past the line's function into what each step calls, for the user to choose
//! stages of: the end-to-end path through a stage, single stages and whole branches. Each
//! selection marked becomes a breakpoint set of its own, on the lines worth a breakpoint in
//! its stages.

use std::cell::Cell;
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Result, bail};
use tuitor_code::{Found, Query, Stop, Update};
use tuitor_core::Engine;
use tuitor_core::breakpoints::SetId;
use tuitor_core::palette;

use crate::tab::TabId;

/// Stages chosen together, to become one breakpoint set.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    /// The set it is to be, which is its colour, given when it is marked.
    pub set: SetId,
    pub stages: BTreeSet<usize>,
}

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
    /// The stage under the cursor.
    pub cursor: usize,
    /// Stages whose calls are hidden.
    pub folded: HashSet<usize>,
    /// What is marked, in the order it was; nothing at first.
    pub marks: Vec<Mark>,
    /// The mark `space` and `b` change.
    pub current: Option<usize>,
    /// The tab's sets, whose colours a new mark's keeps apart from.
    pub sets: Vec<SetId>,
    /// The tab's sets with no breakpoint, which marks take before new colours.
    pub free: Vec<SetId>,
    /// The first row of the trees on screen, kept from frame to frame.
    pub offset: Cell<usize>,
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
            cursor: 0,
            folded: HashSet::new(),
            marks: Vec::new(),
            current: None,
            sets: engine.breakpoints().sets().to_vec(),
            free: free_sets(engine),
            offset: Cell::new(0),
        }
    }

    /// Takes in what the query said.
    pub fn poll(&mut self) {
        let Some(query) = &mut self.query else { return };
        for update in query.updates() {
            match update {
                Update::Progress(text) => self.progress = text,
                Update::Done(found) => {
                    self.query = None;
                    self.show(found);
                }
            }
        }
    }

    /// Shows what was found: the branches off the paths folded, the cursor on the first
    /// stage of the function asked about.
    pub fn show(&mut self, found: Result<Found, String>) {
        if let Ok(found) = &found {
            self.folded = (0..found.stages.len())
                .filter(|s| !found.on_path(*s) && !found.stages[*s].children.is_empty())
                .collect();
            self.cursor = found.stages.iter().position(|s| s.anchor).unwrap_or(0);
        }
        self.found = Some(found);
    }

    pub fn tree(&self) -> Option<&Found> {
        match &self.found {
            Some(Ok(found)) => Some(found),
            _ => None,
        }
    }

    /// The stages on screen, depth first: those below a folded stage are hidden.
    pub fn rows(&self) -> Vec<usize> {
        let Some(found) = self.tree() else { return Vec::new() };
        let mut rows = Vec::new();
        let mut stage = 0;
        while stage < found.stages.len() {
            rows.push(stage);
            stage = if self.folded.contains(&stage) { found.stages[stage].end } else { stage + 1 };
        }
        rows
    }

    /// Moves the cursor `delta` rows.
    pub fn select(&mut self, delta: isize) {
        let rows = self.rows();
        let at = rows.iter().position(|s| *s == self.cursor).unwrap_or(0);
        if let Some(&stage) = rows.get(at.saturating_add_signed(delta).min(rows.len().saturating_sub(1))) {
            self.cursor = stage;
        }
    }

    /// `right`: shows the calls of the stage under the cursor, or goes to the first of them.
    /// `left`: hides them, or goes to the stage that makes the call.
    pub fn fold(&mut self, open: bool) {
        let Some(found) = self.tree() else { return };
        let Some(stage) = found.stages.get(self.cursor) else { return };
        let (children, parent) = (stage.children.first().copied(), stage.parent);
        let folded = self.folded.contains(&self.cursor);
        match (open, children, folded) {
            (true, Some(_), true) => drop(self.folded.remove(&self.cursor)),
            (true, Some(first), false) => self.cursor = first,
            (false, Some(_), false) => drop(self.folded.insert(self.cursor)),
            (false, _, _) => self.cursor = parent.unwrap_or(self.cursor),
            (true, None, _) => {}
        }
    }

    /// The set of the next mark: an empty set no mark has, else a colour apart from those of
    /// the tab's sets and of the marks ([`palette::next`]).
    fn next_set(&self) -> SetId {
        let given: Vec<SetId> = self.marks.iter().map(|mark| mark.set).collect();
        self.free.iter().copied().find(|set| !given.contains(set)).unwrap_or_else(|| {
            let taken: Vec<SetId> = self.sets.iter().chain(&given).copied().collect();
            palette::next(&taken)
        })
    }

    /// What `set` is called among the tab's sets and the marks.
    pub fn name(&self, set: SetId) -> String {
        let mut all = self.sets.clone();
        all.extend(self.marks.iter().map(|mark| mark.set).filter(|set| !self.sets.contains(set)));
        palette::name(set, &all)
    }

    /// Marks `stages` as a new selection, the one to change from now on.
    fn add_mark(&mut self, stages: BTreeSet<usize>) -> SetId {
        let set = self.next_set();
        self.marks.push(Mark { set, stages });
        self.current = Some(self.marks.len() - 1);
        set
    }

    /// `paths mark`: the end-to-end path through the stage under the cursor, a new mark.
    pub fn mark(&mut self) -> Result<(SetId, usize, bool), String> {
        let found = self.tree().ok_or("no paths to mark")?;
        let path: BTreeSet<usize> = found.path_through(self.cursor).into_iter().collect();
        let through = path.iter().any(|s| found.stages[*s].anchor);
        if let Some(known) = self.marks.iter().position(|mark| mark.stages == path) {
            self.current = Some(known);
            return Err("that path is marked already: [ and ] go from mark to mark".into());
        }
        let stages = path.len();
        Ok((self.add_mark(path), stages, through))
    }

    /// `paths stage` (`all` false) puts the stage under the cursor in the current mark or
    /// takes it out; `paths branch` (`all` true) the stage and every stage below it. With no
    /// mark, they start one. A mark left with no stage is unmarked.
    pub fn choose(&mut self, all: bool) -> Result<String, String> {
        let found = self.tree().ok_or("no paths to choose from")?;
        let stages: Vec<usize> = if all { found.branch(self.cursor).collect() } else { vec![self.cursor] };
        let Some(current) = self.current else {
            let set = self.add_mark(stages.iter().copied().collect());
            return Ok(format!("{} marked, with {}", self.name(set), count(stages.len(), "stage")));
        };
        let name = self.name(self.marks[current].set);
        let mark = &mut self.marks[current];
        let taking = stages.iter().all(|s| mark.stages.contains(s));
        for stage in &stages {
            if taking {
                mark.stages.remove(stage);
            } else {
                mark.stages.insert(*stage);
            }
        }
        let left = mark.stages.len();
        if left == 0 {
            self.unmark();
            return Ok(format!("{name} unmarked: no stage left in it"));
        }
        let what = if taking { "out of" } else { "in" };
        Ok(format!("{} {what} {name}, {} in it", count(stages.len(), "stage"), count(left, "stage")))
    }

    /// `paths unmark`: drops the current mark, which frees its colour; the one before it is
    /// current then. Says which it was.
    pub fn unmark(&mut self) -> Option<String> {
        let current = self.current?;
        let name = self.name(self.marks[current].set);
        self.marks.remove(current);
        self.current = if self.marks.is_empty() { None } else { Some(current.saturating_sub(1)) };
        Some(name)
    }

    /// `paths next`, `paths prev`: the mark after or before the current one is current. Says
    /// which it is.
    pub fn cycle(&mut self, delta: isize) -> Option<String> {
        let len = self.marks.len();
        if len == 0 {
            return None;
        }
        let at = self.current.map_or(0, |c| (c as isize + delta).rem_euclid(len as isize) as usize);
        self.current = Some(at);
        Some(self.name(self.marks[at].set))
    }

    /// The marks `stage` is in.
    pub fn marks_of(&self, stage: usize) -> Vec<&Mark> {
        self.marks.iter().filter(|mark| mark.stages.contains(&stage)).collect()
    }

    /// The lines of each mark, with its set.
    pub fn chosen(&self) -> Vec<(Vec<Stop>, SetId)> {
        let Some(found) = self.tree() else { return Vec::new() };
        self.marks.iter().map(|mark| (found.selection(&mark.stages), mark.set)).collect()
    }

    /// The lines of the stage under the cursor: as in the current mark when it is there, else
    /// as on the end-to-end path through it. Says which.
    pub fn lines_here(&self) -> Option<(Vec<Stop>, Option<SetId>)> {
        let found = self.tree()?;
        let mark = self.current.map(|c| &self.marks[c]).filter(|mark| mark.stages.contains(&self.cursor));
        Some(match mark {
            Some(mark) => (found.lines(self.cursor, |child| mark.stages.contains(&child)), Some(mark.set)),
            None => {
                let path: BTreeSet<usize> = found.path_through(self.cursor).into_iter().collect();
                (found.lines(self.cursor, |child| path.contains(&child)), None)
            }
        })
    }
}

fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// The sets with no breakpoint, which marks are put in before new sets are made.
pub fn free_sets(engine: &Engine) -> Vec<SetId> {
    let bps = engine.breakpoints();
    bps.sets().iter().copied().filter(|set| bps.members(*set).next().is_none()).collect()
}

/// What taking marks did.
#[derive(Debug, Default, PartialEq)]
pub struct Taken {
    /// The set of each mark, in order.
    pub sets: Vec<SetId>,
    pub added: usize,
    /// Lines that had a breakpoint already, which joined the mark's set.
    pub existing: usize,
}

/// Makes each mark a set of breakpoints on its lines, in the set given with it, made in its
/// colour where there is none, and the first one the active set. A line several marks share is
/// one breakpoint, in each of their sets.
pub fn take(engine: &mut Engine, marks: &[(Vec<Stop>, SetId)]) -> Result<Taken> {
    let mut taken = Taken::default();
    let mut lines: Vec<(PathBuf, u32)> = Vec::new();
    for (stops, set) in marks {
        let set = *set;
        if !engine.breakpoints().sets().contains(&set) {
            engine.new_set_in(set)?;
        }
        for stop in stops {
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
        None => bail!("nothing marked"),
    }
    Ok(taken)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tuitor_code::{Entry, Facts, Function, Root, Stage, Why};
    use tuitor_core::EngineOptions;
    use tuitor_core::config::Adapters;

    use super::*;

    fn engine() -> Engine {
        let options = EngineOptions { adapters: Adapters::default(), workspace: "/w".into(), state_file: None };
        Engine::new(options).0
    }

    fn stops(lines: &[(&str, u32)]) -> Vec<Stop> {
        lines
            .iter()
            .map(|(file, line)| Stop { path: Path::new("/w").join(file), line: *line, why: Why::Entry, stage: 0 })
            .collect()
    }

    #[test]
    fn each_mark_is_a_set_and_a_line_they_share_one_breakpoint_in_both() {
        let mut engine = engine();
        // The set there always is has a breakpoint: the marks go to new colours.
        let first_set = engine.breakpoints().active_set();
        engine.toggle_breakpoint(Path::new("/w/b.php"), 9);
        assert!(free_sets(&engine).is_empty());
        let one = palette::next(&[first_set]);
        let two = palette::next(&[first_set, one]);
        let first = stops(&[("a.php", 3), ("b.php", 9)]);
        let second = stops(&[("a.php", 3), ("c.php", 5)]);
        let taken = take(&mut engine, &[(first, one), (second, two)]).expect("taken");
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
        // The first mark stops; the other records.
        assert_eq!((bps.sets(), bps.active_set()), (&[first_set, one, two][..], one));
    }

    /// A function of the project in /w, whose body starts at `entry` (1-based).
    fn function(name: &str, file: &str, entry: u32) -> Function {
        let short = name.rsplit("::").next().unwrap_or(name).to_owned();
        let facts = Facts { entry: Some(entry - 1), writes: Vec::new(), uses: Vec::new() };
        Function { name: name.into(), short, path: Path::new("/w").join(file), line: entry - 1, facts }
    }

    /// The tree of OrderController::show, from `(depth, function, line of the call)` rows,
    /// depth first, through Money::lineTotal.
    fn found() -> Found {
        let functions = vec![
            function("OrderController::show", "OrderController.php", 21),
            function("OrderService::total", "OrderService.php", 21),
            function("OrderRepository::find", "OrderRepository.php", 15),
            function("OrderRepository::load", "OrderRepository.php", 20),
            function("Money::lineTotal", "Money.php", 14),
            function("Dispatcher::dispatch", "Dispatcher.php", 20),
            function("Money::format", "Money.php", 23),
        ];
        let rows = [(0, 0, 0), (1, 1, 22), (2, 2, 21), (3, 3, 15), (2, 4, 23), (2, 5, 29), (1, 6, 23)];
        let mut stages: Vec<Stage> = Vec::new();
        let mut open: Vec<usize> = Vec::new();
        for (i, &(depth, function, site)) in rows.iter().enumerate() {
            open.truncate(depth);
            let parent = open.last().copied();
            stages.push(Stage {
                function,
                parent,
                children: Vec::new(),
                end: i + 1,
                depth,
                sites: parent.map(|_| vec![site]).unwrap_or_default(),
                observed: false,
                anchor: function == 4,
                leads: false,
                cut: None,
                root: parent.is_none().then_some(Root::Entry(Entry::Controller)),
            });
            if let Some(parent) = parent {
                stages[parent].children.push(i);
            }
            open.push(i);
        }
        for i in (0..stages.len()).rev() {
            stages[i].end = stages[i].children.last().map_or(i + 1, |c| stages[*c].end);
            stages[i].leads = stages[i].anchor || stages[i].children.iter().any(|c| stages[*c].leads);
        }
        Found {
            functions,
            target: 4,
            line: 16,
            stages,
            notes: Vec::new(),
            unseen: String::new(),
            requests: 0,
            state: 3,
        }
    }

    /// PATHS as `poll` leaves it with the tree found, for the sets of `engine`.
    fn shown(engine: &Engine) -> Paths {
        let mut paths = Paths {
            tab: 1,
            place: "Money.php:16".into(),
            started: Instant::now(),
            query: None,
            progress: String::new(),
            found: None,
            cursor: 0,
            folded: HashSet::new(),
            marks: Vec::new(),
            current: None,
            sets: engine.breakpoints().sets().to_vec(),
            free: free_sets(engine),
            offset: Cell::new(0),
        };
        paths.show(Ok(found()));
        paths
    }

    fn lines(stops: &[Stop]) -> Vec<String> {
        let file = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        stops.iter().map(|stop| format!("{}:{} {:?}", file(&stop.path), stop.line, stop.why)).collect()
    }

    #[test]
    fn branches_off_the_paths_start_folded_and_the_cursor_on_the_anchor() {
        let mut paths = shown(&engine());
        // OrderRepository::find leads nowhere near the helper: its calls are hidden.
        assert_eq!((paths.rows(), paths.cursor), (vec![0, 1, 2, 4, 5, 6], 4));
        assert!(paths.marks.is_empty());
        paths.select(-1);
        assert_eq!(paths.cursor, 2);
        paths.fold(true);
        assert_eq!(paths.rows(), [0, 1, 2, 3, 4, 5, 6]);
        paths.fold(true);
        assert_eq!(paths.cursor, 3);
        paths.fold(false);
        assert_eq!(paths.cursor, 2);
        paths.fold(false);
        assert_eq!(paths.rows(), [0, 1, 2, 4, 5, 6]);
        paths.select(100);
        assert_eq!(paths.cursor, 6);
    }

    #[test]
    fn a_path_stages_and_branches_are_marked_and_each_mark_gets_its_set() {
        let engine = engine();
        // The tab's one set has no breakpoint: the first mark takes it.
        let empty = engine.breakpoints().active_set();
        let mut paths = shown(&engine);
        // m on the anchor: the path from the controller down to it.
        assert_eq!(paths.mark(), Ok((empty, 3, true)));
        assert_eq!(paths.marks[0].stages, BTreeSet::from([0, 1, 4]));
        let (stops, set) = &paths.chosen()[0];
        assert_eq!(
            (lines(stops), *set),
            (
                vec![
                    "OrderController.php:21 Entry".to_owned(),
                    "OrderController.php:22 Calls(\"total\")".into(),
                    "OrderService.php:21 Entry".into(),
                    "OrderService.php:23 Calls(\"lineTotal\")".into(),
                    "Money.php:14 Entry".into(),
                    "Money.php:16 Target".into(),
                ],
                empty
            )
        );
        assert!(paths.mark().is_err_and(|why| why.contains("marked already")));
        // The controller goes on after the service: its call into format joins the mark.
        paths.cursor = 6;
        paths.choose(false).expect("stage in");
        assert!(lines(&paths.chosen()[0].0).contains(&"OrderController.php:23 Calls(\"format\")".to_owned()));
        // A branch in, and out again.
        let first = paths.name(empty);
        paths.cursor = 2;
        assert_eq!(paths.choose(true), Ok(format!("2 stages in {first}, 6 stages in it")));
        assert_eq!(paths.choose(true), Ok(format!("2 stages out of {first}, 4 stages in it")));
        // A second mark, through the dispatcher, in a colour apart from the first: it does not
        // pass the anchor.
        paths.cursor = 5;
        let second = palette::next(&[empty]);
        assert_eq!(paths.mark(), Ok((second, 3, false)));
        assert_eq!((paths.current, paths.marks_of(1).len()), (Some(1), 2));
        // As many marks as wanted, each in a colour of its own.
        for stage in [2, 6, 4] {
            paths.cursor = stage;
            paths.mark().expect("marked");
        }
        let mut colours: Vec<SetId> = paths.marks.iter().map(|mark| mark.set).collect();
        colours.sort();
        colours.dedup();
        assert_eq!(colours.len(), 5);
        assert_eq!(paths.cycle(1).as_deref(), Some(first.as_str()));
        assert_eq!(paths.unmark().as_deref(), Some(first.as_str()));
        assert_eq!((paths.marks.len(), paths.current), (4, Some(0)));
        while paths.unmark().is_some() {}
        // With no mark, a stage starts one, in the colour freed first; taken out again, it
        // leaves the mark empty, unmarked.
        paths.cursor = 5;
        assert_eq!(paths.choose(false), Ok(format!("{first} marked, with 1 stage")));
        assert_eq!((paths.marks[0].stages.clone(), paths.marks[0].set), (BTreeSet::from([5]), empty));
        assert_eq!(paths.choose(false), Ok(format!("{first} unmarked: no stage left in it")));
        assert!(paths.marks.is_empty());
    }

    #[test]
    fn marks_become_sets_that_share_the_lines_of_the_stages_they_share() {
        let mut engine = engine();
        let mut paths = shown(&engine);
        let (one, ..) = paths.mark().expect("the helper's path");
        paths.cursor = 5;
        let (two, ..) = paths.mark().expect("the dispatcher's");
        let taken = take(&mut engine, &paths.chosen()).expect("taken");
        assert_eq!((taken.sets, taken.added, taken.existing), (vec![one, two], 8, 0));
        let bps = engine.breakpoints();
        assert_eq!((bps.sets(), bps.active_set()), (&[one, two][..], one));
        let at = |file: &str, line: u32| bps.at(&Path::new("/w").join(file), line).map(|bp| bp.sets.clone());
        // The controller and the service are on both paths; the helper and the dispatcher on one.
        assert_eq!(at("OrderController.php", 22), Some(vec![one, two]));
        assert_eq!(at("OrderService.php", 21), Some(vec![one, two]));
        assert_eq!(at("OrderService.php", 23), Some(vec![one]));
        assert_eq!(at("OrderService.php", 29), Some(vec![two]));
        assert_eq!(at("Money.php", 16), Some(vec![one]));
        assert_eq!(at("Dispatcher.php", 20), Some(vec![two]));
    }

    #[test]
    fn an_empty_set_is_taken_before_a_new_colour() {
        let mut engine = engine();
        let empty = engine.breakpoints().active_set();
        assert_eq!(free_sets(&engine), [empty]);
        assert_eq!(take(&mut engine, &[(stops(&[("a.php", 3)]), empty)]).expect("taken").sets, [empty]);
        assert_eq!(engine.breakpoints().sets(), [empty]);
    }
}
