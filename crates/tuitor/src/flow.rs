//! FLOW for a tab: the files of the session on screen, put together from its stops, the
//! runs remembered for the project, and an Xdebug trace when the session's configuration
//! names a directory of them (`xdebugTraceDir`). Runs are recorded here as stops come in.

use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use serde_json::Value;
use tuitor_core::flow::{Flow, Located, Observed, Row, Run, Status, TraceSummary};
use tuitor_core::persist;
use tuitor_core::session::{Session, SessionId, SessionState};
use tuitor_core::snapshot::{Frame, Snapshot};
use tuitor_core::trace::{self, Position, StackFrame, Trace, TraceFile, map_path};

use crate::model::package_of;
use crate::outline::{self, Symbol};
use crate::tab::Tab;
use crate::ui::syntax::Lang;

/// The field of a launch configuration that names the directory Xdebug writes traces to,
/// as PHP sees it; `pathMappings` turn it and the paths in the traces into local ones.
pub const TRACE_DIR: &str = "xdebugTraceDir";
/// Newest traces tried for one that took the path of the viewed stop.
const TRACES_TRIED: usize = 8;
/// Larger traces are left alone: reading one would stall the screen.
const TRACE_BYTES: u64 = 256 << 20;

/// What a tab keeps for FLOW.
#[derive(Default)]
pub struct FlowState {
    /// Where the project's runs are remembered; `None` in a tab without a project.
    file: Option<PathBuf>,
    /// Oldest first, including the runs of this tab's sessions.
    runs: Vec<Run>,
    /// Where the runs of each session start, as indices into its history. A session is one
    /// run, unless its adapter debugs request after request in it, as vscode-php-debug does
    /// while it listens: each request is a thread of its own, gone by the next one's stop.
    starts: HashMap<SessionId, Vec<usize>>,
    /// The number each run is recorded under, by session and start.
    numbers: HashMap<(SessionId, usize), u32>,
    /// Stops recorded per session.
    recorded: HashMap<SessionId, usize>,
    traces: RefCell<HashMap<PathBuf, Rc<CachedTrace>>>,
    cache: RefCell<Option<(Key, Rc<FlowView>)>>,
}

struct CachedTrace {
    modified: SystemTime,
    len: u64,
    mappings: Vec<(String, PathBuf)>,
    trace: Result<Trace, String>,
    /// Functions declared in the files the trace names, by the names traces give them.
    declared: OnceCell<HashMap<String, PathBuf>>,
}

/// What a flow is computed from; a new stop, another stop viewed or the end of the
/// session (when its own trace becomes complete) computes it again.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Key {
    session: SessionId,
    run: Range<usize>,
    stops: usize,
    viewed: u32,
    state: SessionState,
    runs: usize,
}

/// The flow at the viewed stop, with what it was made from.
pub struct FlowView {
    pub flow: Flow,
    /// The number this run is recorded under.
    pub run: Option<u32>,
    /// Stops of the session before this run's first: the flow counts its stops from 1.
    pub base: u32,
    pub stops: usize,
    pub trace: TraceNote,
}

/// Whether a trace was used, and if not, why.
#[derive(Debug, Clone, PartialEq)]
pub enum TraceNote {
    /// The session's configuration names no trace directory, and it is not a PHP one.
    NotPhp,
    /// A PHP configuration without [`TRACE_DIR`].
    Off,
    Unreadable {
        dir: PathBuf,
        error: String,
    },
    /// No finished trace in `dir` took this stop's path.
    NoMatch {
        dir: PathBuf,
        checked: usize,
        running: usize,
    },
    Found {
        file: PathBuf,
        modified: SystemTime,
        started: Option<String>,
        calls: usize,
        end: f64,
        now: f64,
        candidates: usize,
        pass: usize,
        /// Stops in a debugger left out of its times, and their length.
        pauses: usize,
        paused: f64,
    },
}

impl FlowState {
    pub fn open(project: &Path) -> FlowState {
        let file = persist::flows_file(project);
        // Runs that cannot be read only cost predictions.
        let runs = file.as_deref().and_then(|f| persist::load_runs(f).ok()).unwrap_or_default();
        FlowState { file, runs, ..FlowState::default() }
    }

