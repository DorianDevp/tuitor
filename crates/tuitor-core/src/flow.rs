//! The flow of a run: the files that took part, in the order execution first reached
//! them, and those still to come. Every row says where it comes from:
//!
//! - the stops of this run: every file on every recorded stack ([`Observed`]);
//! - an earlier run from the same entry point, recorded the same way ([`Run`]);
//! - an Xdebug trace of a request that took the same path ([`TraceSummary`]).
//!
//! Nothing is guessed beyond what those say: a file is "predicted" only when an earlier
//! run or a trace reached it after the point this run is at.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::snapshot::{Frame, Snapshot, is_library_path};
use crate::trace::{Position, Trace};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Location {
    pub path: PathBuf,
    pub line: u32,
}

impl Location {
    fn of(frame: &Frame) -> Option<Location> {
        Some(Location { path: frame.path.clone()?, line: frame.line })
    }
}

/// One function calling another, seen on a stack or in a trace.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Edge {
    pub caller: String,
    pub callee: String,
    /// The line the call is made from, in the caller's file.
    pub site: Location,
    pub callee_path: PathBuf,
}

/// The top of the stack at one stop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stop {
    pub top: Option<Location>,
    /// Stopped by a breakpoint.
    pub hit: bool,
}

/// What the stops of one run showed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Observed {
    /// One per stop, the first stop first.
    pub stops: Vec<Stop>,
    /// Every file on a stack, in the order first seen.
    pub files: Vec<ObservedFile>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservedFile {
    pub path: PathBuf,
    pub library: bool,
    /// Function names with the number of stops each was on the stack at.
    pub functions: Vec<(String, u32)>,
    /// The stop each function was first on the stack at.
    #[serde(default)]
    pub first_seen: BTreeMap<String, u32>,
    /// Lines a frame in this file was at.
    pub lines: BTreeSet<u32>,
    /// Stop numbers (1-based) at which the file was on the stack.
    pub stops: Vec<u32>,
}

impl Observed {
    /// Reads the stacks of the stops of one run, oldest first, numbered from 1 in that
    /// order. Outer frames come first, so that a file is listed before the files it called
    /// into. Frames without a file are left out.
    pub fn from_stops(stops: &[Snapshot]) -> Observed {
        let mut observed = Observed::default();
        let mut edges = HashSet::new();
        for (number, snapshot) in (1..).zip(stops) {
            let hit = !snapshot.hit_breakpoints.is_empty();
            observed.stops.push(Stop { top: snapshot.top().and_then(Location::of), hit });
            let mut functions_here: HashSet<(&Path, &str)> = HashSet::new();
            for frame in snapshot.frames.iter().rev() {
                let Some(path) = frame.path.as_deref() else { continue };
                let index = observed.files.iter().position(|f| f.path == path).unwrap_or_else(|| {
                    observed.files.push(ObservedFile {
                        path: path.to_path_buf(),
                        library: frame.library,
                        functions: Vec::new(),
                        first_seen: BTreeMap::new(),
                        lines: BTreeSet::new(),
                        stops: Vec::new(),
                    });
                    observed.files.len() - 1
                });
                let file = &mut observed.files[index];
                file.lines.insert(frame.line);
                if file.stops.last() != Some(&number) {
                    file.stops.push(number);
                }
                // A function counts once per stop, however deep it recurses.
                if functions_here.insert((path, frame.name.as_str())) {
                    match file.functions.iter_mut().find(|(name, _)| *name == frame.name) {
                        Some((_, count)) => *count += 1,
                        None => {
                            file.functions.push((frame.name.clone(), 1));
                            file.first_seen.insert(frame.name.clone(), number);
                        }
                    }
                }
            }
            for pair in snapshot.frames.windows(2) {
                let [callee, caller] = pair else { continue };
                let (Some(site), Some(callee_path)) = (Location::of(caller), callee.path.clone()) else { continue };
                let edge = Edge { caller: caller.name.clone(), callee: callee.name.clone(), site, callee_path };
                if edges.insert(edge.clone()) {
                    observed.edges.push(edge);
                }
            }
        }
        observed
    }

    fn file(&self, path: &Path) -> Option<&ObservedFile> {
        self.files.iter().find(|f| f.path == path)
    }
}

/// A run as it is remembered: what its stops showed, and where it started.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// Counted per project, from 1.
    pub number: u32,
    /// When its first stop happened, `2026-10-20 01:12`.
    pub started: String,
    /// Where its first stop was.
    pub entry: Option<Location>,
    /// Where it first stopped at a breakpoint.
    pub first_hit: Option<Location>,
    pub observed: Observed,
}

