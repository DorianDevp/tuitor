//! `tuitor session <name> <verb>`: a peer of a shared session on the command line, for a
//! script or an agent. Each call reads the journal, prints plain text and ends; `join` and
//! `leave` keep the peer in the session in between, so that its vote is waited for.

use std::collections::{BTreeSet, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use clap::Subcommand;
use serde_json::Value;
use tuitor_core::compare::{self, Delta, Recording};
use tuitor_core::config::{Adapters, home_relative};
use tuitor_core::journal::{At, Body, Call, Journal, Peers, Proposals, Record, State, session_dir};
use tuitor_core::palette;
use tuitor_core::project::Project;
use tuitor_core::session::{Session, SessionId, SessionState};
use tuitor_core::snapshot::{Frame, Snapshot, Var};
use tuitor_core::store::Store;
use tuitor_core::{Engine, EngineOptions};

use crate::flow::FlowState;

#[derive(Subcommand)]
pub enum Verb {
    /// Be in the session until `leave`: calls that move the program on wait for this peer's vote.
    Join {
        /// This peer's calls go at once, without waiting for the others' votes.
        #[arg(long)]
        su: bool,
    },
    Leave,
    /// The stop: where, the stack with the code around each frame, variables, watches, votes, notes.
    Show {
        #[arg(long)]
        session: Option<SessionId>,
        #[arg(long)]
        stop: Option<u32>,
        /// The frame whose variables to list; read first if it was not.
        #[arg(long, default_value_t = 0)]
        frame: usize,
        /// Lines of code shown above and below each frame's line.
        #[arg(long, default_value_t = 6)]
        context: usize,
        /// Frames shown with their code; the others take a line each.
        #[arg(long, default_value_t = 3)]
        frames: usize,
        /// How deep nested variables are listed.
        #[arg(long, default_value_t = 3)]
        depth: usize,
    },
    /// Waits until the program stops, a call waits for votes, or a session ends; then `show`.
    Wait {
        /// stop, proposal or exit.
        what: String,
        #[arg(long, default_value_t = 600)]
        timeout: u64,
    },
    /// Runs a command: continue, next, step in, step out, pause, stop, restart, start ‹config›,
    /// eval ‹expr›, expand ‹Scope.var.field›, frame ‹n›, bp ‹file:line›, bp delete ‹file:line›,
    /// bp clear, set new, set active ‹set›, watch ‹expr›, watch delete ‹expr›.
    Do {
        #[arg(long)]
        session: Option<SessionId>,
        #[arg(long, default_value_t = 0)]
        frame: usize,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        words: Vec<String>,
    },
    /// Yes or no to the call that waits for this peer's vote.
    Vote {
        answer: String,
    },
    /// A note every peer sees; `--at stop` or `--at stop/frame` of the current session.
    Note {
        #[arg(required = true, trailing_var_arg = true)]
        words: Vec<String>,
        #[arg(long)]
        at: Option<String>,
    },
    /// The stops of a session.
    History {
        #[arg(long)]
        session: Option<SessionId>,
    },
    /// The call trees through a line, from the language server.
    Paths {
        location: String,
        #[arg(long, default_value_t = 120)]
        timeout: u64,
    },
    /// Two runs of stops lined up by breakpoint hit, value by value: `‹session›:‹from›-‹to›` each,
    /// optionally only the stops of one set.
    Compare {
        a: String,
        b: String,
        #[arg(long)]
        set: Option<String>,
    },
}

pub struct Context {
    pub name: String,
    pub peer: String,
    pub workspace: PathBuf,
    pub adapters: Adapters,
    pub store: Store,
    pub launch_json: Option<PathBuf>,
    pub servers: tuitor_code::Servers,
}

struct Replayed {
    journal: Journal,
    records: Vec<Record>,
    engine: Engine,
    monitor: bool,
}

impl Context {
    fn replay(&self) -> Result<Replayed> {
        let journal = Journal::open(&session_dir(&self.name)?)?;
        let records = journal.read_all()?;
        let monitor = journal.monitor_alive();
        let options =
            EngineOptions { adapters: self.adapters.clone(), workspace: self.workspace.clone(), state_file: None };
        let (mut engine, _) = Engine::new(options);
        engine.replay(&records, !monitor);
        Ok(Replayed { journal, records, engine, monitor })
    }

    fn append(&self, journal: &Journal, body: Body) -> Result<()> {
        journal.append(&Record::new(&self.peer, body))
    }
}

pub async fn run(context: Context, verb: Verb) -> Result<ExitCode> {
    match verb {
        Verb::Join { su } => {
            let replayed = context.replay()?;
            context.append(&replayed.journal, Body::Join { pid: None })?;
            if su {
                context.append(&replayed.journal, Body::Su)?;
            }
            crate::shared::ensure_monitor(&replayed.journal, &context.name, &context.workspace);
            println!("{} is in {}{}", context.peer, context.name, if su { " as su" } else { "" });
        }
        Verb::Leave => {
            let replayed = context.replay()?;
            context.append(&replayed.journal, Body::Leave)?;
            println!("{} left {}", context.peer, context.name);
        }
        Verb::Show { session, stop, frame, context: lines, frames, depth } => {
            let mut replayed = context.replay()?;
            let sid = pick_session(&replayed.engine, session)?;
            if let Some(sid) = sid {
                read_frame(&context, &mut replayed, sid, stop, frame).await?;
            }
            let view = View { lines, frames, depth, frame };
            print!("{}", show(&context, &replayed, sid, stop, &view));
        }
        Verb::Wait { what, timeout } => return wait(&context, &what, Duration::from_secs(timeout)).await,
        Verb::Do { session, frame, words } => return run_do(&context, session, frame, &words.join(" ")).await,
        Verb::Vote { answer } => {
            let yes = match answer.as_str() {
                "yes" | "y" => true,
                "no" | "n" => false,
                other => bail!("vote yes or no, not `{other}`"),
            };
            let replayed = context.replay()?;
            let proposals = proposals(&replayed.records);
            let proposal = proposals
                .open()
                .iter()
                .rev()
                .find(|p| p.by != context.peer && !p.yes.contains(&context.peer))
                .context("nothing waits for your vote")?;
            context.append(&replayed.journal, Body::Vote { call: proposal.id.clone(), yes })?;
            println!("{} {}", if yes { "yes to" } else { "no to" }, proposal.call.describe());
        }
        Verb::Note { words, at } => {
            let replayed = context.replay()?;
            let at = match at {
                Some(at) => Some(parse_at(&replayed.engine, &at)?),
                None => None,
            };
            context.append(&replayed.journal, Body::Note { text: words.join(" "), at })?;
            println!("noted");
        }
        Verb::History { session } => {
            let replayed = context.replay()?;
            let sid = pick_session(&replayed.engine, session)?.context("no session has stopped yet")?;
            print!("{}", history(&context, &replayed.engine, sid));
        }
        Verb::Paths { location, timeout } => return paths(&context, &location, Duration::from_secs(timeout)).await,
        Verb::Compare { a, b, set } => {
            let replayed = context.replay()?;
            print!("{}", compare_runs(&context, &replayed.engine, &a, &b, set.as_deref())?);
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// The session asked for, else the one that stopped last, else the last with stops.
fn pick_session(engine: &Engine, asked: Option<SessionId>) -> Result<Option<SessionId>> {
    if let Some(sid) = asked {
        return engine.session(sid).map(|s| Some(s.id)).with_context(|| format!("no session #{sid}"));
    }
    let latest = |s: &&Session| s.latest_stop().map(|stop| stop.time);
    let stopped = engine.sessions().iter().filter(|s| s.is_stopped()).max_by_key(latest);
    let any = engine.sessions().iter().filter(|s| !s.history.is_empty()).max_by_key(latest);
    Ok(stopped.or(any).or(engine.sessions().last()).map(|s| s.id))
}

fn proposals(records: &[Record]) -> Proposals {
    let mut proposals = Proposals::default();
    for record in records {
        proposals.see(record);
    }
    proposals
}

fn peers(records: &[Record]) -> Peers {
    let mut peers = Peers::default();
    for record in records {
        peers.see(record);
    }
    peers
}

/// Has the monitor read frame `frame` of the live stop, if it was not, and waits a little for it.
async fn read_frame(
    context: &Context,
    replayed: &mut Replayed,
    sid: SessionId,
    stop: Option<u32>,
    frame: usize,
) -> Result<()> {
    let Some(session) = replayed.engine.session(sid) else { return Ok(()) };
    let Some(live) = session.live() else { return Ok(()) };
    if stop.is_some_and(|n| n != live.number) || live.frames.get(frame).is_none_or(|f| f.scopes.is_some()) {
        return Ok(());
    }
    let call = Call::ReadFrame { session: sid, stop: live.number, frame };
    let id = format!("{}-{}-frame", context.peer, std::process::id());
    context.append(&replayed.journal, Body::Call { id, call })?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
        *replayed = context.replay()?;
        let read = replayed
            .engine
            .session(sid)
            .and_then(|s| s.live())
            .and_then(|s| s.frames.get(frame))
            .is_some_and(|f| f.scopes.is_some());
        if read {
            break;
        }
    }
    Ok(())
}

struct View {
    lines: usize,
    frames: usize,
    depth: usize,
    frame: usize,
}

fn show(context: &Context, replayed: &Replayed, sid: Option<SessionId>, stop: Option<u32>, view: &View) -> String {
    let mut out = String::new();
    let engine = &replayed.engine;
    let attached: Vec<String> = peers(&replayed.records).attached().into_iter().collect();
    let monitor = if replayed.monitor { "monitor running" } else { "no monitor" };
    let _ = writeln!(out, "shared session {} · peers: {} · {monitor}", context.name, list(&attached));
    let sessions: Vec<String> = engine
        .sessions()
        .iter()
        .filter(|s| s.is_alive())
        .map(|s| format!("#{} {} {}", s.id, s.config.name, state_name(s.state)))
        .collect();
    let _ =
        writeln!(out, "sessions: {}", if sessions.is_empty() { "none running".into() } else { sessions.join(" · ") });
    let attached: BTreeSet<String> = attached.into_iter().collect();
    for proposal in proposals(&replayed.records).open() {
        let waiting = proposal.waiting_for(&attached);
        let _ = writeln!(
            out,
            "vote: {} proposes {} · waiting for {}",
            proposal.by,
            proposal.call.describe(),
            list(&waiting)
        );
    }
    let Some(session) = sid.and_then(|sid| engine.session(sid)) else {
        out.push_str("no stop yet\n");
        return out;
    };
    let snapshot = match stop {
        Some(n) => session.snapshot(n),
        None => session.latest_stop().or(session.latest()),
    };
    let Some(snapshot) = snapshot else {
        let _ = writeln!(out, "#{} {} {}: no stop yet", session.id, session.config.name, state_name(session.state));
        return out;
    };
    let live = if session.live().is_some_and(|l| l.number == snapshot.number) { "live" } else { "history" };
    let set = snapshot.sets.first().map(|s| palette::name(*s, engine.breakpoints().sets())).unwrap_or_default();
    let _ = writeln!(
        out,
        "\n#{} {} · stop {} of {} ({live}) · {}{} · {}{}",
        session.id,
        session.config.name,
        snapshot.number,
        session.history.len(),
        snapshot.reason,
        snapshot.description.as_deref().map(|d| format!(": {d}")).unwrap_or_default(),
        snapshot.time.format("%H:%M:%S%.3f"),
        if set.is_empty() { String::new() } else { format!(" · set {set}") },
    );
    if let Some(top) = snapshot.frames.first() {
        let _ = writeln!(out, "at {}  {}", where_is(context, top), top.name);
    }
    out.push_str("\nstack:\n");
    let mut with_code = 0;
    for (i, frame) in snapshot.frames.iter().enumerate() {
        let library = if frame.library { "  (library)" } else { "" };
        let _ = writeln!(out, "  {i:>2}  {}  {}{library}", frame.name, where_is(context, frame));
        if !frame.library && with_code < view.frames {
            with_code += 1;
            out.push_str(&code_around(frame, view.lines));
        }
    }
    let _ = writeln!(out, "\nvariables (frame {}):", view.frame);
    match snapshot.frames.get(view.frame).and_then(|f| f.scopes.as_ref()) {
        None => out.push_str("  not read: the stop is history, or the frame was never picked while live\n"),
        Some(scopes) => {
            for scope in scopes {
                match &scope.vars {
                    None => {
                        let _ = writeln!(out, "  {} (not read; expand it: do expand {})", scope.name, scope.name);
                    }
                    Some(vars) => {
                        let _ = writeln!(out, "  {}", scope.name);
                        vars_tree(&mut out, vars, 2, view.depth, &scope.name);
                    }
                }
            }
        }
    }
    if !snapshot.watches.is_empty() {
        out.push_str("\nwatches:\n");
        for watch in &snapshot.watches {
            let value = watch.value.clone().unwrap_or_else(|e| format!("({e})"));
            let _ = writeln!(out, "  {} = {}", watch.expr, short(&value));
        }
    }
    let notes: Vec<&Record> = replayed.records.iter().filter(|r| matches!(r.body, Body::Note { .. })).collect();
    if !notes.is_empty() {
        out.push_str("\nnotes:\n");
        for record in notes.iter().rev().take(8).rev() {
            if let Body::Note { text, at } = &record.body {
                let at = at.map(|a| format!(" @#{}/{}/{}", a.session, a.stop, a.frame)).unwrap_or_default();
                let _ = writeln!(out, "  {} {}{at}: {text}", record.time.format("%H:%M"), record.peer);
            }
        }
    }
    out
}

fn list(names: &[String]) -> String {
    if names.is_empty() { "nobody".into() } else { names.join(", ") }
}

fn state_name(state: SessionState) -> &'static str {
    match state {
        SessionState::Starting => "starting",
        SessionState::Running => "running",
        SessionState::Stopped => "stopped",
        SessionState::Exited => "ended",
    }
}

fn where_is(context: &Context, frame: &Frame) -> String {
    match &frame.path {
        Some(path) => {
            let shown =
                path.strip_prefix(&context.workspace).map_or_else(|_| home_relative(path), |p| p.display().to_string());
            format!("{shown}:{}", frame.line)
        }
        None => frame.location(),
    }
}

fn code_around(frame: &Frame, lines: usize) -> String {
    let Some(text) = frame.path.as_deref().and_then(|p| std::fs::read_to_string(p).ok()) else { return String::new() };
    let all: Vec<&str> = text.lines().collect();
    let at = usize::try_from(frame.line).unwrap_or(0).max(1);
    let from = at.saturating_sub(lines).max(1);
    let to = (at + lines).min(all.len());
    let mut out = String::new();
    for n in from..=to {
        let mark = if n == at { "▶" } else { " " };
        let _ = writeln!(out, "       {mark}{n:>5} │ {}", all.get(n - 1).copied().unwrap_or_default());
    }
    out
}

fn vars_tree(out: &mut String, vars: &[Var], indent: usize, depth: usize, path: &str) {
    for var in vars {
        let kind = var.type_name.as_deref().map(|t| format!("  ({t})")).unwrap_or_default();
        let here = format!("{path}.{}", var.name);
        let more = match (&var.children, var.reference > 0) {
            (None, true) => format!("  [+ do expand {here}]"),
            _ => String::new(),
        };
        let _ = writeln!(out, "{}{} = {}{kind}{more}", "  ".repeat(indent), var.name, short(&var.value));
        if let Some(children) = &var.children
            && depth > 1
        {
            vars_tree(out, children, indent + 1, depth - 1, &here);
        }
    }
}

fn short(value: &str) -> String {
    let flat = value.replace('\n', "⏎");
    if flat.chars().count() > 200 { format!("{}…", flat.chars().take(200).collect::<String>()) } else { flat }
}

fn history(context: &Context, engine: &Engine, sid: SessionId) -> String {
    let mut out = String::new();
    let Some(session) = engine.session(sid) else { return out };
    let _ = writeln!(
        out,
        "#{} {} {} · {} stops",
        session.id,
        session.config.name,
        state_name(session.state),
        session.history.len()
    );
    for stop in &session.history {
        let set = stop.sets.first().map(|s| palette::name(*s, engine.breakpoints().sets())).unwrap_or_default();
        let silent = if stop.silent { "↷" } else { "●" };
        let at = stop.frames.first().map(|f| format!("{}  {}", where_is(context, f), f.name)).unwrap_or_default();
        let ran = stop.ran_ms.map(|ms| format!(" +{ms}ms")).unwrap_or_default();
        let _ = writeln!(
            out,
            "  {silent} {:>4}  {}{ran}  {:<10} {set:<8} {at}",
            stop.number,
            stop.time.format("%H:%M:%S%.3f"),
            stop.reason
        );
    }
    out
}

fn parse_at(engine: &Engine, text: &str) -> Result<At> {
    let sid = pick_session(engine, None)?.context("no session to point at")?;
    let (stop, frame) = text.split_once('/').unwrap_or((text, "0"));
    Ok(At {
        session: sid,
        stop: stop.parse().context("--at stop or stop/frame")?,
        frame: frame.parse().context("--at stop/frame")?,
    })
}

async fn wait(context: &Context, what: &str, timeout: Duration) -> Result<ExitCode> {
    if !matches!(what, "stop" | "proposal" | "exit") {
        bail!("wait for stop, proposal or exit, not `{what}`");
    }
    let replayed = context.replay()?;
    let mut known: HashSet<(SessionId, u32)> =
        replayed.engine.sessions().iter().flat_map(|s| s.history.iter().map(move |h| (s.id, h.number))).collect();
    let mut reader = replayed.journal.reader_at_end();
    let deadline = Instant::now() + timeout;
    let mut found: Option<SessionId> = None;
    while found.is_none() {
        if Instant::now() >= deadline {
            println!("nothing yet after {}s", timeout.as_secs());
            return Ok(ExitCode::FAILURE);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        for record in reader.read()? {
            match (&record.body, what) {
                (Body::State(State::Stop { session, snapshot }), "stop") => {
                    if known.insert((*session, snapshot.number)) && !snapshot.silent {
                        found = Some(*session);
                    }
                }
                (Body::Call { call, .. }, "proposal") if call.controls() && record.peer != context.peer => {
                    found = Some(session_of(call).unwrap_or_default());
                }
                (Body::State(State::Session(meta)), "exit") if meta.state == SessionState::Exited => {
                    found = Some(meta.id)
                }
                _ => {}
            }
        }
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let replayed = context.replay()?;
    let sid = found.filter(|sid| replayed.engine.session(*sid).is_some()).or(pick_session(&replayed.engine, None)?);
    let view = View { lines: 6, frames: 3, depth: 3, frame: 0 };
    print!("{}", show(context, &replayed, sid, None, &view));
    Ok(ExitCode::SUCCESS)
}

fn session_of(call: &Call) -> Option<SessionId> {
    match call {
        Call::Stop { session } | Call::Restart { session } | Call::Resume { session, .. } | Call::Pause { session } => {
            Some(*session)
        }
        _ => None,
    }
}

async fn run_do(context: &Context, session: Option<SessionId>, frame: usize, line: &str) -> Result<ExitCode> {
    let mut replayed = context.replay()?;
    let line = line.trim();
    let (word, rest) = line.split_once(' ').map_or((line, ""), |(w, r)| (w, r.trim()));
    let current = || -> Result<SessionId> { pick_session(&replayed.engine, session)?.context("no session") };
    let live_stop = |sid: SessionId| replayed.engine.session(sid).and_then(|s| s.live()).map(|s| s.number);
    let call = match (word, rest) {
        ("continue" | "c", "") => Some(Call::Resume { session: current()?, step: "continue".into() }),
        ("next" | "n", "") => Some(Call::Resume { session: current()?, step: "next".into() }),
        ("step", "in") | ("s", "") => Some(Call::Resume { session: current()?, step: "step in".into() }),
        ("step", "out") | ("o", "") => Some(Call::Resume { session: current()?, step: "step out".into() }),
        ("pause", "") => Some(Call::Pause { session: current()? }),
        ("stop", "") => Some(Call::Stop { session: current()? }),
        ("restart", "") => Some(Call::Restart { session: current()? }),
        ("start", name) if !name.is_empty() => Some(Call::Start { config: launch_config(context, name)? }),
        ("eval", expr) if !expr.is_empty() => {
            Some(Call::Evaluate { session: current()?, frame, expr: expr.to_owned() })
        }
        ("expand", path) if !path.is_empty() => {
            let sid = current()?;
            let stop = live_stop(sid).context("expand needs the live stop")?;
            Some(Call::Expand { session: sid, stop, frame, path: path.split('.').map(str::to_owned).collect() })
        }
        ("frame", n) => {
            let sid = current()?;
            let stop = live_stop(sid).context("a frame is read at the live stop")?;
            Some(Call::ReadFrame { session: sid, stop, frame: n.parse().context("frame ‹n›")? })
        }
        _ => None,
    };
    let Some(call) = call else {
        let said = breakpoints(&mut replayed.engine, line)?;
        context.append(&replayed.journal, Body::State(State::Breakpoints(replayed.engine.persisted())))?;
        println!("{said}");
        return Ok(ExitCode::SUCCESS);
    };
    if matches!(call, Call::Start { .. }) {
        crate::shared::ensure_monitor(&replayed.journal, &context.name, &context.workspace);
    }
    let id = format!("{}-{}-do", context.peer, std::process::id());
    let controls = call.controls();
    let evaluates = matches!(call, Call::Evaluate { .. });
    let mut reader = replayed.journal.reader_at_end();
    context.append(&replayed.journal, Body::Call { id: id.clone(), call: call.clone() })?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut replied: Option<(bool, String)> = None;
    let mut repl: Option<String> = None;
    while Instant::now() < deadline && (replied.is_none() || (evaluates && repl.is_none())) {
        tokio::time::sleep(Duration::from_millis(100)).await;
        for record in reader.read()? {
            match record.body {
                Body::Reply { call, ok, text } if call == id => replied = Some((ok, text)),
                Body::State(State::Output(line)) if evaluates && line.kind == tuitor_core::log::OutputKind::Repl => {
                    repl = Some(line.text);
                }
                _ => {}
            }
        }
    }
    match replied {
        Some((true, text)) => println!("{text}"),
        Some((false, text)) => {
            println!("refused: {text}");
            return Ok(ExitCode::FAILURE);
        }
        None if controls => {
            let attached = peers(&context.replay()?.records).attached();
            let waiting: Vec<String> = attached.into_iter().filter(|p| p != &context.peer).collect();
            println!("{} waits for the vote of {}", call.describe(), list(&waiting));
        }
        None => println!("no answer from the monitor yet"),
    }
    if let Some(text) = repl {
        println!("{text}");
    }
    Ok(ExitCode::SUCCESS)
}

fn launch_config(context: &Context, name: &str) -> Result<Value> {
    let project = Project::open(&context.workspace, &context.store, context.launch_json.as_deref());
    let config = project.configurations.iter().find(|c| c.name().eq_ignore_ascii_case(name)).with_context(|| {
        let names: Vec<&str> = project.configurations.iter().map(|c| c.name()).collect();
        format!("no configuration `{name}` (have: {})", names.join(", "))
    })?;
    Ok(Value::Object(config.launch(&project.variables())?.arguments))
}

/// Breakpoint, set and watch commands, on the replayed engine; says what changed.
fn breakpoints(engine: &mut Engine, line: &str) -> Result<String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let location = |text: &str| -> Result<(PathBuf, u32)> {
        let (file, line) = text.rsplit_once(':').context("file:line")?;
        let path = std::fs::canonicalize(file).with_context(|| format!("no file {file}"))?;
        Ok((path, line.parse().context("file:line")?))
    };
    Ok(match words.as_slice() {
        ["bp", "clear"] => {
            engine.clear_breakpoints();
            "breakpoints cleared".into()
        }
        ["bp", "delete", at] => {
            let (path, line) = location(at)?;
            let id = engine.breakpoints().at(&path, line).map(|b| b.id).context("no breakpoint there")?;
            engine.delete_breakpoint(id)?;
            format!("deleted #{id}")
        }
        ["bp", at, rest @ ..] | ["bp", "toggle", at, rest @ ..]
            if rest.first().is_none_or(|word| *word == "for" || *word == "if") =>
        {
            let (path, line) = location(at)?;
            let split = rest.iter().position(|word| *word == "if").unwrap_or(rest.len());
            let config = rest[..split].get(1..).map(|words| words.join(" ")).filter(|c| !c.is_empty());
            let condition = rest.get(split + 1..).map(|words| words.join(" ")).filter(|c| !c.is_empty());
            let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let toggle = engine.toggle_breakpoint_for(&path, line, config.clone());
            let said = crate::app::breakpoint_said(toggle, &file, line, config.as_deref());
            match (toggle, condition) {
                (tuitor_core::breakpoints::Toggle::Added(id), Some(condition)) => {
                    engine.update_breakpoint(id, |b| b.condition = Some(condition.clone()))?;
                    format!("{said}, if {condition}")
                }
                _ => said,
            }
        }
        ["set", "new"] => {
            let set = engine.new_set();
            format!("set {} is active", palette::name(set, engine.breakpoints().sets()))
        }
        ["set", "active", name] => {
            let set = crate::sets::parse_set(engine.breakpoints(), name).map_err(anyhow::Error::msg)?;
            engine.activate_set(set)?;
            format!("set {name} is active")
        }
        ["watch", "delete", rest @ ..] => {
            let expr = rest.join(" ");
            if engine.remove_watch(&expr) { format!("watch {expr} deleted") } else { format!("no watch {expr}") }
        }
        ["watch", rest @ ..] if !rest.is_empty() => {
            let expr = rest.join(" ");
            engine.add_watch(&expr);
            format!("watching {expr}")
        }
        _ => bail!("unknown command `{line}`; see tuitor session --help"),
    })
}

async fn paths(context: &Context, location: &str, timeout: Duration) -> Result<ExitCode> {
    let (file, line) = location.rsplit_once(':').context("paths ‹file:line›")?;
    let file = std::fs::canonicalize(file).with_context(|| format!("no file {file}"))?;
    let line: u32 = line.parse().context("paths ‹file:line›")?;
    let observed = FlowState::open(&context.workspace).observed();
    let mut code = tuitor_code::Code::new(context.workspace.clone(), context.servers.clone());
    let mut query = code.paths(file, line, observed).map_err(anyhow::Error::msg)?;
    let deadline = Instant::now() + timeout;
    let found = loop {
        if Instant::now() >= deadline {
            bail!("the language server said nothing in {}s", timeout.as_secs());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut done = None;
        for update in query.updates() {
            match update {
                tuitor_code::Update::Progress(text) => eprintln!("{text}"),
                tuitor_code::Update::Done(result) => done = Some(result),
            }
        }
        if let Some(result) = done {
            break result.map_err(anyhow::Error::msg)?;
        }
    };
    print!("{}", render_paths(&context.workspace, &found));
    code.shutdown().await;
    Ok(ExitCode::SUCCESS)
}

fn render_paths(workspace: &Path, found: &tuitor_code::Found) -> String {
    let shown =
        |path: &Path| path.strip_prefix(workspace).map_or_else(|_| home_relative(path), |p| p.display().to_string());
    let mut out = String::new();
    let target = found.target();
    let _ = writeln!(out, "paths through {}:{} in {}", shown(&target.path), found.line, target.name);
    for (i, stage) in found.stages.iter().enumerate() {
        let function = found.function(i);
        if stage.parent.is_none() {
            let root = match stage.root {
                Some(tuitor_code::Root::Entry(entry)) => format!("{entry:?}").to_lowercase(),
                Some(tuitor_code::Root::Uncalled) => "no caller known".into(),
                Some(tuitor_code::Root::Cut) => "cut".into(),
                None => String::new(),
            };
            let _ = writeln!(out, "\ntree ({root})");
        }
        let mark = if stage.anchor {
            "◆ "
        } else if stage.observed {
            "⇢ "
        } else {
            ""
        };
        let path = if found.on_path(i) { "" } else { "  (off the path)" };
        let sites = if stage.sites.is_empty() {
            String::new()
        } else {
            format!("  called at {}", stage.sites.iter().map(u32::to_string).collect::<Vec<_>>().join(", "))
        };
        let cut = stage.cut.map(|c| format!("  [{c:?}]").to_lowercase()).unwrap_or_default();
        let _ = writeln!(
            out,
            "{}{mark}{}  {}:{}{sites}{cut}{path}",
            "  ".repeat(stage.depth + 1),
            function.name,
            shown(&function.path),
            function.line
        );
    }
    out.push_str("\nlines worth a breakpoint, on the end-to-end paths:\n");
    let mut seen: HashSet<(PathBuf, u32)> = HashSet::new();
    for stage in (0..found.stages.len()).filter(|s| found.on_path(*s)) {
        for stop in found.lines(stage, |s| found.on_path(s)) {
            if seen.insert((stop.path.clone(), stop.line)) {
                let why = match &stop.why {
                    tuitor_code::Why::Target => "the line asked about".to_owned(),
                    tuitor_code::Why::Calls(name) => format!("calls {name}"),
                    tuitor_code::Why::Entry => "entry".to_owned(),
                    tuitor_code::Why::Writes(name) => format!("writes {name}"),
                    tuitor_code::Why::Uses(name) => format!("uses {name}"),
                };
                let _ = writeln!(out, "  {}:{}  {why}", shown(&stop.path), stop.line);
            }
        }
    }
    for note in &found.notes {
        let _ = writeln!(out, "note: {note}");
    }
    out
}

fn compare_runs(context: &Context, engine: &Engine, a: &str, b: &str, set: Option<&str>) -> Result<String> {
    let set = match set {
        Some(name) => Some(crate::sets::parse_set(engine.breakpoints(), name).map_err(anyhow::Error::msg)?),
        None => None,
    };
    let recording = |text: &str| -> Result<Recording<'_>> {
        let (sid, range) = text.split_once(':').unwrap_or((text, ""));
        let sid: SessionId = sid.trim_start_matches('#').parse().context("‹session›:‹from›-‹to›")?;
        let session = engine.session(sid).with_context(|| format!("no session #{sid}"))?;
        let (from, to) = match range.split_once('-') {
            Some((from, to)) => (from.parse().context("from")?, to.parse().context("to")?),
            None => (1, u32::try_from(session.history.len()).unwrap_or(u32::MAX)),
        };
        let stops: Vec<&Snapshot> = session
            .history
            .iter()
            .filter(|s| (from..=to).contains(&s.number))
            .filter(|s| set.is_none_or(|set| s.sets.contains(&set)))
            .collect();
        let start = stops.first().map(|s| s.time).context("no stops in that range")?;
        Ok(Recording { stops, start })
    };
    let (ra, rb) = (recording(a)?, recording(b)?);
    let mut out = String::new();
    let _ = writeln!(out, "compare {a} with {b}");
    for entry in compare::align(&ra, &rb) {
        let at = entry.a.or(entry.b).and_then(|s| s.frames.first()).map(|f| where_is(context, f)).unwrap_or_default();
        match (entry.a, entry.b) {
            (Some(x), Some(y)) => {
                let deltas: Vec<(String, Delta)> =
                    compare::differences(x, y).into_iter().filter(|(_, d)| d.differs()).collect();
                let _ = writeln!(
                    out,
                    "\n#{} hit {} · {at} · stops {} / {} · {} differ",
                    entry.breakpoint,
                    entry.occurrence,
                    x.number,
                    y.number,
                    deltas.len()
                );
                for (name, delta) in deltas {
                    let text = match delta {
                        Delta::Differs(was, now) => format!("{} → {}", short(&was), short(&now)),
                        Delta::Appears(now) => format!("only second: {}", short(&now)),
                        Delta::Disappears(was) => format!("only first: {}", short(&was)),
                        Delta::Same(_) => continue,
                    };
                    let _ = writeln!(out, "  {name}: {text}");
                }
            }
            (Some(x), None) => {
                let _ = writeln!(
                    out,
                    "\n#{} hit {} · {at} · only first (stop {})",
                    entry.breakpoint, entry.occurrence, x.number
                );
            }
            (None, Some(y)) => {
                let _ = writeln!(
                    out,
                    "\n#{} hit {} · {at} · only second (stop {})",
                    entry.breakpoint, entry.occurrence, y.number
                );
            }
            (None, None) => {}
        }
    }
    Ok(out)
}