    /// Records the stops of `sessions` that are new in the runs they belong to.
    pub fn record(&mut self, sessions: &[Session]) {
        for session in sessions {
            let stops = session.history.len();
            let done = self.recorded.get(&session.id).copied().unwrap_or(0);
            if stops == done {
                continue;
            }
            self.recorded.insert(session.id, stops);
            let starts = self.starts.entry(session.id).or_default();
            starts.extend((done..stops).filter(|&i| i == 0 || next_request(session, i)));
            let runs: Vec<Range<usize>> = runs(starts, stops).into_iter().filter(|run| run.end > done).collect();
            for run in runs {
                self.remember(session, run);
            }
        }
    }

    fn remember(&mut self, session: &Session, run: Range<usize>) {
        let Some(file) = self.file.clone() else { return };
        let next = self.runs.iter().map(|r| r.number).chain(self.numbers.values().copied()).max().unwrap_or(0) + 1;
        let number = *self.numbers.entry((session.id, run.start)).or_insert(next);
        let started = session.history[run.start].time.format("%Y-%m-%d %H:%M").to_string();
        let run = Run::new(number, started, Observed::from_stops(&session.history[run]));
        match persist::remember_run(&file, &run) {
            Ok(runs) => self.runs = runs,
            Err(err) => tracing::warn!("remembering run #{number}: {err:#}"),
        }
    }

    /// The stops of the run of `session` that stop index `index` belongs to.
    fn run_of(&self, session: &Session, index: usize) -> Range<usize> {
        let starts = self.starts.get(&session.id).map(Vec::as_slice).unwrap_or_default();
        runs(starts, session.history.len())
            .into_iter()
            .find(|run| run.contains(&index))
            .unwrap_or(0..session.history.len())
    }
}

/// A run of this tab's sessions, whose stops are still in memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRef {
    pub session: SessionId,
    /// Its stops, as indices into the session's history.
    pub stops: Range<usize>,
    /// The number it is remembered under, in a tab with a project.
    pub number: Option<u32>,
}

impl RunRef {
    /// `run #4`, or the session's name for a run that is not remembered.
    pub fn label(&self, sessions: &[Session]) -> String {
        match self.number {
            Some(number) => format!("run #{number}"),
            None => sessions.iter().find(|s| s.id == self.session).map_or_else(String::new, |s| s.name.clone()),
        }
    }
}

impl FlowState {
    /// The runs of `sessions` that have stops, in the order they began.
    pub fn in_memory(&self, sessions: &[Session]) -> Vec<RunRef> {
        let mut found = Vec::new();
        for session in sessions.iter().filter(|s| !s.history.is_empty()) {
            let starts = self.starts.get(&session.id).map_or(&[0][..], Vec::as_slice);
            for stops in runs(starts, session.history.len()) {
                let number = self.numbers.get(&(session.id, stops.start)).copied();
                found.push((session.history[stops.start].time, RunRef { session: session.id, stops, number }));
            }
        }
        found.sort_by_key(|(began, _)| *began);
        found.into_iter().map(|(_, run)| run).collect()
    }
}

/// Whether stop `index` of `session` is the first of another request: it stopped another
/// thread than the stop before, and that thread is gone. Threads of one program that are
/// stopped in turn are all still there.
fn next_request(session: &Session, index: usize) -> bool {
    let (before, now) = (&session.history[index - 1], &session.history[index]);
    before.thread_id != now.thread_id && !session.threads.iter().any(|t| t.id == before.thread_id)
}

/// The runs that start at `starts`, among `stops` stops.
fn runs(starts: &[usize], stops: usize) -> Vec<Range<usize>> {
    starts.iter().enumerate().map(|(i, &start)| start..starts.get(i + 1).copied().unwrap_or(stops)).collect()
}

impl Tab {
    /// The flow of the run of the active session at the viewed stop; `None` before its
    /// first stop.
    pub fn flow(&self) -> Option<Rc<FlowView>> {
        let session = self.active()?;
        let viewed = self.viewed()?;
        let run = self.flows.run_of(session, usize::try_from(viewed.number).ok()?.checked_sub(1)?);
        let key = Key {
            session: session.id,
            run: run.clone(),
            stops: session.history.len(),
            viewed: viewed.number,
            state: session.state,
            runs: self.flows.runs.len(),
        };
        if let Some((cached, view)) = &*self.flows.cache.borrow()
            && *cached == key
        {
            return Some(view.clone());
        }
        let view = Rc::new(self.compute_flow(session, viewed, run));
        *self.flows.cache.borrow_mut() = Some((key, view.clone()));
        Some(view)
    }