impl Run {
    pub fn new(number: u32, started: String, observed: Observed) -> Run {
        let entry = observed.stops.first().and_then(|s| s.top.clone());
        let first_hit = observed.stops.iter().find(|s| s.hit).and_then(|s| s.top.clone());
        Run { number, started, entry, first_hit, observed }
    }

    /// Whether `earlier` started where this run did: at the same first stop, and, once this
    /// run has hit a breakpoint, it stopped at that breakpoint too. A request stopped on
    /// entry starts like every other; its first breakpoint tells which one it is.
    pub fn same_entry(&self, earlier: &Run) -> bool {
        let stopped_at = |hit: &Location| earlier.observed.stops.iter().any(|s| s.top.as_ref() == Some(hit));
        self.entry.is_some() && earlier.entry == self.entry && self.first_hit.as_ref().is_none_or(stopped_at)
    }
}

/// How an earlier run lines up with this one: the stop of the earlier run that was where
/// the viewed stop is, if there was one.
#[derive(Debug, Clone, PartialEq)]
pub struct Alignment {
    pub run: u32,
    pub started: String,
    /// The stop of the earlier run at the viewed stop's location.
    pub matched: Option<(Location, u32)>,
    /// Files it predicts.
    pub predicted: usize,
}

/// What a trace says about each file it reached, after [`Trace::bodies`] placed its calls.
#[derive(Debug, Clone, Default)]
pub struct TraceSummary {
    /// In the order first reached.
    pub files: Vec<TracedFile>,
    pub edges: Vec<Edge>,
    pub end: f64,
}

#[derive(Debug, Clone)]
pub struct TracedFile {
    pub path: PathBuf,
    /// From the first call into it or made from it, to the last such call's end.
    pub first: f64,
    pub last: f64,
    /// Functions whose code is in this file, with how often they were called.
    pub functions: Vec<(String, u32)>,
    /// When each of them was first called.
    pub first_calls: HashMap<String, f64>,
    pub calls: u32,
    /// Lines that made calls, with when.
    pub lines: BTreeMap<u32, Vec<f64>>,
}

impl TraceSummary {
    /// Sums `trace` up by file. `known` places functions the trace cannot (see [`Trace::bodies`]).
    /// Including a file that runs nothing on its own, as an autoloader does with a class,
    /// only declares what is in it: the file takes part once its functions are called.
    pub fn new(trace: &Trace, known: &HashMap<String, PathBuf>) -> TraceSummary {
        let bodies = trace.bodies(known);
        let mut runs_code = vec![false; trace.calls.len()];
        for call in &trace.calls {
            if let Some(parent) = call.parent {
                runs_code[parent] = true;
            }
        }
        let mut files: Vec<TracedFile> = Vec::new();
        let mut index: HashMap<&Path, usize> = HashMap::new();
        let mut edges = HashSet::new();
        let mut summary = TraceSummary { end: trace.end, ..TraceSummary::default() };
        for (i, call) in trace.calls.iter().enumerate() {
            let end = trace.end_of(i);
            // The line it was called from ran, in the caller's file.
            let site = trace.file(call.file);
            if !site.as_os_str().is_empty() && call.line > 0 {
                let at = traced_file(&mut files, &mut index, site, call.start);
                let file = &mut files[at];
                file.lines.entry(call.line).or_default().push(call.start);
                file.last = file.last.max(call.start);
            }
            let Some(body) = bodies[i].filter(|_| call.include.is_none() || runs_code[i]) else { continue };
            let at = traced_file(&mut files, &mut index, body, call.start);
            let file = &mut files[at];
            file.calls += 1;
            file.first = file.first.min(call.start);
            file.last = file.last.max(end);
            match file.functions.iter_mut().find(|(name, _)| *name == call.name) {
                Some((_, count)) => *count += 1,
                None => {
                    file.functions.push((call.name.clone(), 1));
                    file.first_calls.insert(call.name.clone(), call.start);
                }
            }
            if let Some(parent) = call.parent
                && call.include.is_none()
            {
                let edge = Edge {
                    caller: trace.calls[parent].name.clone(),
                    callee: call.name.clone(),
                    site: Location { path: site.to_path_buf(), line: call.line },
                    callee_path: body.to_path_buf(),
                };
                if edges.insert(edge.clone()) {
                    summary.edges.push(edge);
                }
            }
        }
        files.sort_by(|a, b| a.first.total_cmp(&b.first));
        summary.files = files;
        summary
    }

