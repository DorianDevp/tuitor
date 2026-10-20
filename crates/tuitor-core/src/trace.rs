//! Xdebug function traces in the computerized format (`xdebug.trace_format=1`). A trace
//! has a line for every call a request made: its depth, the time it started and ended,
//! the function, and the file and line it was called *from*. Where a function's own code
//! is, the trace does not say; [`Trace::bodies`] works it out from what it does say.
//!
//! <https://xdebug.org/docs/trace#trace_format>

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Result, bail};

/// A gap between two lines of a trace longer than this is a stop in a debugger, unless PHP
/// spent it in a function of its own.
const PAUSE: f64 = 0.1;

/// One call. Times are seconds since the request started.
#[derive(Debug, Clone)]
pub struct Call {
    pub depth: u32,
    pub name: String,
    /// A function written in PHP, as opposed to one built into PHP or an extension.
    pub user: bool,
    /// For `include`, `require` and their `_once` forms: the file included.
    pub include: Option<FileId>,
    /// Where it was called from.
    pub file: FileId,
    pub line: u32,
    pub start: f64,
    /// `None` when the trace ends before the call returns.
    pub end: Option<f64>,
    pub parent: Option<usize>,
    /// The first argument as Xdebug wrote it (`'App\\Domain\\Money'`).
    pub first_arg: Option<String>,
    /// For a closure, the file it is written in: Xdebug names closures after it.
    pub closure_in: Option<FileId>,
}

/// Index into [`Trace::files`].
pub type FileId = u32;

#[derive(Debug, Clone, Default)]
pub struct Trace {
    /// "2026-10-20 01:12:52.803481", from `TRACE START`.
    pub started: Option<String>,
    /// The trace has its `TRACE END` line: the request finished. One still being written
    /// belongs to a request that runs, possibly the one being debugged.
    pub complete: bool,
    /// Every file named in the trace, after path mapping.
    pub files: Vec<PathBuf>,
    pub calls: Vec<Call>,
    /// When the last call ended, or the last time the trace mentions for one still running.
    pub end: f64,
    /// Gaps left out of the times as stops in a debugger, and how long they were in all.
    pub pauses: usize,
    pub paused: f64,
}

/// A frame of a stack to find in a trace, top first.
#[derive(Debug, Clone, Copy)]
pub struct StackFrame<'a> {
    pub name: &'a str,
    pub path: Option<&'a Path>,
    pub line: u32,
}

/// Where a stack is in a trace: the calls along it, outermost first, and the time the
/// stopped line is reached.
#[derive(Debug, Clone, PartialEq)]
pub struct Position {
    pub path: Vec<usize>,
    pub time: f64,
    /// How often the trace passes the stopped line on the same path, and which of those
    /// passes this is, from 0.
    pub candidates: usize,
    pub pass: usize,
}

/// Times of a trace from its first call, without the stops in a debugger: a request that
/// was debugged while it was traced would have them in its times, and a debugger connecting
/// takes time before the first call. Time spent in a function of PHP's own (`usleep`, a
/// query) shows as a gap between its entry and its exit, and stays.
#[derive(Default)]
struct Clock {
    last: Option<f64>,
    shift: f64,
    pauses: usize,
    paused: f64,
}

impl Clock {
    /// `raw` without the pauses before it; `in_php` when the gap since the last line was
    /// spent in PHP's own function.
    fn at(&mut self, raw: f64, in_php: bool) -> f64 {
        let Some(last) = self.last else {
            self.last = Some(raw);
            self.shift = raw;
            return 0.0;
        };
        let gap = raw - last;
        if gap > PAUSE && !in_php {
            self.shift += gap;
            self.pauses += 1;
            self.paused += gap;
        }
        self.last = Some(raw);
        raw - self.shift
    }
}