    fn compute_flow(&self, session: &Session, viewed: &Snapshot, run: Range<usize>) -> FlowView {
        let stops = &session.history[run.clone()];
        let base = u32::try_from(run.start).unwrap_or(0);
        let observed = Observed::from_stops(stops);
        let number = self.flows.numbers.get(&(session.id, run.start)).copied();
        let this = Run::new(number.unwrap_or(0), String::new(), observed.clone());
        let earlier: Vec<&Run> =
            self.flows.runs.iter().rev().filter(|r| Some(r.number) != number && this.same_entry(r)).collect();
        let (found, note) = self.find_trace(session, stops, viewed, &observed);
        let located = found.as_ref().map(|(summary, position)| Located { summary, position });
        let build = |run: Option<&Run>| {
            let located = located.as_ref().map(|l| Located { summary: l.summary, position: l.position });
            Flow::new(&observed, viewed.number - base, &viewed.frames, run, located)
        };
        // The latest earlier run that still has something to predict, else the latest.
        let flow = earlier
            .iter()
            .map(|run| build(Some(run)))
            .find(|flow| flow.earlier.as_ref().is_some_and(|a| a.predicted > 0))
            .unwrap_or_else(|| build(earlier.first().copied()));
        FlowView { flow, run: number, base, stops: stops.len(), trace: note }
    }

    /// The newest finished trace in the configured directory that took the viewed stop's
    /// path. A trace still being written belongs to a request still running, likely this
    /// one, and is passed over.
    fn find_trace(
        &self,
        session: &Session,
        stops: &[Snapshot],
        viewed: &Snapshot,
        observed: &Observed,
    ) -> (Option<(TraceSummary, Position)>, TraceNote) {
        let root = self.engine.session(self.engine.root(session.id)).unwrap_or(session);
        let arguments = &root.config.arguments;
        let Some(dir) = arguments.get(TRACE_DIR).and_then(Value::as_str).filter(|d| !d.is_empty()) else {
            let note = if root.config.adapter == "php" { TraceNote::Off } else { TraceNote::NotPhp };
            return (None, note);
        };
        let mappings = path_mappings(arguments);
        let dir = self.engine.workspace().join(map_path(dir, &mappings));
        let files = match trace::list(&dir) {
            Ok(files) => files,
            Err(err) => return (None, TraceNote::Unreadable { dir, error: err.to_string() }),
        };
        self.flows.traces.borrow_mut().retain(|path, _| files.iter().any(|f| &f.path == path));
        let stack: Vec<StackFrame<'_>> =
            viewed.frames.iter().map(|f| StackFrame { name: &f.name, path: f.path.as_deref(), line: f.line }).collect();
        let occurrence = occurrence(stops, viewed);
        // Functions that make no calls are placed by what was seen on stacks, here and in
        // earlier runs, else by where a file declares them.
        let seen = self.flows.runs.iter().map(|run| &run.observed).chain([observed]);
        let stacks: Vec<(String, PathBuf)> = seen
            .flat_map(|o| &o.files)
            .flat_map(|file| file.functions.iter().map(|(name, _)| (name.clone(), file.path.clone())))
            .collect();
        let (mut checked, mut running) = (0, 0);
        for file in files.iter().filter(|f| f.len <= TRACE_BYTES).take(TRACES_TRIED) {
            let cached = self.load_trace(file, &mappings);
            let Ok(trace) = &cached.trace else { continue };
            if !trace.complete {
                running += 1;
                continue;
            }
            checked += 1;
            if let Some(position) = trace.locate(&stack, occurrence) {
                let mut known = cached.declared.get_or_init(|| declared(trace)).clone();
                known.extend(stacks);
                let note = TraceNote::Found {
                    file: file.path.clone(),
                    modified: file.modified,
                    started: trace.started.clone(),
                    calls: trace.calls.len(),
                    end: trace.end,
                    now: position.time,
                    candidates: position.candidates,
                    pass: position.pass,
                    pauses: trace.pauses,
                    paused: trace.paused,
                };
                return (Some((TraceSummary::new(trace, &known), position)), note);
            }
        }
        (None, TraceNote::NoMatch { dir, checked, running })
    }