    fn file(&self, path: &Path) -> Option<&TracedFile> {
        self.files.iter().find(|f| f.path == path)
    }
}

/// The index of `path` in `files`, added as first reached at `time` if it is not there.
fn traced_file<'a>(
    files: &mut Vec<TracedFile>,
    index: &mut HashMap<&'a Path, usize>,
    path: &'a Path,
    time: f64,
) -> usize {
    *index.entry(path).or_insert_with(|| {
        files.push(TracedFile {
            path: path.to_path_buf(),
            first: time,
            last: time,
            functions: Vec::new(),
            first_calls: HashMap::new(),
            calls: 0,
            lines: BTreeMap::new(),
        });
        files.len() - 1
    })
}

/// A trace that took the same path as this run, found at the viewed stop.
pub struct Located<'a> {
    pub summary: &'a TraceSummary,
    pub position: &'a Position,
}

/// What the flow is drawn against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Axis {
    /// Seconds since the traced request's first call, up to its last call's end.
    Time(f64),
    /// Stop numbers, up to this one.
    Stops(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// The top of the viewed stop's stack is in this file.
    Executing,
    /// A frame of the viewed stop's stack is in this file.
    Stack,
    /// Reached before, by this run or, per the trace, on the same path.
    Ran,
    /// Reached after this point by an earlier run or the trace, not by this run so far.
    Predicted,
}

/// Where a row comes from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sources {
    pub observed: bool,
    pub trace: bool,
    /// The number of the earlier run.
    pub run: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub path: PathBuf,
    pub library: bool,
    pub status: Status,
    pub sources: Sources,
    /// With calls (trace) or stops (otherwise) per function.
    pub functions: Vec<(String, u32)>,
    /// Where on the axis each function is first reached, by the trace or an earlier run.
    pub first_calls: HashMap<String, f64>,
    /// Calls into the file (trace), or stops it was on the stack at.
    pub count: u32,
    /// Stops of this run (or the earlier run, for its rows) it was on the stack at.
    pub stops: u32,
    /// From first to last activity, on the axis; `None` where the axis cannot place it.
    pub span: Option<(f64, f64)>,
    /// Lines reached, with where on the axis when that is known.
    pub lines: BTreeMap<u32, Vec<f64>>,
    /// The line of the innermost frame in this file at the viewed stop.
    pub stack_line: Option<u32>,
    /// How far past its span an earlier run has it active, as predicted.
    pub ahead: Option<f64>,
}