impl Trace {
    /// Reads a trace; `map` turns the paths it names (as PHP saw them, in a container
    /// perhaps) into local ones.
    pub fn parse(text: &str, map: &dyn Fn(&str) -> PathBuf) -> Result<Trace> {
        let mut trace = Trace::default();
        let mut ids: HashMap<String, FileId> = HashMap::new();
        let mut file_id = |trace: &mut Trace, raw: &str| -> FileId {
            *ids.entry(raw.to_owned()).or_insert_with(|| {
                trace.files.push(map(raw));
                FileId::try_from(trace.files.len() - 1).unwrap_or(FileId::MAX)
            })
        };
        let mut by_number: HashMap<&str, usize> = HashMap::new();
        // The open call at each depth, outermost first.
        let mut open: Vec<usize> = Vec::new();
        let mut format_seen = false;
        let mut clock = Clock::default();
        // A call into PHP's own function that the last line started.
        let mut in_php: Option<&str> = None;

        for line in text.lines() {
            if let Some(start) = line.strip_prefix("TRACE START") {
                trace.started = Some(bracketed(start).to_owned());
                continue;
            }
            if line.starts_with("TRACE END") {
                trace.complete = true;
                continue;
            }
            if let Some(format) = line.strip_prefix("File format:") {
                format_seen = true;
                if !matches!(format.trim(), "2" | "3" | "4") {
                    bail!("file format {} is not one tuitor reads", format.trim());
                }
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            // The footer's time, when tracing stopped after the request's code, is left out.
            match fields.as_slice() {
                [depth, number, "0", time, _memory, name, user, include, file, line, rest @ ..] => {
                    let (Ok(depth), Ok(start)) = (depth.parse::<u32>(), time.parse::<f64>()) else { continue };
                    let start = clock.at(start, false);
                    in_php = (*user == "0").then_some(*number);
                    let depth_index = usize::try_from(depth.max(1) - 1).unwrap_or(0);
                    open.truncate(depth_index);
                    let include = (!include.is_empty()).then(|| file_id(&mut trace, include));
                    let closure_in = closure_file(name).map(|file| file_id(&mut trace, file));
                    let call = Call {
                        depth,
                        name: (*name).to_owned(),
                        user: *user == "1",
                        include,
                        file: file_id(&mut trace, file),
                        line: line.parse().unwrap_or(0),
                        start,
                        end: None,
                        parent: open.last().copied(),
                        first_arg: rest.get(1).filter(|arg| !arg.is_empty()).map(|arg| (*arg).to_owned()),
                        closure_in,
                    };
                    trace.end = trace.end.max(start);
                    trace.calls.push(call);
                    let index = trace.calls.len() - 1;
                    by_number.insert(number, index);
                    open.push(index);
                }
                [_, number, "1", time, ..] => {
                    let (Some(&index), Ok(end)) = (by_number.get(number), time.parse::<f64>()) else { continue };
                    let end = clock.at(end, in_php.take() == Some(*number));
                    trace.calls[index].end = Some(end);
                    trace.end = trace.end.max(end);
                }
                _ => {}
            }
        }
        if !format_seen {
            bail!("not a computerized Xdebug trace (xdebug.trace_format=1)");
        }
        (trace.pauses, trace.paused) = (clock.pauses, clock.paused);
        Ok(trace)
    }

    pub fn file(&self, id: FileId) -> &Path {
        usize::try_from(id).ok().and_then(|i| self.files.get(i)).map_or(Path::new(""), PathBuf::as_path)
    }

    fn id_of(&self, path: &Path) -> Option<FileId> {
        self.files.iter().position(|f| f == path).and_then(|i| FileId::try_from(i).ok())
    }

    /// When a call ended, or the end of the trace for one that had not.
    pub fn end_of(&self, index: usize) -> f64 {
        self.calls[index].end.unwrap_or(self.end)
    }

    /// The calls made directly by each call, in order.
    pub fn children(&self) -> Vec<Vec<usize>> {
        let mut children = vec![Vec::new(); self.calls.len()];
        for (index, call) in self.calls.iter().enumerate() {
            if let Some(parent) = call.parent {
                children[parent].push(index);
            }
        }
        children
    }

    /// The file whose code each call ran, where the trace tells: a closure is named after
    /// its file; an `include` runs the file it includes; a call that made calls ran where
    /// those were made from; a method, where its class was loaded from by an autoloader or
    /// where another method of the class ran; a function, where it ran at another call.
    /// `known` adds what was seen elsewhere, such as function names on a debugger's stack.
    /// Calls into PHP itself (`strlen`) have none.
    pub fn bodies<'a>(&'a self, known: &'a HashMap<String, PathBuf>) -> Vec<Option<&'a Path>> {
        let children = self.children();
        let mut bodies: Vec<Option<FileId>> = vec![None; self.calls.len()];
        let mut classes: HashMap<String, FileId> = HashMap::new();
        let mut functions: HashMap<&str, FileId> = HashMap::new();
        for (index, call) in self.calls.iter().enumerate() {
            if let Some(file) = call.include {
                bodies[index] = Some(file);
                if let Some(class) = self.loading(index) {
                    classes.entry(class).or_insert(file);
                }
            } else if call.closure_in.is_some() {
                bodies[index] = call.closure_in;
            }
            if bodies[index].is_none()
                && call.user
                && let Some(&first) = children[index].first()
            {
                bodies[index] = Some(self.calls[first].file);
            }
            if let (Some(body), true) = (bodies[index], call.user) {
                functions.entry(&call.name).or_insert(body);
                if let Some(class) = class_of(&call.name) {
                    classes.entry(class.to_owned()).or_insert(body);
                }
            }
        }
        self.calls
            .iter()
            .zip(bodies)
            .map(|(call, body)| {
                let found = || {
                    let class = class_of(&call.name).and_then(|class| classes.get(class));
                    class.or_else(|| functions.get(call.name.as_str())).copied()
                };
                match body.or_else(|| call.user.then(found).flatten()) {
                    Some(id) => Some(self.file(id)),
                    None if call.user => {
                        let known = known.get(&call.name).or_else(|| known.get(without_namespace(&call.name)));
                        known.map(PathBuf::as_path)
                    }
                    None => None,
                }
            })
            .collect()
    }

    /// The class an `include` loads, when it is made by an autoloader: a call above it,
    /// the nearest one with an argument, was given the class name.
    fn loading(&self, include: usize) -> Option<String> {
        let mut at = self.calls[include].parent;
        for _ in 0..4 {
            let call = &self.calls[at?];
            if let Some(arg) = &call.first_arg {
                return class_name(arg);
            }
            at = call.parent;
        }
        None
    }

    /// Finds `stack` (top first): the call of its outermost function from the same file, and
    /// below it each next one called from the line the frame above it is stopped on. When
    /// the path was taken more than once (a loop, a function called again), `occurrence`
    /// picks which; it counts from 0 and stops at the last one.
    pub fn locate(&self, stack: &[StackFrame<'_>], occurrence: usize) -> Option<Position> {
        let (bottom, rest) = stack.split_last()?;
        let bottom_file = self.id_of(bottom.path?)?;
        let children = self.children();
        let mut paths: Vec<Vec<usize>> = (0..self.calls.len())
            .filter(|&i| {
                let call = &self.calls[i];
                call.parent.is_none() && call.name == bottom.name && call.file == bottom_file
            })
            .map(|i| vec![i])
            .collect();
        // Each frame was called from the line its caller, the frame below, is on.
        let mut caller = bottom;
        for frame in rest.iter().rev() {
            let file = caller.path.and_then(|p| self.id_of(p));
            paths = paths
                .into_iter()
                .flat_map(|path| {
                    let last = *path.last().unwrap_or(&0);
                    children[last]
                        .iter()
                        .filter(|&&c| {
                            let call = &self.calls[c];
                            call.name == frame.name && Some(call.file) == file && call.line == caller.line
                        })
                        .map(|&c| {
                            let mut longer = path.clone();
                            longer.push(c);
                            longer
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
            caller = frame;
        }
        // Every time the stopped line was passed on this path, in order: once per call it made
        // (a loop goes round it again, or the function is called again), or, for a line that
        // calls nothing, once per call of the function, after what came before the line.
        let top = stack.first()?;
        let passes: Vec<(&Vec<usize>, f64)> = paths
            .iter()
            .flat_map(|path| {
                let call = *path.last().unwrap_or(&0);
                let made = &children[call];
                let at_line: Vec<f64> =
                    made.iter().filter(|&&c| self.calls[c].line == top.line).map(|&c| self.calls[c].start).collect();
                if at_line.is_empty() {
                    let before = made.iter().rev().find(|&&c| self.calls[c].line < top.line);
                    vec![(path, before.map_or(self.calls[call].start, |&c| self.end_of(c)))]
                } else {
                    at_line.into_iter().map(|time| (path, time)).collect()
                }
            })
            .collect();
        let pass = occurrence.min(passes.len().checked_sub(1)?);
        let (path, time) = passes.get(pass)?;
        Some(Position { path: (*path).clone(), time: *time, candidates: passes.len(), pass })
    }
}

/// The text between `[` and `]`.
fn bracketed(text: &str) -> &str {
    let text = text.trim();
    text.strip_prefix('[').and_then(|t| t.strip_suffix(']')).unwrap_or(text)
}

/// `{closure:/app/src/x.php:4-12}` → `/app/src/x.php`.
fn closure_file(name: &str) -> Option<&str> {
    let inner = name.strip_prefix("{closure:")?.strip_suffix('}')?;
    inner.rsplit_once(':').map(|(file, _lines)| file)
}

/// `App\Domain\Money->add` → `Money->add`.
pub fn without_namespace(function: &str) -> &str {
    let owner_end = function.find("->").or_else(|| function.find("::")).unwrap_or(function.len());
    function[..owner_end].rfind('\\').map_or(function, |at| &function[at + 1..])
}

/// `App\Domain\Money->add` and `App\Domain\Money::zero` → `App\Domain\Money`.
pub fn class_of(function: &str) -> Option<&str> {
    function.split_once("->").or_else(|| function.split_once("::")).map(|(class, _)| class)
}

/// A string argument that reads as a class name: `'App\\Domain\\Money'` → `App\Domain\Money`.
fn class_name(arg: &str) -> Option<String> {
    let inner = arg.strip_prefix('\'')?.strip_suffix('\'')?.replace("\\\\", "\\");
    let name = inner.trim_start_matches('\\');
    let valid = name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '\\')
        && name.starts_with(|c: char| c.is_alphabetic() || c == '_');
    valid.then(|| name.to_owned())
}

/// A trace file in a directory, newest first in [`list`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceFile {
    pub path: PathBuf,
    pub modified: SystemTime,
    pub len: u64,
}

/// The trace files in `dir`, newest first: `*.xt`, and `*.xt.gz`, as Xdebug writes them
/// when it was built with zlib (`xdebug.use_compression` is then on by default).
pub fn list(dir: &Path) -> std::io::Result<Vec<TraceFile>> {
    let mut files: Vec<TraceFile> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.ends_with(".xt") || name.ends_with(".xt.gz")
        })
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            meta.is_file().then(|| TraceFile {
                path: entry.path(),
                modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                len: meta.len(),
            })
        })
        .collect();
    files.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| b.path.cmp(&a.path)));
    Ok(files)
}