    /// `file` parsed, from the cache while it has not changed.
    fn load_trace(&self, file: &TraceFile, mappings: &[(String, PathBuf)]) -> Rc<CachedTrace> {
        if let Some(cached) = self.flows.traces.borrow().get(&file.path)
            && cached.modified == file.modified
            && cached.len == file.len
            && cached.mappings == mappings
        {
            return cached.clone();
        }
        let map = |raw: &str| {
            let path = map_path(raw, mappings);
            std::fs::canonicalize(&path).unwrap_or(path)
        };
        let trace = trace::read(&file.path, TRACE_BYTES)
            .map_err(|e| e.to_string())
            .and_then(|text| Trace::parse(&text, &map).map_err(|e| format!("{e:#}")));
        let cached = Rc::new(CachedTrace {
            modified: file.modified,
            len: file.len,
            mappings: mappings.to_vec(),
            trace,
            declared: OnceCell::new(),
        });
        self.flows.traces.borrow_mut().insert(file.path.clone(), cached.clone());
        cached
    }
}

/// The functions and methods declared in the files `trace` names, as OUTLINE reads them,
/// by the names a trace gives them without namespace (`Money->add`, `Money::zero`,
/// `lineTotal`). A name declared in more than one file is left out.
fn declared(trace: &Trace) -> HashMap<String, PathBuf> {
    let mut found: HashMap<String, Option<PathBuf>> = HashMap::new();
    let files: BTreeSet<&PathBuf> = trace.files.iter().collect();
    for path in files {
        let lang = Lang::from_path(path);
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        let lines: Vec<String> = text.lines().map(str::to_owned).collect();
        for symbol in crate::outline::outline(&lines, lang) {
            let names = match &symbol.container {
                Some(owner) => vec![format!("{owner}->{}", symbol.name), format!("{owner}::{}", symbol.name)],
                None => vec![symbol.name.clone()],
            };
            for name in names {
                found
                    .entry(name)
                    .and_modify(|at| {
                        if at.as_ref() != Some(path) {
                            *at = None;
                        }
                    })
                    .or_insert_with(|| Some(path.clone()));
            }
        }
    }
    found.into_iter().filter_map(|(name, path)| Some((name, path?))).collect()
}

/// `pathMappings` of a configuration: the path the adapter sees → the local one.
fn path_mappings(arguments: &serde_json::Map<String, Value>) -> Vec<(String, PathBuf)> {
    let Some(Value::Object(mappings)) = arguments.get("pathMappings") else { return Vec::new() };
    mappings.iter().filter_map(|(remote, local)| Some((remote.clone(), PathBuf::from(local.as_str()?)))).collect()
}

/// How many earlier stops of `history` had the same stack as `viewed`: the times the
/// program went this way before, which picks the matching call in a trace.
fn occurrence(history: &[Snapshot], viewed: &Snapshot) -> usize {
    let same = |a: &[Frame], b: &[Frame]| {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.name == y.name && x.path == y.path && x.line == y.line)
    };
    history.iter().filter(|s| s.number < viewed.number && same(&s.frames, &viewed.frames)).count()
}

/// A line of FLOW: a file, or library files reached one after another, as one line.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    File(usize),
    Library { package: String, rows: Vec<usize> },
}

impl Item {
    pub fn rows(&self) -> Vec<usize> {
        match self {
            Item::File(row) => vec![*row],
            Item::Library { rows, .. } => rows.clone(),
        }
    }

    /// The most current status of its files.
    pub fn status(&self, flow: &Flow) -> Status {
        self.rows().iter().map(|&r| flow.rows[r].status).min().unwrap_or(Status::Predicted)
    }
}

/// The lines of FLOW. Library files reached one after another, from the same package, are
/// one line unless `expanded`.
pub fn items(flow: &Flow, expanded: bool) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    for (index, row) in flow.rows.iter().enumerate() {
        if expanded || !row.library {
            items.push(Item::File(index));
            continue;
        }
        let package = package_of(&row.path);
        // A library file of the same package just before becomes a line with this one.
        let previous = match items.last() {
            Some(Item::File(previous))
                if flow.rows[*previous].library && package_of(&flow.rows[*previous].path) == package =>
            {
                Some(*previous)
            }
            _ => None,
        };
        match (items.last_mut(), previous) {
            (Some(Item::Library { package: last, rows }), _) if *last == package => rows.push(index),
            (Some(last), Some(previous)) => *last = Item::Library { package, rows: vec![previous, index] },
            _ => items.push(Item::File(index)),
        }
    }
    items
}

/// The line of the file where execution is, else of the innermost one on the stack.
pub fn executing(flow: &Flow, items: &[Item]) -> Option<usize> {
    let is = |status: Status| move |item: &Item| item.status(flow) == status;
    items.iter().position(is(Status::Executing)).or_else(|| items.iter().rposition(is(Status::Stack)))
}

