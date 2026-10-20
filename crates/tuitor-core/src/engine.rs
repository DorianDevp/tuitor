//! The engine owns all debugger state. It is driven from one loop: the UI calls
//! its methods, and results of asynchronous DAP traffic come back as
//! [`EngineMsg`]s that the loop passes to [`Engine::handle`]. Nothing is shared
//! across threads, so there are no locks.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use chrono::Local;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tuitor_dap::protocol::{RequestMessage, ResponseMessage};
use tuitor_dap::requests::{
    Attach, ConfigurationDone, Continue, Disconnect, Initialize, Launch, Next, Pause, Request, SetBreakpoints,
    SetExceptionBreakpoints, SourceContent, StepIn, StepOut, Terminate, Threads, Variables,
};
use tuitor_dap::types::{
    self as dap, Capabilities, DisconnectArguments, Empty, InitializeArguments, SetBreakpointsArguments,
    SetExceptionBreakpointsArguments, Source, SourceArguments, ThreadArguments, ThreadsResponse, VariablesArguments,
};
use tuitor_dap::{AdapterCommand, Client, Event, Incoming, Transport};

use crate::breakpoints::{Breakpoint, BreakpointId, Breakpoints, Toggle};
use crate::capture::{self, Capture, canonical, short_error};
use crate::config::{AdapterSpec, Adapters, INSTALL_HELP, LaunchConfig, Missing, RequestKind, TransportKind};
use crate::install::Package;
use crate::log::{Bounded, EventLine, OutputKind, OutputLine};
use crate::persist::{self, Persisted};
use crate::session::{ExceptionFilter, PendingStop, Session, SessionId, SessionState, SourceText};
use crate::snapshot::{ScopeVars, Snapshot, Var, VarPath, WatchValue};
use crate::tmpdir::TempDir;

const OUTPUT_LINES: usize = 10_000;
const EVENT_LINES: usize = 1_000;
const KILL_GRACE: Duration = Duration::from_secs(2);
/// Why tuitor turns down `runInTerminal`: its terminal is its own screen.
const NO_TERMINAL: &str = "the program is to run in a terminal, which tuitor cannot give it; \
                           set \"console\": \"internalConsole\" and its output shows in OUTPUT";

pub struct EngineOptions {
    pub adapters: Adapters,
    pub workspace: PathBuf,
    /// Where breakpoints and watches are remembered; `None` keeps them in memory only.
    pub state_file: Option<PathBuf>,
}

/// Result of asynchronous work, to be passed back to [`Engine::handle`].
pub struct EngineMsg(Msg);

enum Msg {
    Connected {
        sid: SessionId,
        result: Result<(Client, mpsc::UnboundedReceiver<Incoming>), tuitor_dap::Error>,
    },
    Initialized {
        sid: SessionId,
        result: Result<Capabilities, tuitor_dap::Error>,
    },
    Incoming {
        sid: SessionId,
        incoming: Incoming,
    },
    BreakpointsSet {
        sid: SessionId,
        path: PathBuf,
        ids: Vec<BreakpointId>,
        result: DapResult<dap::SetBreakpointsResponse>,
    },
    Captured {
        sid: SessionId,
        epoch: u64,
        result: DapResult<Capture>,
    },
    /// The scopes of a frame other than the top one.
    FrameRead {
        sid: SessionId,
        epoch: u64,
        number: u32,
        frame: usize,
        result: DapResult<Vec<ScopeVars>>,
    },
    Children {
        sid: SessionId,
        number: u32,
        frame: usize,
        path: VarPath,
        result: DapResult<Vec<Var>>,
    },
    Watch {
        sid: SessionId,
        number: u32,
        expr: String,
        result: DapResult<String>,
    },
    Source {
        sid: SessionId,
        reference: i64,
        result: DapResult<dap::SourceResponse>,
    },
    Evaluated {
        sid: SessionId,
        expr: String,
        result: DapResult<String>,
    },
    Threads {
        sid: SessionId,
        result: DapResult<ThreadsResponse>,
    },
    Done {
        sid: SessionId,
        command: &'static str,
        result: DapResult<()>,
    },
}

type DapResult<T> = Result<T, tuitor_dap::Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Continue,
    Over,
    In,
    Out,
}

impl Step {
    pub fn name(self) -> &'static str {
        match self {
            Step::Continue => "continue",
            Step::Over => "next",
            Step::In => "step in",
            Step::Out => "step out",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

/// Something worth telling the user right now (shown on the status line).
#[derive(Debug, Clone)]
pub struct Notice {
    pub session: Option<SessionId>,
    pub level: Level,
    pub text: String,
}

pub struct Engine {
    tx: mpsc::UnboundedSender<EngineMsg>,
    adapters: Adapters,
    workspace: PathBuf,
    state_file: Option<PathBuf>,
    sessions: Vec<Session>,
    next_session: SessionId,
    breakpoints: Breakpoints,
    watches: Vec<String>,
    output: Bounded<OutputLine>,
    events: Bounded<EventLine>,
    notices: Vec<Notice>,
    /// Adapter processes being ended, whose temporary directories go once they have exited.
    releasing: Vec<JoinHandle<()>>,
}

impl Engine {
    pub fn new(options: EngineOptions) -> (Engine, mpsc::UnboundedReceiver<EngineMsg>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut notices = Vec::new();
        let saved = match options.state_file.as_deref().map(persist::load).transpose() {
            Ok(saved) => saved.unwrap_or_default(),
            Err(err) => {
                notices.push(Notice { session: None, level: Level::Warn, text: format!("{err:#}") });
                Persisted::default()
            }
        };
        let engine = Engine {
            tx,
            adapters: options.adapters,
            workspace: options.workspace,
            state_file: options.state_file,
            sessions: Vec::new(),
            next_session: 1,
            breakpoints: Breakpoints::from_saved(saved.breakpoints),
            watches: saved.watches,
            output: Bounded::new(OUTPUT_LINES),
            events: Bounded::new(EVENT_LINES),
            notices,
            releasing: Vec::new(),
        };
        (engine, rx)
    }

    // ---- queries ------------------------------------------------------------------

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub fn adapters(&self) -> &Adapters {
        &self.adapters
    }

    pub fn sessions(&self) -> &[Session] {
        &self.sessions
    }

    pub fn session(&self, sid: SessionId) -> Option<&Session> {
        self.sessions.iter().find(|s| s.id == sid)
    }

    /// The session that was started from a configuration, at the top of `sid`'s parents.
    pub fn root(&self, mut sid: SessionId) -> SessionId {
        while let Some(parent) = self.session(sid).and_then(|s| s.parent) {
            sid = parent;
        }
        sid
    }

    pub fn breakpoints(&self) -> &Breakpoints {
        &self.breakpoints
    }

    pub fn watches(&self) -> &[String] {
        &self.watches
    }

    pub fn output(&self) -> &Bounded<OutputLine> {
        &self.output
    }

    pub fn events(&self) -> &Bounded<EventLine> {
        &self.events
    }

    pub fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.notices)
    }