/// The text of a trace file, unpacked if it is gzipped, refused beyond `limit` bytes of
/// text. Of a gzipped trace still being written, what unpacks so far: it has no
/// `TRACE END` yet, so [`Trace::parse`] takes it as incomplete.
pub fn read(path: &Path, limit: u64) -> std::io::Result<String> {
    use std::io::{Error, ErrorKind, Read};
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    if path.extension().is_some_and(|e| e == "gz") {
        let read = flate2::read::MultiGzDecoder::new(file).take(limit + 1).read_to_end(&mut bytes);
        match read {
            Err(err) if err.kind() == ErrorKind::UnexpectedEof => {}
            other => drop(other?),
        }
    } else {
        file.take(limit + 1).read_to_end(&mut bytes)?;
    }
    if bytes.len() as u64 > limit {
        return Err(Error::other(format!("over {} MB of text", limit >> 20)));
    }
    String::from_utf8(bytes).map_err(|e| Error::new(ErrorKind::InvalidData, e))
}

/// `path` as tuitor sees it, by the longest of `mappings` (remote → local) it starts with.
pub fn map_path(path: &str, mappings: &[(String, PathBuf)]) -> PathBuf {
    let best = mappings
        .iter()
        .filter(|(remote, _)| {
            let remote = remote.trim_end_matches('/');
            path == remote || path.strip_prefix(remote).is_some_and(|rest| rest.starts_with('/'))
        })
        .max_by_key(|(remote, _)| remote.trim_end_matches('/').len());
    match best {
        Some((remote, local)) => local.join(path[remote.trim_end_matches('/').len()..].trim_start_matches('/')),
        None => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request of a small app, as Xdebug 3.5 writes it (paths shortened): index.php
    /// autoloads the controller, which builds a Money and calls a service. Money's
    /// constructor makes no calls, so only the autoloader tells where it is.
    const TRACE: &str = "Version: 3.5.3
File format: 4
TRACE START [2026-10-20 01:12:52.803481]
1\t0\t0\t0.000100\t494696\t{main}\t1\t\t/app/public/index.php\t0\t0
2\t1\t0\t0.000150\t497696\trequire\t1\t/app/vendor/autoload.php\t/app/public/index.php\t5\t0
3\t2\t0\t0.000155\t498080\tspl_autoload_register\t0\t\t/app/vendor/autoload.php\t4\t1\tclass Closure { }
3\t2\t1\t0.000163\t498528
2\t1\t1\t0.000165\t498488
2\t3\t0\t0.000167\t498328\t{closure:/app/vendor/autoload.php:4-12}\t1\t\t/app/public/index.php\t9\t1\t'App\\\\Controller\\\\Assign'
3\t4\t0\t0.000224\t502592\trequire\t1\t/app/src/Controller/Assign.php\t/app/vendor/autoload.php\t8\t0
3\t4\t1\t0.000228\t502592
2\t3\t1\t0.000229\t501888
2\t5\t0\t0.000235\t501736\tApp\\Controller\\Assign->__invoke\t1\t\t/app/public/index.php\t10\t2\t812\t'ada'
3\t6\t0\t0.000238\t501736\t{closure:/app/vendor/autoload.php:4-12}\t1\t\t/app/src/Controller/Assign.php\t15\t1\t'App\\\\Domain\\\\Money'
4\t7\t0\t0.000275\t504376\trequire\t1\t/app/src/Domain/Money.php\t/app/vendor/autoload.php\t8\t0
4\t7\t1\t0.000278\t504376
3\t6\t1\t0.000282\t503736
3\t8\t0\t0.000300\t503584\tApp\\Domain\\Money->__construct\t1\t\t/app/src/Controller/Assign.php\t15\t1\t8900
3\t8\t1\t0.000310\t503584
3\t9\t0\t0.000320\t503584\tsprintf\t0\t\t/app/src/Controller/Assign.php\t16\t2\t'%d'\t8900
3\t9\t1\t0.000330\t503584
3\t10\t0\t0.000340\t503584\tApp\\Domain\\Money->__construct\t1\t\t/app/src/Controller/Assign.php\t15\t1\t990
3\t10\t1\t0.000350\t503584
2\t5\t1\t0.000400\t501736
1\t0\t1\t0.000420\t526400
\t\t\t0.000448\t510592
TRACE END   [2026-10-20 01:12:52.804767]
";

    fn mapped() -> Trace {
        let mappings = [("/app".to_owned(), PathBuf::from("/home/me/shop"))];
        Trace::parse(TRACE, &|p| map_path(p, &mappings)).expect("parse")
    }

    fn file_of(trace: &Trace, bodies: &[Option<&Path>], name: &str) -> Option<String> {
        let index = trace.calls.iter().position(|c| c.name == name)?;
        bodies[index].map(|path| path.display().to_string())
    }

    #[test]
    fn reads_calls_with_their_parents_times_and_mapped_files() {
        let trace = mapped();
        assert!(trace.complete);
        assert_eq!(trace.started.as_deref(), Some("2026-10-20 01:12:52.803481"));
        assert_eq!(trace.calls.len(), 11);
        // Times count from the first call, to the last call's end.
        assert!((trace.end - 0.00032).abs() < 1e-9);
        let invoke = &trace.calls[5];
        assert_eq!(invoke.name, "App\\Controller\\Assign->__invoke");
        assert!((invoke.start - 0.000135).abs() < 1e-9 && invoke.end.is_some_and(|e| (e - 0.0003).abs() < 1e-9));
        assert_eq!(invoke.line, 10);
        assert_eq!(trace.file(invoke.file), Path::new("/home/me/shop/public/index.php"));
        assert_eq!(invoke.parent, Some(0));
        assert_eq!(trace.calls[6].parent, Some(5));
        assert!(!trace.calls[9].user, "sprintf is PHP's own");
        assert_eq!(trace.calls[3].first_arg.as_deref(), Some("'App\\\\Controller\\\\Assign'"));
    }

    #[test]
    fn a_trace_still_being_written_is_incomplete() {
        let cut = &TRACE[..TRACE.find("2\t5\t1").expect("exit line")];
        let trace = Trace::parse(cut, &|p| PathBuf::from(p)).expect("parse");
        assert!(!trace.complete);
        assert_eq!(trace.calls[5].end, None);
        assert!((trace.end_of(5) - 0.000250).abs() < 1e-9);
        assert!(Trace::parse("not a trace", &|p| PathBuf::from(p)).is_err());
    }

    #[test]
    fn finds_where_each_call_ran() {
        let trace = mapped();
        let none = HashMap::new();
        let bodies = trace.bodies(&none);
        let file = |name| file_of(&trace, &bodies, name);
        assert_eq!(file("{main}").as_deref(), Some("/home/me/shop/public/index.php"));
        assert_eq!(
            file("{closure:/app/vendor/autoload.php:4-12}").as_deref(),
            Some("/home/me/shop/vendor/autoload.php")
        );
        assert_eq!(
            file("App\\Controller\\Assign->__invoke").as_deref(),
            Some("/home/me/shop/src/Controller/Assign.php")
        );
        // It made no calls; the autoloader loaded its class from Money.php.
        assert_eq!(file("App\\Domain\\Money->__construct").as_deref(), Some("/home/me/shop/src/Domain/Money.php"));
        assert_eq!(file("sprintf"), None);
    }

    #[test]
    fn what_a_debugger_saw_names_files_the_trace_does_not() {
        let text = TRACE.replace("'App\\\\Domain\\\\Money'", "8900").replace(
            "4\t7\t0\t0.000275\t504376\trequire\t1\t/app/src/Domain/Money.php\t/app/vendor/autoload.php\t8\t0\n4\t7\t1\t0.000278\t504376\n",
            "",
        );
        let trace = Trace::parse(&text, &|p| PathBuf::from(p)).expect("parse");
        let none = HashMap::new();
        let unknown = trace.bodies(&none);
        assert_eq!(file_of(&trace, &unknown, "App\\Domain\\Money->__construct"), None);
        // Known by its name without the namespace, as an outline of Money.php has it.
        let known = HashMap::from([("Money->__construct".to_owned(), PathBuf::from("/w/Money.php"))]);
        let bodies = trace.bodies(&known);
        assert_eq!(file_of(&trace, &bodies, "App\\Domain\\Money->__construct").as_deref(), Some("/w/Money.php"));
    }

    #[test]
    fn locates_a_stack_and_the_time_of_its_line() {
        let trace = mapped();
        let index = Path::new("/home/me/shop/public/index.php");
        let assign = Path::new("/home/me/shop/src/Controller/Assign.php");
        let frame = |name, path, line| StackFrame { name, path: Some(path), line };
        let stack = [frame("App\\Controller\\Assign->__invoke", assign, 16), frame("{main}", index, 10)];
        let position = trace.locate(&stack, 0).expect("found");
        assert_eq!(position.path, [0, 5]);
        let ms = |p: Option<Position>| p.map(|p| (p.time * 1e6).round() / 1e3);
        // Line 16 calls sprintf: that call is about to start.
        assert_eq!(ms(Some(position)), Some(0.22));
        // On a line that made no call, after the last call made before it.
        let later = [frame("App\\Controller\\Assign->__invoke", assign, 18), frame("{main}", index, 10)];
        assert_eq!(ms(trace.locate(&later, 0)), Some(0.25));
        // The second time round line 15, of three.
        let again = [frame("App\\Controller\\Assign->__invoke", assign, 15), frame("{main}", index, 10)];
        assert_eq!(ms(trace.locate(&again, 1)), Some(0.2));
        assert_eq!(trace.locate(&again, 0).map(|p| (p.pass, p.candidates)), Some((0, 3)));
        assert_eq!(ms(trace.locate(&again, 0)), Some(0.138));
        // Called from another line: not this request's path.
        let elsewhere = [frame("App\\Controller\\Assign->__invoke", assign, 16), frame("{main}", index, 11)];
        assert_eq!(trace.locate(&elsewhere, 0), None);
    }

    #[test]
    fn stops_in_a_debugger_are_left_out_of_the_times() {
        let text = "File format: 4
TRACE START [2026-10-20 01:12:52.803481]
1\t0\t0\t0.001\t1\t{main}\t1\t\t/a.php\t0\t0
2\t1\t0\t0.002\t1\tusleep\t0\t\t/a.php\t3\t1\t500000
2\t1\t1\t0.502\t1
2\t2\t0\t2.502\t1\tf\t1\t\t/a.php\t4\t0
2\t2\t1\t2.503\t1
1\t0\t1\t2.504\t1
\t\t\t2.505\t1
TRACE END   [2026-10-20 01:12:55.000000]
";
        let trace = Trace::parse(text, &|p| PathBuf::from(p)).expect("parse");
        // Half a second in usleep is the program's; two seconds before f() was a stop.
        assert_eq!((trace.pauses, (trace.paused * 1000.0).round()), (1, 2000.0));
        assert!(trace.calls[1].end.is_some_and(|end| (end - 0.501).abs() < 1e-9));
        assert!((trace.calls[2].start - 0.501).abs() < 1e-9);
        assert!((trace.end - 0.503).abs() < 1e-9);
    }

    #[test]
    fn maps_by_the_longest_matching_prefix() {
        let mappings =
            [("/app".to_owned(), PathBuf::from("/w/api")), ("/app/vendor/".to_owned(), PathBuf::from("/w/vendor"))];
        assert_eq!(map_path("/app/src/A.php", &mappings), Path::new("/w/api/src/A.php"));
        assert_eq!(map_path("/app/vendor/x/B.php", &mappings), Path::new("/w/vendor/x/B.php"));
        assert_eq!(map_path("/application/C.php", &mappings), Path::new("/application/C.php"));
        assert_eq!(map_path("/tmp/x.php", &[]), Path::new("/tmp/x.php"));
    }

    #[test]
    fn lists_traces_newest_first() {
        let dir = std::env::temp_dir().join(format!("tuitor-traces-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        for name in ["a.xt", "b.xt.gz", "c.xt", "notes.txt", "d.gz"] {
            std::fs::write(dir.join(name), "x").expect("write");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let names: Vec<String> = list(&dir)
            .expect("list")
            .into_iter()
            .map(|f| f.path.file_name().unwrap_or_default().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["c.xt", "b.xt.gz", "a.xt"]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn reads_gzipped_traces_and_takes_a_cut_off_one_as_incomplete() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("tuitor-gz-traces-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(TRACE.as_bytes()).expect("gzip");
        let packed = gz.finish().expect("gzip");
        let whole = dir.join("whole.xt.gz");
        std::fs::write(&whole, &packed).expect("write");
        assert_eq!(read(&whole, 1 << 20).expect("read"), TRACE);
        assert!(read(&whole, 100).is_err(), "longer than the limit");

        // As Xdebug leaves it while the request runs: what it flushed, without the end of
        // the stream.
        let so_far = &TRACE[..TRACE.find("TRACE END").expect("end")];
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(so_far.as_bytes()).expect("gzip");
        gz.flush().expect("flush");
        let running = dir.join("running.xt.gz");
        std::fs::write(&running, gz.get_ref()).expect("write");
        let text = read(&running, 1 << 20).expect("read");
        assert_eq!(text, so_far);
        assert!(!Trace::parse(&text, &|p| PathBuf::from(p)).expect("parse").complete);

        let plain = dir.join("plain.xt");
        std::fs::write(&plain, TRACE).expect("write");
        assert_eq!(read(&plain, 1 << 20).expect("read"), TRACE);
        std::fs::remove_dir_all(dir).ok();
    }
}