/// A function name without its namespace, and without its class when the file is named
/// after the class: `App\Domain\Money->add` in Money.php → `add`.
pub fn short_function(name: &str, path: &Path) -> String {
    // Xdebug names a closure after its file and lines: `{closure:/app/a.php:99-101}`.
    if let Some(lines) = name.strip_prefix("{closure:").and_then(|n| n.rsplit_once(':')).map(|(_, l)| l) {
        let first = lines.split('-').next().unwrap_or_default();
        return format!("{{closure:{first}}}");
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    let method = crate::outline::short_name(name);
    match crate::outline::owner_name(name) {
        Some(owner) if owner == stem => method,
        Some(owner) => format!("{owner}{}{method}", if name.contains("->") { "->" } else { "::" }),
        None => method,
    }
}

/// `{main}` or a closure is code at the top of a file, not a function to find.
pub fn is_named_function(name: &str) -> bool {
    !name.starts_with('{') && !name.starts_with('<')
}

/// The symbol that the first named function of `row` stands for, in `symbols`.
pub fn first_symbol<'a>(row: &Row, symbols: &'a [Symbol]) -> Option<&'a Symbol> {
    row.functions.iter().filter(|(name, _)| is_named_function(name)).find_map(|(name, _)| outline::find(symbols, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuitor_core::flow::{Axis, Sources};

    fn row(path: &str, library: bool, status: Status) -> Row {
        Row::new(PathBuf::from(path), library, status, Sources::default())
    }

    #[test]
    fn library_files_one_after_another_are_one_line() {
        let rows = vec![
            row("/w/public/index.php", false, Status::Stack),
            row("/w/vendor/symfony/http-kernel/Kernel.php", true, Status::Stack),
            row("/w/vendor/symfony/http-kernel/HttpKernel.php", true, Status::Ran),
            row("/w/src/Controller.php", false, Status::Executing),
            row("/w/vendor/doctrine/orm/A.php", true, Status::Predicted),
            row("/w/vendor/symfony/mailer/Mailer.php", true, Status::Predicted),
        ];
        let flow = Flow { axis: Axis::Stops(1), now: 1.0, rows, edges: vec![], earlier: None };
        let collapsed = items(&flow, false);
        assert_eq!(
            collapsed,
            [
                Item::File(0),
                Item::Library { package: "symfony/http-kernel".into(), rows: vec![1, 2] },
                Item::File(3),
                Item::File(4),
                Item::File(5),
            ]
        );
        assert_eq!(collapsed[1].status(&flow), Status::Stack);
        assert_eq!(executing(&flow, &collapsed), Some(2));
        assert_eq!(items(&flow, true).len(), 6);
    }

    #[test]
    fn function_names_lose_what_the_file_already_says() {
        let money = Path::new("/w/src/Domain/Money.php");
        assert_eq!(short_function("App\\Domain\\Money->add", money), "add");
        assert_eq!(short_function("App\\Domain\\Money::zero", money), "zero");
        assert_eq!(short_function("App\\Service\\Ledger->post", money), "Ledger->post");
        assert_eq!(short_function("orderTotal", Path::new("/w/orders.js")), "orderTotal");
        assert!(!is_named_function("{main}") && !is_named_function("{closure:/w/a.php:3-5}"));
        assert_eq!(short_function("{closure:/w/orders.php:99-101}", Path::new("/w/orders.php")), "{closure:99}");
    }

    #[test]
    fn earlier_stops_with_the_same_stack_count_as_occurrences() {
        let frame = |line| Frame {
            id: 0,
            name: "orderTotal".into(),
            path: Some("/w/orders.php".into()),
            source_name: None,
            source_reference: None,
            line,
            column: 1,
            library: false,
            scopes: None,
        };
        let stop = |number, line| Snapshot {
            number,
            time: chrono::Local::now(),
            thread_id: 1,
            reason: "breakpoint".into(),
            description: None,
            hit_breakpoints: vec![],
            sets: vec![],
            silent: false,
            frames: vec![frame(line)],
            watches: vec![],
            live: false,
        };
        let history = [stop(1, 68), stop(2, 70), stop(3, 68), stop(4, 68)];
        assert_eq!(occurrence(&history, &history[3]), 2);
        assert_eq!(occurrence(&history, &history[0]), 0);
    }
}