impl Row {
    /// A row for `path` that knows nothing yet but where it comes from.
    pub fn new(path: PathBuf, library: bool, status: Status, sources: Sources) -> Row {
        Row {
            path,
            library,
            status,
            sources,
            functions: Vec::new(),
            first_calls: HashMap::new(),
            count: 0,
            stops: 0,
            span: None,
            lines: BTreeMap::new(),
            stack_line: None,
            ahead: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Flow {
    pub axis: Axis,
    pub now: f64,
    pub rows: Vec<Row>,
    pub edges: Vec<Edge>,
    pub earlier: Option<Alignment>,
}

impl Flow {
    /// Puts the sources together for the stop numbered `viewed` in `observed` (from 1), with
    /// `stack` (top first). `earlier` is an earlier run from the same entry point.
    pub fn new(
        observed: &Observed,
        viewed: u32,
        stack: &[Frame],
        earlier: Option<&Run>,
        trace: Option<Located<'_>>,
    ) -> Flow {
        let mut flow = match trace {
            Some(located) => with_trace(observed, located),
            None => from_stops(observed, viewed),
        };
        if let Some(run) = earlier {
            flow.earlier = Some(add_earlier(&mut flow, observed, viewed, stack, run));
        }
        mark_stack(&mut flow.rows, stack);
        order(&mut flow.rows);
        let mut seen: HashSet<Edge> = HashSet::new();
        flow.edges = observed.edges.iter().chain(&flow.edges).filter(|e| seen.insert((*e).clone())).cloned().collect();
        flow
    }

    pub fn row(&self, path: &Path) -> Option<&Row> {
        self.rows.iter().find(|r| r.path == path)
    }
}

/// Rows from this run's stops, against stop numbers.
fn from_stops(observed: &Observed, viewed: u32) -> Flow {
    let rows = observed
        .files
        .iter()
        .map(|file| {
            let first = f64::from(*file.stops.first().unwrap_or(&1));
            let last = f64::from(*file.stops.last().unwrap_or(&1));
            let sources = Sources { observed: true, ..Sources::default() };
            Row {
                span: Some((first, last)),
                lines: stop_lines(observed, &file.path, 0),
                ..observed_row(file, Status::Ran, sources)
            }
        })
        .collect();
    let end = u32::try_from(observed.stops.len()).unwrap_or(u32::MAX).max(viewed);
    Flow { axis: Axis::Stops(end), now: f64::from(viewed), rows, edges: Vec::new(), earlier: None }
}

/// The lines of `path` the stops were at, at the stop numbers they were at, shifted by `offset`.
fn stop_lines(observed: &Observed, path: &Path, offset: i64) -> BTreeMap<u32, Vec<f64>> {
    let mut lines: BTreeMap<u32, Vec<f64>> = BTreeMap::new();
    for line in observed.file(path).map(|f| f.lines.iter().copied()).into_iter().flatten() {
        lines.entry(line).or_default();
    }
    for (index, stop) in observed.stops.iter().enumerate() {
        if let Some(top) = stop.top.as_ref().filter(|top| top.path == path) {
            let number = i64::try_from(index).unwrap_or(0) + 1 + offset;
            lines.entry(top.line).or_default().push(number as f64);
        }
    }
    lines
}

/// Rows from a trace, against its time; this run's files that it lacks keep their place
/// after the file they were reached from.
fn with_trace(observed: &Observed, located: Located<'_>) -> Flow {
    let Located { summary, position } = located;
    let now = position.time;
    let mut rows: Vec<Row> = Vec::new();
    for file in &observed.files {
        let traced = summary.file(&file.path);
        let sources = Sources { observed: true, trace: traced.is_some(), run: None };
        let mut row = observed_row(file, Status::Ran, sources);
        if let Some(traced) = traced {
            apply_trace(&mut row, traced);
        }
        rows.push(row);
    }
    for traced in &summary.files {
        if observed.file(&traced.path).is_some() {
            continue;
        }
        let status = if traced.first < now { Status::Ran } else { Status::Predicted };
        let sources = Sources { trace: true, ..Sources::default() };
        let mut row = Row::new(traced.path.clone(), is_library_path(&traced.path), status, sources);
        apply_trace(&mut row, traced);
        rows.push(row);
    }
    Flow { axis: Axis::Time(summary.end), now, rows, edges: summary.edges.clone(), earlier: None }
}

/// A row for a file a run's stops saw: its functions, the stops it was on the stack at and
/// the lines its frames were at.
fn observed_row(file: &ObservedFile, status: Status, sources: Sources) -> Row {
    let stops = u32::try_from(file.stops.len()).unwrap_or(u32::MAX);
    Row {
        functions: file.functions.clone(),
        count: stops,
        stops,
        lines: file.lines.iter().map(|line| (*line, Vec::new())).collect(),
        // Recorded by an older tuitor perhaps, which knew fewer library paths.
        ..Row::new(file.path.clone(), file.library || is_library_path(&file.path), status, sources)
    }
}

/// The trace's functions, calls, span and timed lines for `row`'s file.
fn apply_trace(row: &mut Row, traced: &TracedFile) {
    row.span = Some((traced.first, traced.last));
    row.count = traced.calls;
    row.first_calls = traced.first_calls.clone();
    for (name, calls) in &traced.functions {
        match row.functions.iter_mut().find(|(n, _)| n == name) {
            Some((_, count)) => *count = *calls,
            None => row.functions.push((name.clone(), *calls)),
        }
    }
    for (line, times) in &traced.lines {
        row.lines.entry(*line).or_default().extend(times);
    }
}

/// Adds the files `run` reached after the point this run is at and this run has not.
/// Its stops line up with this run's where it stopped where the viewed stop is: the n-th
/// time there in this run with its n-th time, or its last when it was there less often.
fn add_earlier(flow: &mut Flow, observed: &Observed, viewed: u32, stack: &[Frame], run: &Run) -> Alignment {
    let here = stack.first().and_then(Location::of);
    let matched = here.as_ref().and_then(|here| {
        let before = usize::try_from(viewed.saturating_sub(1)).unwrap_or(0).min(observed.stops.len());
        let times = observed.stops[..before].iter().filter(|s| s.top.as_ref() == Some(here)).count();
        let there: Vec<usize> =
            (0..run.observed.stops.len()).filter(|&i| run.observed.stops[i].top.as_ref() == Some(here)).collect();
        let index = *there.get(times).or(there.last())?;
        Some((here.clone(), u32::try_from(index).unwrap_or(0) + 1))
    });
    let after = |file: &ObservedFile| matched.as_ref().is_none_or(|(_, stop)| file.stops.first() > Some(stop));
    let ahead: Vec<&ObservedFile> = run
        .observed
        .files
        .iter()
        .filter(|f| observed.file(&f.path).is_none() && flow.row(&f.path).is_none() && after(f))
        .collect();
    // Without a stop to line up with, they come after this one, in the earlier run's order.
    let offset = match &matched {
        Some((_, stop)) => i64::from(viewed) - i64::from(*stop),
        None => {
            let first = ahead.iter().filter_map(|f| f.stops.first()).min().copied().unwrap_or(1);
            i64::from(viewed) + 1 - i64::from(first)
        }
    };
    let stops_axis = matches!(flow.axis, Axis::Stops(_));
    for file in &ahead {
        let shift = |stop: u32| (i64::from(stop) + offset) as f64;
        let span = match (file.stops.first(), file.stops.last()) {
            (Some(first), Some(last)) if stops_axis => Some((shift(*first), shift(*last))),
            _ => None,
        };
        if let (Some((_, end)), Axis::Stops(axis_end)) = (span, &mut flow.axis) {
            *axis_end = (*axis_end).max(end as u32);
        }
        let sources = Sources { run: Some(run.number), ..Sources::default() };
        let row = observed_row(file, Status::Predicted, sources);
        let lines = if stops_axis { stop_lines(&run.observed, &file.path, offset) } else { row.lines.clone() };
        flow.rows.push(Row { span, lines, ..row });
    }
    if matched.is_some() && stops_axis {
        continue_rows(flow, run, offset);
    }
    flow.edges.extend(run.observed.edges.iter().cloned());
    Alignment { run: run.number, started: run.started.clone(), matched, predicted: ahead.len() }
}

/// Where the earlier run went on in files this run has reached too: their lines it stopped
/// at after this point, the functions it first went into after it, and how far it stayed.
fn continue_rows(flow: &mut Flow, run: &Run, offset: i64) {
    let now = flow.now;
    let shift = |stop: u32| (i64::from(stop) + offset) as f64;
    for file in &run.observed.files {
        let Some(row) = flow.rows.iter_mut().find(|r| r.path == file.path && r.sources.observed) else { continue };
        let Some(last) = file.stops.last().map(|stop| shift(*stop)).filter(|last| *last > now) else { continue };
        for (line, at) in stop_lines(&run.observed, &file.path, offset) {
            row.lines.entry(line).or_default().extend(at.into_iter().filter(|at| *at > now));
        }
        for (name, count) in &file.functions {
            let first = file.first_seen.get(name).map(|stop| shift(*stop));
            if let Some(first) = first.filter(|first| *first > now)
                && !row.functions.iter().any(|(seen, _)| seen == name)
            {
                row.functions.push((name.clone(), *count));
                row.first_calls.insert(name.clone(), first);
            }
        }
        row.ahead = Some(last);
        row.sources.run = Some(run.number);
        if let Axis::Stops(end) = &mut flow.axis {
            *end = (*end).max(last as u32);
        }
    }
}

/// Files on the viewed stop's stack: where it executes, and where it was called from.
fn mark_stack(rows: &mut [Row], stack: &[Frame]) {
    for (depth, frame) in stack.iter().enumerate().rev() {
        let Some(path) = frame.path.as_deref() else { continue };
        if let Some(row) = rows.iter_mut().find(|r| r.path == path) {
            row.status = if depth == 0 { Status::Executing } else { Status::Stack };
            row.stack_line = Some(frame.line);
        }
    }
}

/// By first activity; a row the axis cannot place stays after the row before it.
fn order(rows: &mut Vec<Row>) {
    let mut key = f64::NEG_INFINITY;
    let mut keyed: Vec<(f64, Row)> = rows
        .drain(..)
        .map(|row| {
            if let Some((start, _)) = row.span {
                key = start;
            }
            (key, row)
        })
        .collect();
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.extend(keyed.into_iter().map(|(_, row)| row));
}

#[cfg(test)]
mod tests {
    use chrono::Local;

    use super::*;
    use crate::trace::{StackFrame, map_path};

    fn frame(name: &str, path: &str, line: u32) -> Frame {
        Frame {
            id: 0,
            name: name.into(),
            path: Some(PathBuf::from(path)),
            source_name: None,
            source_reference: None,
            line,
            column: 1,
            library: is_library_path(Path::new(path)),
            scopes: None,
        }
    }

    fn stop(number: u32, hit: bool, frames: Vec<Frame>) -> Snapshot {
        Snapshot {
            number,
            time: Local::now(),
            thread_id: 1,
            reason: if hit { "breakpoint" } else { "step" }.into(),
            description: None,
            hit_breakpoints: if hit { vec![1] } else { vec![] },
            set: hit.then_some(0),
            silent: false,
            frames,
            watches: vec![],
            live: false,
        }
    }

    /// index.php → Controller → Service, then the service steps into Money.
    fn shop_stops() -> Vec<Snapshot> {
        let main = || frame("{main}", "/w/public/index.php", 10);
        let invoke = |line| frame("Controller->__invoke", "/w/src/Controller.php", line);
        vec![
            stop(1, true, vec![invoke(15), main()]),
            stop(2, false, vec![frame("Service->assign", "/w/src/Service.php", 19), invoke(18), main()]),
            stop(
                3,
                false,
                vec![
                    frame("Money::zero", "/w/src/Money.php", 15),
                    frame("Service->assign", "/w/src/Service.php", 19),
                    invoke(18),
                    main(),
                ],
            ),
        ]
    }

    fn names(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.path.file_name().and_then(|n| n.to_str()).unwrap_or("?")).collect()
    }

    #[test]
    fn observes_files_outermost_first_with_functions_lines_and_calls() {
        let observed = Observed::from_stops(&shop_stops());
        let files: Vec<&str> =
            observed.files.iter().map(|f| f.path.file_name().and_then(|n| n.to_str()).unwrap_or("?")).collect();
        assert_eq!(files, ["index.php", "Controller.php", "Service.php", "Money.php"]);
        let controller = &observed.files[1];
        assert_eq!(controller.functions, [("Controller->__invoke".to_owned(), 3)]);
        assert_eq!(observed.files[3].first_seen, BTreeMap::from([("Money::zero".to_owned(), 3)]));
        assert_eq!(controller.lines, BTreeSet::from([15, 18]));
        assert_eq!(controller.stops, [1, 2, 3]);
        assert_eq!(
            observed.stops[0],
            Stop { top: Some(Location { path: "/w/src/Controller.php".into(), line: 15 }), hit: true }
        );
        let edge = observed.edges.iter().find(|e| e.callee == "Money::zero").expect("edge");
        assert_eq!((edge.caller.as_str(), edge.site.line), ("Service->assign", 19));
        assert_eq!(observed.edges.len(), 3, "each call once");
    }

    #[test]
    fn stops_place_files_and_the_stack_says_where_execution_is() {
        let stops = shop_stops();
        let observed = Observed::from_stops(&stops);
        // Looking back at stop 2: Money is reached later in this run.
        let flow = Flow::new(&observed, 2, &stops[1].frames, None, None);
        assert_eq!(flow.axis, Axis::Stops(3));
        assert_eq!(flow.now, 2.0);
        let status: Vec<Status> = flow.rows.iter().map(|r| r.status).collect();
        assert_eq!(status, [Status::Stack, Status::Stack, Status::Executing, Status::Ran]);
        let service = flow.row(Path::new("/w/src/Service.php")).expect("service");
        assert_eq!((service.span, service.stack_line), (Some((2.0, 3.0)), Some(19)));
        assert_eq!(service.lines[&19], [2.0], "the stop it was the top at");
    }

    fn earlier_run() -> Run {
        // The same start, then on past Money into the notifier and the mailer.
        let mut stops = shop_stops();
        let notify = || frame("Notifier->notify", "/w/src/Notifier.php", 14);
        let assign = || frame("Service->assign", "/w/src/Service.php", 23);
        let invoke = || frame("Controller->__invoke", "/w/src/Controller.php", 18);
        let main = || frame("{main}", "/w/public/index.php", 10);
        stops.push(stop(4, true, vec![notify(), assign(), invoke(), main()]));
        let mailer = frame("Mailer->send", "/w/vendor/acme/mailer/Mailer.php", 11);
        stops.push(stop(5, false, vec![mailer, notify(), assign(), invoke(), main()]));
        Run::new(3, "2026-10-20 01:12".into(), Observed::from_stops(&stops))
    }

    #[test]
    fn a_run_is_known_by_its_first_stop_and_the_breakpoint_it_first_hit() {
        let earlier = earlier_run();
        assert_eq!(earlier.first_hit.as_ref().map(|l| l.line), Some(15));
        let stops = shop_stops();
        assert!(Run::new(4, String::new(), Observed::from_stops(&stops[..1])).same_entry(&earlier));
        // Both stopped on entry; the earlier run went on to the notifier's breakpoint.
        let entry = || stop(1, false, vec![frame("{main}", "/w/public/index.php", 3)]);
        let mut entered = earlier.observed.clone();
        entered
            .stops
            .insert(0, Stop { top: Some(Location { path: "/w/public/index.php".into(), line: 3 }), hit: false });
        let entered = Run::new(3, String::new(), entered);
        let current = Run::new(4, String::new(), Observed::from_stops(&[entry()]));
        assert!(current.same_entry(&entered) && !current.same_entry(&earlier));
        let notify = frame("Notifier->notify", "/w/src/Notifier.php", 14);
        let at_notifier = Run::new(4, String::new(), Observed::from_stops(&[entry(), stop(2, true, vec![notify])]));
        assert!(at_notifier.same_entry(&entered));
        let other = frame("Other->run", "/w/src/Other.php", 3);
        let elsewhere = Run::new(4, String::new(), Observed::from_stops(&[entry(), stop(2, true, vec![other])]));
        assert!(!elsewhere.same_entry(&entered), "another request, by its first breakpoint");
    }

    #[test]
    fn the_nth_time_here_lines_up_with_the_earlier_runs_nth_time() {
        // The earlier run stopped at Money.php:15 at its stops 3 and 4.
        let mut stops = shop_stops();
        let mut again = stops[2].clone();
        again.number = 4;
        stops.push(again);
        let notify = frame("Notifier->notify", "/w/src/Notifier.php", 14);
        stops.push(stop(5, true, vec![notify]));
        let earlier = Run::new(3, String::new(), Observed::from_stops(&stops));
        let current = shop_stops();
        let flow = Flow::new(&Observed::from_stops(&current), 3, &current[2].frames, Some(&earlier), None);
        assert_eq!(
            flow.earlier.as_ref().and_then(|a| a.matched.as_ref()).map(|(_, stop)| *stop),
            Some(3),
            "the first time"
        );
        assert_eq!(flow.row(Path::new("/w/src/Notifier.php")).and_then(|r| r.span), Some((5.0, 5.0)));
    }

    #[test]
    fn an_earlier_run_predicts_the_files_it_reached_after_this_point() {
        let stops = shop_stops();
        let observed = Observed::from_stops(&stops);
        let earlier = earlier_run();
        let flow = Flow::new(&observed, 3, &stops[2].frames, Some(&earlier), None);
        assert_eq!(
            names(&flow.rows),
            ["index.php", "Controller.php", "Service.php", "Money.php", "Notifier.php", "Mailer.php"]
        );
        let notifier = flow.row(Path::new("/w/src/Notifier.php")).expect("notifier");
        assert_eq!((notifier.status, notifier.sources.run), (Status::Predicted, Some(3)));
        // Stop 3 of the earlier run was at Money.php:15, as this one is: stop 4 comes next.
        assert_eq!(notifier.span, Some((4.0, 5.0)));
        assert!(flow.row(Path::new("/w/vendor/acme/mailer/Mailer.php")).is_some_and(|r| r.library));
        assert_eq!(flow.axis, Axis::Stops(5));
        // Where it went on in files this run has seen: Service until its stop 5, from line 23.
        let service = flow.row(Path::new("/w/src/Service.php")).expect("service");
        assert_eq!((service.ahead, service.sources.run), (Some(5.0), Some(3)));
        assert!(service.lines.contains_key(&23));
        // Its functions this run has not been in yet are ahead.
        let stops = shop_stops();
        let at_entry = Flow::new(&Observed::from_stops(&stops[..1]), 1, &stops[0].frames, Some(&earlier), None);
        let service = at_entry.row(Path::new("/w/src/Controller.php")).expect("controller");
        assert!(service.first_calls.is_empty(), "the controller has nothing new");
        let money = &flow.row(Path::new("/w/src/Money.php")).expect("money").first_calls;
        assert!(money.is_empty(), "this run is in Money::zero already");
        // A breakpoint stops where the top of the stack is: Notifier.php:14 at its stop 4.
        assert_eq!(notifier.lines[&14], [4.0]);
        assert!(flow.edges.iter().any(|e| e.callee == "Notifier->notify"), "its calls are evidence too");
        let alignment = flow.earlier.expect("aligned");
        assert_eq!((alignment.run, alignment.predicted), (3, 2));
        assert_eq!(alignment.matched.map(|(l, stop)| (l.line, stop)), Some((15, 3)));
    }

    #[test]
    fn without_a_matching_stop_the_earlier_files_come_after_this_one() {
        let stops = shop_stops();
        let observed = Observed::from_stops(&stops[..1]);
        let flow = Flow::new(&observed, 1, &stops[0].frames, Some(&earlier_run()), None);
        // Stop 1 matched (Controller.php:15): everything the earlier run reached after it.
        assert_eq!(flow.rows.len(), 6);
        let elsewhere = vec![frame("other", "/w/src/Other.php", 1)];
        let flow = Flow::new(
            &Observed::from_stops(&[stop(1, false, elsewhere.clone())]),
            1,
            &elsewhere,
            Some(&earlier_run()),
            None,
        );
        let first = flow.rows.iter().find(|r| r.status == Status::Predicted).expect("predicted");
        assert_eq!(first.span.map(|s| s.0), Some(2.0));
        assert_eq!(flow.earlier.and_then(|a| a.matched), None);
    }

    const TRACE: &str = "Version: 3.5.3
File format: 4
TRACE START [2026-10-20 01:12:52.803481]
1\t0\t0\t0.001\t1\t{main}\t1\t\t/app/public/index.php\t0\t0
2\t1\t0\t0.002\t1\tController->__invoke\t1\t\t/app/public/index.php\t10\t0
3\t2\t0\t0.003\t1\tService->assign\t1\t\t/app/src/Controller.php\t18\t0
4\t3\t0\t0.004\t1\tMoney::zero\t1\t\t/app/src/Service.php\t19\t0
5\t4\t0\t0.0045\t1\tsprintf\t0\t\t/app/src/Money.php\t15\t0
5\t4\t1\t0.0046\t1
4\t3\t1\t0.005\t1
4\t5\t0\t0.006\t1\tNotifier->notify\t1\t\t/app/src/Service.php\t23\t0
5\t6\t0\t0.007\t1\tMailer->send\t1\t\t/app/src/Notifier.php\t14\t0
6\t7\t0\t0.0075\t1\tfile_put_contents\t0\t\t/app/vendor/acme/mailer/Mailer.php\t11\t0
6\t7\t1\t0.0076\t1
5\t6\t1\t0.008\t1
4\t5\t1\t0.009\t1
3\t2\t1\t0.010\t1
2\t1\t1\t0.011\t1
1\t0\t1\t0.012\t1
\t\t\t0.013\t1
TRACE END   [2026-10-20 01:12:52.816481]
";

    #[test]
    fn a_trace_times_the_files_and_predicts_what_follows_the_stop() {
        let mappings = [("/app".to_owned(), PathBuf::from("/w"))];
        let trace = Trace::parse(TRACE, &|p| map_path(p, &mappings)).expect("trace");
        let summary = TraceSummary::new(&trace, &HashMap::new());
        let stops = shop_stops();
        let stack: Vec<StackFrame<'_>> = stops[1]
            .frames
            .iter()
            .map(|f| StackFrame { name: &f.name, path: f.path.as_deref(), line: f.line })
            .collect();
        let position = trace.locate(&stack, 0).expect("the stack is in the trace");
        // Times count from the first call, at 1 ms.
        assert!((position.time - 0.003).abs() < 1e-9, "Money::zero is called from Service.php:19");
        let observed = Observed::from_stops(&stops[..2]);
        let flow =
            Flow::new(&observed, 2, &stops[1].frames, None, Some(Located { summary: &summary, position: &position }));
        assert!(matches!(flow.axis, Axis::Time(end) if (end - 0.011).abs() < 1e-9));
        assert_eq!(
            names(&flow.rows),
            ["index.php", "Controller.php", "Service.php", "Money.php", "Notifier.php", "Mailer.php"]
        );
        let money = flow.row(Path::new("/w/src/Money.php")).expect("money");
        // It made a call, so the trace knows its code; reached at the stop's line, not before.
        assert_eq!((money.status, money.sources.trace, money.sources.observed), (Status::Predicted, true, false));
        assert!(money.span.is_some_and(|(a, b)| (a - 0.003).abs() < 1e-9 && (b - 0.004).abs() < 1e-9));
        assert_eq!(money.functions, [("Money::zero".to_owned(), 1)]);
        let service = flow.row(Path::new("/w/src/Service.php")).expect("service");
        assert_eq!(service.status, Status::Executing);
        assert!(service.lines[&19].iter().all(|at| (at - 0.003).abs() < 1e-9));
        assert_eq!(service.count, 1);
        let mailer = flow.row(Path::new("/w/vendor/acme/mailer/Mailer.php")).expect("mailer");
        assert!(mailer.library && mailer.status == Status::Predicted);
        assert!(
            flow.edges
                .iter()
                .any(|e| e.caller == "Notifier->notify" && e.callee == "Mailer->send" && e.site.line == 14)
        );
    }
}