    // ---- sessions -------------------------------------------------------------------

    /// Starts the adapter for `config`; progress arrives through [`Engine::handle`].
    pub fn start(&mut self, config: LaunchConfig) -> Result<SessionId> {
        self.launch(config, None)
    }

    /// Starts a session for `config` in place of `replaces`, which gives up its name.
    fn launch(&mut self, config: LaunchConfig, replaces: Option<SessionId>) -> Result<SessionId> {
        let spec = self.adapters.get(&config.adapter).cloned().ok_or_else(|| {
            let known: Vec<&str> = self.adapters.names().collect();
            anyhow!("no adapter for type `{}` (known: {})", config.adapter, known.join(", "))
        })?;
        let (transport, tmp) = self.transport(&spec)?;

        let sid = self.next_session;
        self.next_session += 1;
        let tag = self.tag_for(&spec);
        let name = self.unique_name(&tag, replaces);
        self.log_event(sid, "starting", format!("{} · {}", config.name, describe(&transport)));
        let mut session = Session::new(sid, name, spec, config, tag);
        session.tmp = tmp;
        self.sessions.push(session);
        self.connect(sid, transport);
        Ok(sid)
    }

    fn connect(&self, sid: SessionId, transport: Transport) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = Client::connect(transport).await;
            let _ = tx.send(EngineMsg(Msg::Connected { sid, result }));
        });
    }

    /// Ends the session's debuggee and adapter. A child session runs part of the program
    /// its parent launched, so stopping it stops the parent, which ends the children.
    pub fn stop(&mut self, sid: SessionId) -> Result<()> {
        let sid = self.root(sid);
        let session = self.session_ref(sid)?;
        let client = session.client.clone().ok_or_else(|| anyhow!("{} is not connected", session.name))?;
        if session.capabilities.supports_terminate_request {
            self.call::<Terminate>(sid, &client, Empty {}, move |r| Msg::Done {
                sid,
                command: "terminate",
                result: r.map(drop),
            });
        } else {
            let args = DisconnectArguments { terminate_debuggee: true };
            self.call::<Disconnect>(sid, &client, args, move |r| Msg::Done {
                sid,
                command: "disconnect",
                result: r.map(drop),
            });
        }
        tokio::spawn(async move {
            tokio::time::sleep(KILL_GRACE).await;
            client.kill();
        });
        Ok(())
    }

    /// Starts the same configuration again in a new session; for a child session, its
    /// parent's, since the adapter made up the child's. The new session keeps the name: the
    /// old one is on its way out, and a `#2` would outlast it.
    pub fn restart(&mut self, sid: SessionId) -> Result<SessionId> {
        let sid = self.root(sid);
        let config = self.session_ref(sid)?.config.clone();
        if self.session_ref(sid)?.is_alive() {
            self.stop(sid)?;
        }
        self.launch(config, Some(sid))
    }

    /// Disconnects every session and ends the adapters, with their temporary directories;
    /// used when quitting and closing a tab.
    pub async fn shutdown(&mut self) {
        let clients: Vec<Client> = self.sessions.iter().filter_map(|s| s.client.clone()).collect();
        let goodbyes =
            clients.iter().filter_map(|c| c.call::<Disconnect>(DisconnectArguments { terminate_debuggee: true }).ok());
        let _ = tokio::time::timeout(Duration::from_millis(500), futures_join(goodbyes.collect())).await;
        let sids: Vec<SessionId> = self.sessions.iter().map(|s| s.id).collect();
        for sid in sids {
            self.release(sid);
        }
        futures_join(std::mem::take(&mut self.releasing)).await;
    }

    // ---- execution ------------------------------------------------------------------

    pub fn resume(&mut self, sid: SessionId, step: Step) -> Result<()> {
        let session = self.session_mut(sid)?;
        if !session.is_stopped() {
            bail!("{} is not stopped", session.name);
        }
        let thread_id = session.stopped_thread.ok_or_else(|| anyhow!("no stopped thread"))?;
        let client = session.client.clone().ok_or_else(|| anyhow!("{} is not connected", session.name))?;
        session.mark_running();

        let args = ThreadArguments { thread_id };
        let command = step.name();
        let done = move |result: DapResult<()>| Msg::Done { sid, command, result };
        match step {
            Step::Continue => self.call::<Continue>(sid, &client, args, move |r| done(r.map(drop))),
            Step::Over => self.call::<Next>(sid, &client, args, move |r| done(r.map(drop))),
            Step::In => self.call::<StepIn>(sid, &client, args, move |r| done(r.map(drop))),
            Step::Out => self.call::<StepOut>(sid, &client, args, move |r| done(r.map(drop))),
        }
        self.log_event(sid, command, format!("thread {thread_id}"));
        Ok(())
    }

    pub fn pause(&mut self, sid: SessionId) -> Result<()> {
        let running: Vec<SessionId> =
            self.children(sid).filter(|c| c.is_alive() && !c.is_stopped()).map(|c| c.id).collect();
        let session = self.session_ref(sid)?;
        // A parent that runs its targets in child sessions has no threads of its own.
        if session.threads.is_empty() && !running.is_empty() {
            return running.into_iter().try_for_each(|child| self.pause(child));
        }
        if session.is_stopped() {
            bail!("{} is already stopped", session.name);
        }
        let thread_id = session
            .stopped_thread
            .or_else(|| session.threads.first().map(|t| t.id))
            .ok_or_else(|| anyhow!("{} has no thread to pause", session.name))?;
        let client = session.client.clone().ok_or_else(|| anyhow!("{} is not connected", session.name))?;
        self.call::<Pause>(sid, &client, ThreadArguments { thread_id }, move |r| Msg::Done {
            sid,
            command: "pause",
            result: r.map(drop),
        });
        Ok(())
    }

    // ---- breakpoints ------------------------------------------------------------------

    pub fn toggle_breakpoint(&mut self, path: &Path, line: u32) -> Toggle {
        let path = canonical(path);
        let toggle = self.breakpoints.toggle(path.clone(), line);
        self.breakpoints_changed(&path);
        toggle
    }

    pub fn update_breakpoint(&mut self, id: BreakpointId, change: impl FnOnce(&mut Breakpoint)) -> Result<()> {
        let path = self
            .breakpoints
            .update(id, change)
            .map(|b| b.path.clone())
            .ok_or_else(|| anyhow!("no breakpoint #{id}"))?;
        self.breakpoints_changed(&path);
        Ok(())
    }

    pub fn delete_breakpoint(&mut self, id: BreakpointId) -> Result<()> {
        let removed = self.breakpoints.remove(id).ok_or_else(|| anyhow!("no breakpoint #{id}"))?;
        self.breakpoints_changed(&removed.path);
        Ok(())
    }

    pub fn clear_breakpoints(&mut self) {
        let files = self.breakpoints.files();
        self.breakpoints = Breakpoints::default();
        for file in files {
            self.breakpoints_changed(&file);
        }
    }

    pub fn toggle_exception_filter(&mut self, sid: SessionId, filter: &str) -> Result<bool> {
        let session = self.session_mut(sid)?;
        let entry = session
            .exception_filters
            .iter_mut()
            .find(|f| f.filter == filter)
            .ok_or_else(|| anyhow!("no exception filter `{filter}`"))?;
        entry.enabled = !entry.enabled;
        let enabled = entry.enabled;
        self.send_exception_filters(sid);
        Ok(enabled)
    }

    // ---- data -------------------------------------------------------------------------

    pub fn add_watch(&mut self, expr: &str) {
        let expr = expr.trim().to_owned();
        if expr.is_empty() || self.watches.contains(&expr) {
            return;
        }
        self.watches.push(expr.clone());
        self.save();
        // Evaluate right away in every session that is stopped, so the column is not empty.
        let live: Vec<(SessionId, u32, i64, Client)> = self
            .sessions
            .iter()
            .filter_map(|s| {
                let snap = s.live()?;
                Some((s.id, snap.number, snap.top()?.id, s.client.clone()?))
            })
            .collect();
        for (sid, number, frame_id, client) in live {
            let tx = self.tx.clone();
            let expr = expr.clone();
            tokio::spawn(async move {
                let result = capture::evaluate(&client, &expr, Some(frame_id), "watch").await;
                let _ = tx.send(EngineMsg(Msg::Watch { sid, number, expr, result }));
            });
        }
    }

    pub fn remove_watch(&mut self, expr: &str) -> bool {
        let before = self.watches.len();
        self.watches.retain(|w| w != expr);
        let removed = self.watches.len() != before;
        if removed {
            self.save();
        }
        removed
    }

    /// Evaluates in frame `frame` of the live stop (or globally while running).
    pub fn evaluate(&mut self, sid: SessionId, frame: usize, expr: &str) -> Result<()> {
        let session = self.session_ref(sid)?;
        let client = session.client.clone().ok_or_else(|| anyhow!("{} is not connected", session.name))?;
        let frame_id = session.live().and_then(|s| s.frames.get(frame)).map(|f| f.id);
        let tx = self.tx.clone();
        let expr = expr.to_owned();
        tokio::spawn(async move {
            let result = capture::evaluate(&client, &expr, frame_id, "repl").await;
            let _ = tx.send(EngineMsg(Msg::Evaluated { sid, expr, result }));
        });
        Ok(())
    }

    /// Expands a scope (`[scope]`) or variable path in frame `frame` of a snapshot; it stays
    /// expanded at later stops, and in other frames.
    pub fn expand(&mut self, sid: SessionId, number: u32, frame: usize, path: VarPath) -> Result<()> {
        let session = self.session_mut(sid)?;
        session.expanded.insert(path.clone());
        let snapshot = session.snapshot(number).ok_or_else(|| anyhow!("no stop #{number}"))?;
        let scopes = snapshot.scopes(frame).unwrap_or_default();
        match path.as_slice() {
            [] => return Ok(()),
            [scope] if !scopes.iter().any(|s| &s.name == scope) => bail!("no scope {scope}"),
            [_, _, ..] if snapshot.var(frame, &path).is_none() => bail!("no variable {}", path.join(".")),
            _ => {}
        }
        let history = !snapshot.live || !session.is_stopped();
        let not_captured = history && !snapshot.unloaded(frame, &path, &session.expanded).is_empty();
        // Rows below that stayed expanded while this one was collapsed are read too. Expansion is
        // shared by all stops and frames, so the live stop loads them in every frame it read,
        // even when `number` is history.
        self.load_expanded(sid, None, &path);
        if not_captured {
            bail!("stop #{number} is history; its children were not captured");
        }
        Ok(())
    }

    /// Reads the scopes of frame `frame` of stop `number`, picked in STACK, as the stop read
    /// those of the top frame. Only the live stop can still be read, and only once.
    pub fn read_frame(&mut self, sid: SessionId, number: u32, frame: usize) {
        let Ok(session) = self.session_ref(sid) else { return };
        let (Some(snapshot), Some(client)) = (session.live(), session.client.clone()) else { return };
        let Some(target) = snapshot.frames.get(frame) else { return };
        if snapshot.number != number || target.scopes.is_some() || session.reading.contains(&frame) {
            return;
        }
        let (epoch, frame_id) = (session.epoch, target.id);
        let expanded: Vec<VarPath> = session.expanded.iter().cloned().collect();
        let seen: Vec<String> = session.seen_scopes.iter().cloned().collect();
        if let Ok(session) = self.session_mut(sid) {
            session.reading.insert(frame);
        }
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = capture::scopes(&client, frame_id, &expanded, &seen).await;
            let _ = tx.send(EngineMsg(Msg::FrameRead { sid, epoch, number, frame, result }));
        });
    }

    /// Asks the adapter for the text of its source `reference`, unless it was asked already;
    /// [`Session::source`] has it once it is there.
    pub fn read_source(&mut self, sid: SessionId, reference: i64) {
        let Ok(session) = self.session_mut(sid) else { return };
        let Some(client) = session.client.clone() else { return };
        if session.sources.contains_key(&reference) {
            return;
        }
        session.sources.insert(reference, SourceText::Reading);
        let source = Source { source_reference: Some(reference), ..Source::default() };
        let args = SourceArguments { source, source_reference: reference };
        self.call::<SourceContent>(sid, &client, args, move |result| Msg::Source { sid, reference, result });
    }

    pub fn collapse(&mut self, sid: SessionId, path: &VarPath) {
        if let Ok(session) = self.session_mut(sid) {
            session.expanded.remove(path);
        }
    }

    pub fn clear_output(&mut self) {
        self.output.clear();
    }

    // ---- message handling ---------------------------------------------------------------

    pub fn handle(&mut self, msg: EngineMsg) {
        match msg.0 {
            Msg::Connected { sid, result } => self.on_connected(sid, result),
            Msg::Initialized { sid, result } => self.on_initialized(sid, result),
            Msg::Incoming { sid, incoming } => self.on_incoming(sid, incoming),
            Msg::BreakpointsSet { sid, path, ids, result } => {
                match result {
                    Ok(response) => self.breakpoints.bind(sid, &path, &ids, &response.breakpoints),
                    Err(err) => self.notice(
                        Some(sid),
                        Level::Warn,
                        format!("breakpoints in {}: {}", file_name(&path), short_error(&err)),
                    ),
                }
                self.configuration_answered(sid);
            }
            Msg::Captured { sid, epoch, result } => self.on_captured(sid, epoch, result),
            Msg::FrameRead { sid, epoch, number, frame, result } => {
                self.on_frame_read(sid, epoch, number, frame, result)
            }
            Msg::Children { sid, number, frame, path, result } => self.on_children(sid, number, frame, &path, result),
            Msg::Watch { sid, number, expr, result } => {
                if let Some(snapshot) = self.session_mut(sid).ok().and_then(|s| s.snapshot_mut(number)) {
                    let value = result.map_err(|e| short_error(&e));
                    snapshot.watches.retain(|w| w.expr != expr);
                    snapshot.watches.push(WatchValue { expr, value });
                }
            }
            Msg::Source { sid, reference, result } => {
                let text = match result {
                    Ok(response) => SourceText::Read(response.content),
                    Err(err) => SourceText::Failed(short_error(&err)),
                };
                if let Ok(session) = self.session_mut(sid) {
                    session.sources.insert(reference, text);
                }
            }
            Msg::Evaluated { sid, expr, result } => {
                let (level, text) = match result {
                    Ok(value) => (Level::Info, format!("{expr} = {value}")),
                    Err(err) => (Level::Warn, format!("{expr}: {}", short_error(&err))),
                };
                self.push_output(sid, OutputKind::Repl, &format!("{text}\n"));
                self.notice(Some(sid), level, text);
            }
            Msg::Threads { sid, result } => {
                if let (Ok(session), Ok(response)) = (self.session_mut(sid), result) {
                    session.threads = response.threads;
                }
                self.name_after_target(sid);
            }
            Msg::Done { sid, command, result } => {
                match result {
                    // Cut off by the end of the session, which said why it ended.
                    Err(tuitor_dap::Error::Closed) if !self.session_ref(sid).is_ok_and(Session::is_alive) => {}
                    Err(err) => self.notice(Some(sid), Level::Error, format!("{command}: {}", short_error(&err))),
                    // The session is over; js-debug then closes the connection without `terminated`.
                    Ok(()) if command == Disconnect::COMMAND => {
                        if let Ok(session) = self.session_mut(sid) {
                            session.mark_exited();
                        }
                    }
                    Ok(()) => {}
                }
                if command == SetExceptionBreakpoints::COMMAND {
                    self.configuration_answered(sid);
                }
            }
        }
    }

    fn on_connected(&mut self, sid: SessionId, result: DapResult<(Client, mpsc::UnboundedReceiver<Incoming>)>) {
        let (client, mut incoming) = match result {
            Ok(connection) => connection,
            Err(err) => return self.fail(sid, format!("adapter: {err}")),
        };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            while let Some(incoming) = incoming.recv().await {
                let closed = matches!(incoming, Incoming::Closed);
                if tx.send(EngineMsg(Msg::Incoming { sid, incoming })).is_err() || closed {
                    break;
                }
            }
        });
        let Ok(session) = self.session_mut(sid) else { return };
        let args = InitializeArguments::new(session.adapter.adapter_id());
        session.client = Some(client.clone());
        self.call::<Initialize>(sid, &client, args, move |result| Msg::Initialized { sid, result });
    }

    fn on_initialized(&mut self, sid: SessionId, result: DapResult<Capabilities>) {
        let capabilities = match result {
            Ok(caps) => caps,
            Err(err) => return self.fail(sid, format!("initialize: {}", short_error(&err))),
        };
        let Ok(session) = self.session_mut(sid) else { return };
        session.exception_filters = capabilities
            .exception_breakpoint_filters
            .iter()
            .map(|f| ExceptionFilter { filter: f.filter.clone(), label: f.label.clone(), enabled: f.default })
            .collect();
        session.capabilities = capabilities;
        let arguments = session.adapter.launch_arguments(&session.config);
        let (Some(client), request, args) = (session.client.clone(), session.config.request, Value::Object(arguments))
        else {
            return;
        };
        session.launch_sent = true;
        let configure_now = session.configure_pending;

        let done = move |result: DapResult<dap::Empty>| match result {
            Ok(_) => Msg::Done { sid, command: "launch", result: Ok(()) },
            Err(err) => Msg::Done { sid, command: "launch", result: Err(err) },
        };
        match request {
            RequestKind::Launch => self.call::<Launch>(sid, &client, args, done),
            RequestKind::Attach => self.call::<Attach>(sid, &client, args, done),
        }
        if configure_now {
            self.configure(sid);
        }
    }

    /// Sends breakpoints once the adapter says it is ready, and `configurationDone` once they
    /// are answered.
    fn configure(&mut self, sid: SessionId) {
        let Ok(session) = self.session_mut(sid) else { return };
        session.configured = true;
        session.configure_pending = false;
        if session.state == SessionState::Starting {
            session.state = SessionState::Running;
        }
        let Some(client) = session.client.clone() else { return };
        let spec = session.adapter.clone();

        let files: Vec<PathBuf> = self.breakpoints.files().into_iter().filter(|p| spec.handles(p)).collect();
        let mut unanswered = files.len();
        for path in files {
            self.send_breakpoints(sid, &client, path);
        }
        unanswered += usize::from(self.send_exception_filters(sid));
        let Ok(session) = self.session_mut(sid) else { return };
        session.configuration_unanswered = unanswered;
        if unanswered == 0 {
            self.send_configuration_done(sid);
        }
    }

    /// Counts an answer to a request sent by `configure`; after the last one the program may start.
    fn configuration_answered(&mut self, sid: SessionId) {
        let Ok(session) = self.session_mut(sid) else { return };
        if session.configuration_unanswered == 0 {
            return; // an answer to a later change
        }
        session.configuration_unanswered -= 1;
        if session.configuration_unanswered == 0 {
            self.send_configuration_done(sid);
        }
    }

    /// Lets the program start. Like VS Code, tuitor waits for the breakpoints to be answered
    /// first: gdb writes "Breakpoint 1 (…) pending." from another thread, and text that lagged
    /// behind this response would pass for the program's output.
    fn send_configuration_done(&mut self, sid: SessionId) {
        let Ok(session) = self.session_mut(sid) else { return };
        if !session.capabilities.supports_configuration_done_request {
            session.configuration_done = true; // nothing to wait for
            return;
        }
        let Some(client) = session.client.clone() else { return };
        // Its answer comes among the events, so that output sent before it is known to be the adapter's.
        if let Err(err) = client.call_in_stream::<ConfigurationDone>(Empty {}) {
            self.notice(Some(sid), Level::Error, format!("configurationDone: {}", short_error(&err)));
        }
    }

    fn on_incoming(&mut self, sid: SessionId, incoming: Incoming) {
        match incoming {
            Incoming::Event(event) => self.on_event(sid, event),
            Incoming::Request(request) => self.on_reverse_request(sid, &request),
            Incoming::Response(response) => self.on_response(sid, response),
            Incoming::Log(line) => self.push_output(sid, OutputKind::Adapter, &format!("{line}\n")),
            Incoming::Closed => {
                self.breakpoints.unbind_session(sid);
                self.release(sid);
                if let Ok(session) = self.session_mut(sid) {
                    let was_alive = session.is_alive();
                    session.mark_exited();
                    let name = session.name.clone();
                    if was_alive {
                        self.notice(Some(sid), Level::Warn, format!("{name}: adapter disconnected"));
                    }
                }
                self.end_children(sid);
                self.log_event(sid, "closed", String::new());
            }
        }
    }

    /// Requests from the adapter to tuitor. Each gets an answer, so that the adapter never
    /// waits for one that will not come.
    fn on_reverse_request(&mut self, sid: SessionId, request: &RequestMessage) {
        let Some(client) = self.session_ref(sid).ok().and_then(|s| s.client.clone()) else { return };
        let (level, refusal) = match request.command.as_str() {
            "startDebugging" => match self.start_child(sid, &request.arguments) {
                Ok(()) => {
                    let _ = client.respond(request, true, Value::Null);
                    return;
                }
                Err(err) => (Level::Error, format!("startDebugging: {err:#}")),
            },
            "runInTerminal" => (Level::Error, NO_TERMINAL.to_owned()),
            other => (Level::Warn, format!("tuitor does not support `{other}`")),
        };
        let _ = client.respond_error(request, &refusal);
        self.log_event(sid, "refused", request.command.clone());
        if request.command == "runInTerminal" {
            // The adapter cannot start the program without one, yet js-debug answers `launch`
            // with success and waits for good; the session ends here instead.
            return self.fail(self.root(sid), refusal);
        }
        self.notice(Some(sid), level, refusal);
    }

    /// Starts the session an adapter asks for with `startDebugging`, as a child of the one
    /// that asked: a new connection to the same adapter (js-debug serves every session on
    /// its port) that sends the adapter's configuration back and is configured like any other.
    fn start_child(&mut self, parent: SessionId, arguments: &Value) -> Result<()> {
        let Some(Value::Object(configuration)) = arguments.get("configuration") else {
            bail!("no configuration");
        };
        let request = match arguments.get("request").and_then(Value::as_str) {
            Some("launch") => RequestKind::Launch,
            Some("attach") => RequestKind::Attach,
            other => bail!("unknown request {other:?}"),
        };
        let parent_session = self.session_ref(parent)?;
        let spec = parent_session.adapter.clone();
        let (transport, tmp) = match parent_session.client.as_ref().and_then(Client::address) {
            Some(address) => (Transport::Tcp(address.to_owned()), None),
            // An adapter on stdio serves one session per process.
            None => self.transport(&spec)?,
        };
        // js-debug names the target: the script and its pid, or the page's title.
        let title = configuration.get("name").and_then(Value::as_str).unwrap_or("child").to_owned();
        let name = self.unique(format!("{} › {title}", parent_session.name), None);
        let detail = format!("{title} · child of {}", parent_session.name);
        let tag = parent_session.tag.clone();
        let config =
            LaunchConfig { name: title, adapter: spec.name.clone(), request, arguments: configuration.clone() };

        let sid = self.next_session;
        self.next_session += 1;
        self.log_event(sid, "starting", detail);
        let mut session = Session::new(sid, name, spec, config, tag);
        session.parent = Some(parent);
        session.tmp = tmp;
        self.sessions.push(session);
        // The code runs in the children, so breakpoints go to them from now on.
        self.breakpoints.unbind_session(parent);
        self.connect(sid, transport);
        Ok(())
    }

    /// Renames a child session after its target's current title. js-debug names the child
    /// when the target appears, which for a page is before it loads (`about:blank`), and
    /// names the target's one thread after its title from then on.
    fn name_after_target(&mut self, sid: SessionId) {
        let Ok(session) = self.session_ref(sid) else { return };
        let (Some(parent), [thread]) = (session.parent, session.threads.as_slice()) else { return };
        if thread.name.is_empty() || thread.name == session.config.name {
            return;
        }
        let title = thread.name.clone();
        let parent_name = self.session(parent).map(|p| p.name.clone()).unwrap_or_default();
        let name = self.unique(format!("{parent_name} › {title}"), None);
        if let Ok(session) = self.session_mut(sid) {
            session.name = name;
            session.config.name = title;
        }
    }

    /// Ends the child sessions of a parent that ended: they ran part of its program.
    fn end_children(&mut self, parent: SessionId) {
        let children: Vec<(SessionId, Client)> =
            self.children(parent).filter(|c| c.is_alive()).filter_map(|c| Some((c.id, c.client.clone()?))).collect();
        for (sid, client) in children {
            let args = DisconnectArguments { terminate_debuggee: true };
            // The adapter may be gone already; then there is nothing left to end.
            self.call::<Disconnect>(sid, &client, args, move |r| Msg::Done {
                sid,
                command: "disconnect",
                result: r.map(drop).or(Ok(())),
            });
        }
    }

    /// Answers to requests sent with `call_in_stream`, in their place among the events.
    fn on_response(&mut self, sid: SessionId, response: ResponseMessage) {
        if response.command == ConfigurationDone::COMMAND
            && let Ok(session) = self.session_mut(sid)
        {
            session.configuration_done = true;
        }
        if !response.success {
            let message = response.message.unwrap_or_default();
            self.notice(Some(sid), Level::Error, format!("{}: {message}", response.command));
        }
    }

    fn on_event(&mut self, sid: SessionId, event: Event) {
        match event {
            Event::Initialized => {
                let Ok(session) = self.session_mut(sid) else { return };
                if session.configured || session.configure_pending {
                    // js-debug's server says it again once a child session's adapter takes over
                    // after `launch`. That adapter already has what the first configuration sent,
                    // and answers breakpoints sent again with an empty list.
                    return self.log_event(sid, "ignored", "initialized again; already configured".into());
                }
                if session.launch_sent {
                    self.configure(sid);
                } else {
                    session.configure_pending = true;
                }
                self.log_event(sid, "initialized", String::new());
            }
            Event::Stopped(body) => {
                let hits: Vec<BreakpointId> =
                    body.hit_breakpoint_ids.iter().filter_map(|id| self.breakpoints.by_adapter_id(sid, *id)).collect();
                for id in &hits {
                    self.breakpoints.record_hit(*id);
                }
                let detail = match &body.description {
                    Some(d) => format!("{} · {d}", body.reason),
                    None => body.reason.clone(),
                };
                let Ok(session) = self.session_mut(sid) else { return };
                session.state = SessionState::Stopped;
                session.moved_on();
                session.stopped_thread = body.thread_id.or(session.stopped_thread);
                session.pending_stop =
                    Some(PendingStop { reason: body.reason, description: body.description.or(body.text), hits });
                self.log_event(sid, "stopped", detail);
                self.capture(sid);
            }
            Event::Continued(body) => {
                if let Ok(session) = self.session_mut(sid)
                    && session.is_stopped()
                {
                    session.mark_running();
                }
                self.log_event(sid, "continued", body.thread_id.map(|t| format!("thread {t}")).unwrap_or_default());
            }
            Event::Exited(body) => {
                if let Ok(session) = self.session_mut(sid) {
                    session.exit_code = Some(body.exit_code);
                }
                self.log_event(sid, "exited", format!("code {}", body.exit_code));
            }
            Event::Terminated => {
                let client = self.session_ref(sid).ok().and_then(|s| s.client.clone());
                if let Ok(session) = self.session_mut(sid) {
                    session.mark_exited();
                }
                if let Some(client) = client {
                    let args = DisconnectArguments { terminate_debuggee: false };
                    self.call::<Disconnect>(sid, &client, args, move |r| Msg::Done {
                        sid,
                        command: "disconnect",
                        result: r.map(drop).or(Ok(())),
                    });
                }
                self.end_children(sid);
                let name = self.session_ref(sid).map(|s| s.name.clone()).unwrap_or_default();
                self.notice(Some(sid), Level::Info, format!("{name} terminated"));
                self.log_event(sid, "terminated", String::new());
            }
            Event::Thread(body) => {
                self.log_event(sid, "thread", format!("{} {}", body.reason, body.thread_id));
                self.refresh_threads(sid);
            }
            Event::Output(body) => {
                let program_may_run = self.session_ref(sid).is_ok_and(|s| s.configuration_done);
                if let Some(kind) = OutputKind::from_category(body.category.as_deref(), program_may_run) {
                    if body.category.as_deref() == Some("important") {
                        self.notice(Some(sid), Level::Warn, body.output.trim().to_owned());
                    }
                    self.push_output(sid, kind, &body.output);
                }
            }
            Event::Breakpoint(body) => self.breakpoints.rebind(sid, &body.breakpoint),
            Event::Process(body) => {
                let pid = body.system_process_id.map(|p| format!(" pid {p}")).unwrap_or_default();
                self.log_event(sid, "process", format!("{}{pid}", body.name));
            }
            // js-debug announces each of the hundreds of scripts Node loads; they would push
            // everything else out of EVENTS.
            Event::Other { event, .. } if event == "loadedSource" => {}
            Event::Other { event, .. } => self.log_event(sid, &event, String::new()),
        }
    }

    fn capture(&mut self, sid: SessionId) {
        let Ok(session) = self.session_ref(sid) else { return };
        let Some(client) = session.client.clone() else { return };
        let (epoch, thread) = (session.epoch, session.stopped_thread);
        let expanded: Vec<VarPath> = session.expanded.iter().cloned().collect();
        let seen: Vec<String> = session.seen_scopes.iter().cloned().collect();
        let watches = self.watches.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = capture::capture(client, thread, watches, expanded, seen).await;
            let _ = tx.send(EngineMsg(Msg::Captured { sid, epoch, result }));
        });
    }

    fn on_captured(&mut self, sid: SessionId, epoch: u64, result: DapResult<Capture>) {
        let Ok(session) = self.session_ref(sid) else { return };
        if session.epoch != epoch {
            return; // the program moved on while we were reading it
        }
        let capture = match result {
            Ok(capture) => capture,
            Err(err) => {
                return self.notice(Some(sid), Level::Warn, format!("reading the stop failed: {}", short_error(&err)));
            }
        };
        let mut stop = self.session_mut(sid).ok().and_then(|s| s.pending_stop.take()).unwrap_or(PendingStop {
            reason: "pause".into(),
            description: None,
            hits: Vec::new(),
        });

        // Not every adapter reports hitBreakpointIds; fall back to the stop location.
        let top = capture.frames.first();
        if stop.hits.is_empty()
            && stop.reason == "breakpoint"
            && let Some(bp) = top.and_then(|f| self.breakpoints.at(f.path.as_deref()?, f.line))
        {
            stop.hits.push(bp.id);
            let id = bp.id;
            self.breakpoints.record_hit(id);
        }

        let location = top.map(|f| f.location()).unwrap_or_else(|| "?".into());
        // Where it stopped, with the whole path, for EVENTS to go to.
        let at = top.map(|f| match &f.path {
            Some(path) => format!("{}:{} · {}", path.display(), f.line, f.name),
            None => format!("{location} · {}", f.name),
        });
        let reason = match stop.hits.first() {
            Some(id) => format!("breakpoint #{id}"),
            None => stop.reason.clone(),
        };

        let Ok(session) = self.session_mut(sid) else { return };
        // A native adapter debugs many languages; the first stop tells which one.
        if session.tag == "native"
            && let Some(tag) = top.and_then(|f| f.path.as_deref()).and_then(native_tag)
        {
            session.name = session.name.replace("·native", &format!("·{tag}"));
            session.tag = tag.to_owned();
        }
        if let Some(scopes) = capture.frames.first().and_then(|top| top.scopes.as_deref()) {
            session.see_scopes(scopes);
        }
        for older in &mut session.history {
            older.live = false;
        }
        session.stopped_thread = Some(capture.thread_id);
        let number = u32::try_from(session.history.len()).unwrap_or(u32::MAX - 1) + 1;
        session.history.push(Snapshot {
            number,
            time: Local::now(),
            thread_id: capture.thread_id,
            reason: stop.reason,
            description: stop.description,
            hit_breakpoints: stop.hits,
            frames: capture.frames,
            watches: capture.watches,
            live: true,
        });
        let name = session.name.clone();
        if let Some(at) = at {
            self.log_event(sid, "at", at);
        }
        // Paths expanded while the stop was being read, or whose reading failed, load now.
        self.load_expanded(sid, Some(0), &[]);
        self.notice(Some(sid), Level::Info, format!("{name} stopped: {reason} at {location} · #{number}"));
    }

    fn on_frame_read(
        &mut self,
        sid: SessionId,
        epoch: u64,
        number: u32,
        frame: usize,
        result: DapResult<Vec<ScopeVars>>,
    ) {
        let Ok(session) = self.session_mut(sid) else { return };
        if session.epoch != epoch || session.latest().map(|s| s.number) != Some(number) {
            return; // the program moved on while we were reading it
        }
        session.reading.remove(&frame);
        let scopes = match result {
            Ok(scopes) => scopes,
            Err(err) => {
                let text = format!("reading frame #{frame} failed: {}", short_error(&err));
                return self.notice(Some(sid), Level::Warn, text);
            }
        };
        session.see_scopes(&scopes);
        let Some(target) = session.history.last_mut().and_then(|s| s.frames.get_mut(frame)) else { return };
        target.scopes = Some(scopes);
        self.load_expanded(sid, Some(frame), &[]);
    }

    fn on_children(&mut self, sid: SessionId, number: u32, frame: usize, path: &VarPath, result: DapResult<Vec<Var>>) {
        let vars = match result {
            Ok(vars) => vars,
            Err(err) => {
                // At the live stop the row folds back rather than wait for children that will not come.
                if self.session_ref(sid).ok().and_then(Session::live).is_some_and(|s| s.number == number) {
                    self.collapse(sid, path);
                }
                return self.notice(
                    Some(sid),
                    Level::Warn,
                    format!("expand {}: {}", path.join("."), short_error(&err)),
                );
            }
        };
        let Some(snapshot) = self.session_mut(sid).ok().and_then(|s| s.snapshot_mut(number)) else { return };
        if let [scope] = path.as_slice() {
            let scopes = snapshot.frames.get_mut(frame).and_then(|f| f.scopes.as_mut());
            if let Some(scope) = scopes.and_then(|scopes| scopes.iter_mut().find(|s| &s.name == scope)) {
                scope.vars = Some(vars);
            }
        } else if let Some(var) = snapshot.var_mut(frame, path) {
            var.children = Some(vars);
        }
        // Rows below that stayed expanded can be read now that their parents are there.
        self.load_expanded(sid, Some(frame), path);
    }

    /// Requests children for the rows on `root`'s branch that the live stop shows expanded
    /// but has not read, so that none stays "loading…" with nothing on the way: in frame
    /// `frame`, or with `None` in every frame whose scopes were read.
    fn load_expanded(&self, sid: SessionId, frame: Option<usize>, root: &[String]) {
        let Ok(session) = self.session_ref(sid) else { return };
        let (Some(snapshot), Some(client)) = (session.live(), &session.client) else { return };
        let number = snapshot.number;
        let frames = match frame {
            Some(frame) => frame..frame + 1,
            None => 0..snapshot.frames.len(),
        };
        for frame in frames {
            for (path, reference) in snapshot.unloaded(frame, root, &session.expanded) {
                let args = VariablesArguments { variables_reference: reference };
                self.call::<Variables>(sid, client, args, move |result| Msg::Children {
                    sid,
                    number,
                    frame,
                    path,
                    result: result.map(|r| r.variables.into_iter().map(capture::var).collect()),
                });
            }
        }
    }

    // ---- helpers ----------------------------------------------------------------------

    /// Writes a request now (preserving order) and reports its result as a message.
    fn call<R>(
        &self,
        _sid: SessionId,
        client: &Client,
        args: R::Arguments,
        done: impl FnOnce(DapResult<R::Response>) -> Msg + Send + 'static,
    ) where
        R: Request,
    {
        let tx = self.tx.clone();
        match client.call::<R>(args) {
            Ok(pending) => {
                tokio::spawn(async move {
                    let _ = tx.send(EngineMsg(done(pending.await)));
                });
            }
            Err(err) => {
                let _ = tx.send(EngineMsg(done(Err(err))));
            }
        }
    }

    fn send_breakpoints(&self, sid: SessionId, client: &Client, path: PathBuf) {
        let (ids, breakpoints) = self.breakpoints.for_adapter(&path);
        let source = Source {
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()),
            path: Some(path.display().to_string()),
            ..Source::default()
        };
        let args = SetBreakpointsArguments { source, breakpoints };
        self.call::<SetBreakpoints>(sid, client, args, move |result| Msg::BreakpointsSet { sid, path, ids, result });
    }

    /// Returns whether a request was sent.
    fn send_exception_filters(&self, sid: SessionId) -> bool {
        let Ok(session) = self.session_ref(sid) else { return false };
        let (Some(client), true) = (session.client.clone(), session.configured) else { return false };
        if session.capabilities.exception_breakpoint_filters.is_empty() {
            return false;
        }
        let filters = session.exception_filters.iter().filter(|f| f.enabled).map(|f| f.filter.clone()).collect();
        self.call::<SetExceptionBreakpoints>(sid, &client, SetExceptionBreakpointsArguments { filters }, move |r| {
            Msg::Done { sid, command: "setExceptionBreakpoints", result: r.map(drop) }
        });
        true
    }

    fn breakpoints_changed(&mut self, path: &Path) {
        let targets: Vec<(SessionId, Client)> = self
            .sessions
            .iter()
            .filter(|s| s.configured && s.is_alive() && s.handles(path))
            .filter(|s| self.children(s.id).next().is_none())
            .filter_map(|s| Some((s.id, s.client.clone()?)))
            .collect();
        for (sid, client) in targets {
            self.send_breakpoints(sid, &client, path.to_path_buf());
        }
        self.save();
    }

    fn refresh_threads(&self, sid: SessionId) {
        let Some(client) = self.session_ref(sid).ok().and_then(|s| s.client.clone()) else { return };
        self.call::<Threads>(sid, &client, Empty {}, move |result| Msg::Threads { sid, result });
    }

    fn save(&mut self) {
        let Some(file) = &self.state_file else { return };
        let state = Persisted { breakpoints: self.breakpoints.saved().to_vec(), watches: self.watches.clone() };
        if let Err(err) = persist::save(file, &state) {
            let text = format!("{err:#}");
            self.notice(None, Level::Warn, text);
        }
    }

    fn push_output(&mut self, sid: SessionId, kind: OutputKind, text: &str) {
        let Ok(session) = self.session_mut(sid) else { return };
        let buffer = session.partial_output.entry(kind).or_default();
        buffer.push_str(text);
        let Some(last_newline) = buffer.rfind('\n') else { return };
        let complete: String = buffer.drain(..=last_newline).collect();
        for line in complete.lines() {
            self.output.push(OutputLine {
                time: Local::now(),
                session: sid,
                kind,
                text: line.trim_end_matches('\r').to_owned(),
            });
        }
    }

    fn log_event(&mut self, sid: SessionId, name: &str, detail: String) {
        self.events.push(EventLine { time: Local::now(), session: sid, name: name.to_owned(), detail });
    }

    fn notice(&mut self, session: Option<SessionId>, level: Level, text: String) {
        self.notices.push(Notice { session, level, text });
    }

    /// Ends a session that cannot go on, and the children it has, saying why.
    fn fail(&mut self, sid: SessionId, text: String) {
        if let Ok(session) = self.session_mut(sid) {
            session.state = SessionState::Exited;
        }
        self.release(sid);
        self.end_children(sid);
        self.log_event(sid, "failed", text.clone());
        self.notice(Some(sid), Level::Error, text);
    }

    /// Lets go of the adapter of a session that is over: the process tuitor started for it
    /// is ended, and its temporary directory removed once it has exited, since until then
    /// it, or a browser it started, may still write there.
    fn release(&mut self, sid: SessionId) {
        let Ok(session) = self.session_mut(sid) else { return };
        let (client, tmp) = (session.client.take(), session.tmp.take());
        if client.is_none() && tmp.is_none() {
            return;
        }
        self.releasing.retain(|task| !task.is_finished());
        self.releasing.push(tokio::spawn(async move {
            if let Some(client) = client {
                client.kill_and_wait().await;
            }
            drop(tmp);
        }));
    }

    fn session_ref(&self, sid: SessionId) -> Result<&Session> {
        self.session(sid).ok_or_else(|| anyhow!("no session {sid}"))
    }

    fn session_mut(&mut self, sid: SessionId) -> Result<&mut Session> {
        self.sessions.iter_mut().find(|s| s.id == sid).ok_or_else(|| anyhow!("no session {sid}"))
    }

    fn children(&self, parent: SessionId) -> impl Iterator<Item = &Session> {
        self.sessions.iter().filter(move |s| s.parent == Some(parent))
    }

    /// How to reach the adapter. One that tuitor starts comes with the temporary directory
    /// it gets, for its session to keep until the process has exited.
    fn transport(&self, spec: &AdapterSpec) -> Result<(Transport, Option<TempDir>)> {
        let command = || -> Result<(AdapterCommand, Option<TempDir>)> {
            let program = spec
                .locate(&self.workspace)
                .map_err(|missing| anyhow!("adapter `{}`: {missing}; {}", spec.name, install_help(spec, &missing)))?;
            // Node-based adapters print deprecation warnings that would clutter OUTPUT.
            let mut env = vec![("NODE_NO_WARNINGS".to_owned(), "1".to_owned())];
            // What it and the programs it starts leave in the temporary directory goes with it.
            let tmp = TempDir::create();
            if let Some(dir) = tmp.as_ref().and_then(|tmp| tmp.path().to_str()) {
                env.push(("TMPDIR".to_owned(), dir.to_owned()));
            }
            // The adapter's own settings win.
            env.extend(spec.env.iter().map(|(k, v)| (k.clone(), v.clone())));
            let command = AdapterCommand {
                program: program.display().to_string(),
                args: spec.args.clone(),
                cwd: Some(self.workspace.clone()),
                env,
            };
            Ok((command, tmp))
        };
        match spec.transport {
            TransportKind::Stdio => command().map(|(command, tmp)| (Transport::Stdio(command), tmp)),
            TransportKind::Server => command().map(|(command, tmp)| (Transport::Server(command), tmp)),
            TransportKind::Tcp => {
                let address =
                    spec.address.clone().ok_or_else(|| anyhow!("adapter `{}` needs an `address`", spec.name))?;
                Ok((Transport::Tcp(address), None))
            }
        }
    }

    fn tag_for(&self, spec: &AdapterSpec) -> String {
        if spec.tag == "native" && self.workspace.join("Cargo.toml").exists() {
            "rs".to_owned()
        } else {
            spec.tag.clone()
        }
    }

    fn unique_name(&self, tag: &str, replaces: Option<SessionId>) -> String {
        let base = self.workspace.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.unique(format!("{base}·{tag}"), replaces)
    }

    /// `name`, or `name#2`, `name#3`, … when a live session already has it, other than the
    /// one being replaced and its children.
    fn unique(&self, name: String, replaces: Option<SessionId>) -> String {
        let taken = |candidate: &str| {
            self.sessions.iter().any(|s| s.name == candidate && s.is_alive() && Some(self.root(s.id)) != replaces)
        };
        if !taken(&name) {
            return name;
        }
        (2..).map(|n| format!("{name}#{n}")).find(|c| !taken(c)).unwrap_or(name)
    }
}

/// What installs what an adapter misses: `/install`, unless it is there but off PATH, which
/// DOCTOR explains.
fn install_help(spec: &AdapterSpec, missing: &Missing) -> String {
    match (Package::parse(&spec.name), missing) {
        // Node.js itself, which tuitor does not install.
        (_, Missing::Program(program)) if program == "node" => INSTALL_HELP.to_owned(),
        (Some(package), Missing::Program(_) | Missing::Script(_)) => format!("/install {package} installs it"),
        _ => INSTALL_HELP.to_owned(),
    }
}

fn native_tag(path: &Path) -> Option<&'static str> {
    Some(match path.extension()?.to_str()? {
        "rs" => "rs",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" => "c++",
        "zig" => "zig",
        _ => return None,
    })
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

fn describe(transport: &Transport) -> String {
    match transport {
        Transport::Stdio(cmd) | Transport::Server(cmd) => {
            Path::new(&cmd.program).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        }
        Transport::Tcp(addr) => addr.clone(),
    }
}

async fn futures_join<F: std::future::Future>(futures: Vec<F>) {
    for future in futures {
        let _ = future.await;
    }
}
