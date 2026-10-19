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
use tuitor_dap::requests::{
    Attach, ConfigurationDone, Continue, Disconnect, Initialize, Launch, Next, Pause, Request, SetBreakpoints,
    SetExceptionBreakpoints, StepIn, StepOut, Terminate, Threads, Variables,
};
use tuitor_dap::types::{
    self as dap, Capabilities, DisconnectArguments, Empty, InitializeArguments, SetBreakpointsArguments,
    SetExceptionBreakpointsArguments, Source, ThreadArguments, ThreadsResponse, VariablesArguments,
};
use tuitor_dap::{AdapterCommand, Client, Event, Incoming, Transport};

use crate::breakpoints::{Breakpoint, BreakpointId, Breakpoints, Toggle};
use crate::capture::{self, Capture, canonical, short_error};
use crate::config::{AdapterSpec, Adapters, INSTALL_HELP, LaunchConfig, RequestKind, TransportKind};
use crate::log::{Bounded, EventLine, OutputKind, OutputLine};
use crate::persist::{self, Persisted};
use crate::session::{ExceptionFilter, PendingStop, Session, SessionId, SessionState};
use crate::snapshot::{Snapshot, Var, VarPath, WatchValue};

const OUTPUT_LINES: usize = 10_000;
const EVENT_LINES: usize = 1_000;
const KILL_GRACE: Duration = Duration::from_secs(2);

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
    Children {
        sid: SessionId,
        number: u32,
        path: VarPath,
        result: DapResult<Vec<Var>>,
    },
    Watch {
        sid: SessionId,
        number: u32,
        expr: String,
        result: DapResult<String>,
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
        let spec = self.adapters.get(&config.adapter).cloned().ok_or_else(|| {
            let known: Vec<&str> = self.adapters.names().collect();
            anyhow!("no adapter for type `{}` (known: {})", config.adapter, known.join(", "))
        })?;
        let transport = self.transport(&spec)?;

        let sid = self.next_session;
        self.next_session += 1;
        let tag = self.tag_for(&spec);
        let name = self.unique_name(&tag);
        self.log_event(sid, "starting", format!("{} · {}", config.name, describe(&transport)));
        self.sessions.push(Session::new(sid, name, spec, config, tag));

        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = Client::connect(transport).await;
            let _ = tx.send(EngineMsg(Msg::Connected { sid, result }));
        });
        Ok(sid)
    }

    /// Ends the session's debuggee and adapter.
    pub fn stop(&mut self, sid: SessionId) -> Result<()> {
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

    /// Starts the same configuration again in a new session.
    pub fn restart(&mut self, sid: SessionId) -> Result<SessionId> {
        let config = self.session_ref(sid)?.config.clone();
        if self.session_ref(sid)?.is_alive() {
            self.stop(sid)?;
        }
        self.start(config)
    }

    /// Disconnects every session; used when quitting.
    pub async fn shutdown(&mut self) {
        let clients: Vec<Client> = self.sessions.iter().filter_map(|s| s.client.clone()).collect();
        let goodbyes =
            clients.iter().filter_map(|c| c.call::<Disconnect>(DisconnectArguments { terminate_debuggee: true }).ok());
        let _ = tokio::time::timeout(Duration::from_millis(500), futures_join(goodbyes.collect())).await;
        for client in clients {
            client.kill();
        }
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
        let session = self.session_ref(sid)?;
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

    /// Evaluates in the top frame of the live stop (or globally while running).
    pub fn evaluate(&mut self, sid: SessionId, expr: &str) -> Result<()> {
        let session = self.session_ref(sid)?;
        let client = session.client.clone().ok_or_else(|| anyhow!("{} is not connected", session.name))?;
        let frame_id = session.live().and_then(|s| s.top()).map(|f| f.id);
        let tx = self.tx.clone();
        let expr = expr.to_owned();
        tokio::spawn(async move {
            let result = capture::evaluate(&client, &expr, frame_id, "repl").await;
            let _ = tx.send(EngineMsg(Msg::Evaluated { sid, expr, result }));
        });
        Ok(())
    }

    /// Expands a scope (`[scope]`) or variable path in a snapshot; it stays expanded at later stops.
    pub fn expand(&mut self, sid: SessionId, number: u32, path: VarPath) -> Result<()> {
        let session = self.session_mut(sid)?;
        session.expanded.insert(path.clone());
        let snapshot = session.snapshot(number).ok_or_else(|| anyhow!("no stop #{number}"))?;
        let reference = match path.as_slice() {
            [] => return Ok(()),
            [scope] => {
                let scope =
                    snapshot.scopes.iter().find(|s| &s.name == scope).ok_or_else(|| anyhow!("no scope {scope}"))?;
                if scope.vars.is_some() {
                    return Ok(());
                }
                scope.reference
            }
            _ => {
                let var = snapshot.var(&path).ok_or_else(|| anyhow!("no variable {}", path.join(".")))?;
                if var.children.is_some() || var.reference <= 0 {
                    return Ok(());
                }
                var.reference
            }
        };
        if !snapshot.live || !session.is_stopped() {
            bail!("stop #{number} is history; its children were not captured");
        }
        let client = session.client.clone().ok_or_else(|| anyhow!("{} is not connected", session.name))?;
        self.call::<Variables>(sid, &client, VariablesArguments { variables_reference: reference }, move |result| {
            Msg::Children {
                sid,
                number,
                path,
                result: result.map(|r| r.variables.into_iter().map(capture::var).collect()),
            }
        });
        Ok(())
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
            Msg::BreakpointsSet { sid, path, ids, result } => match result {
                Ok(response) => self.breakpoints.bind(sid, &path, &ids, &response.breakpoints),
                Err(err) => self.notice(
                    Some(sid),
                    Level::Warn,
                    format!("breakpoints in {}: {}", file_name(&path), short_error(&err)),
                ),
            },
            Msg::Captured { sid, epoch, result } => self.on_captured(sid, epoch, result),
            Msg::Children { sid, number, path, result } => self.on_children(sid, number, &path, result),
            Msg::Watch { sid, number, expr, result } => {
                if let Some(snapshot) = self.session_mut(sid).ok().and_then(|s| s.snapshot_mut(number)) {
                    let value = result.map_err(|e| short_error(&e));
                    snapshot.watches.retain(|w| w.expr != expr);
                    snapshot.watches.push(WatchValue { expr, value });
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
            }
            Msg::Done { sid, command, result } => {
                if let Err(err) = result {
                    self.notice(Some(sid), Level::Error, format!("{command}: {}", short_error(&err)));
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
        let (Some(client), request, args) =
            (session.client.clone(), session.config.request, Value::Object(session.config.arguments.clone()))
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

    /// Sends breakpoints and `configurationDone` once the adapter says it is ready.
    fn configure(&mut self, sid: SessionId) {
        let Ok(session) = self.session_mut(sid) else { return };
        session.configured = true;
        session.configure_pending = false;
        if session.state == SessionState::Starting {
            session.state = SessionState::Running;
        }
        let Some(client) = session.client.clone() else { return };
        let config_done = session.capabilities.supports_configuration_done_request;
        let spec = session.adapter.clone();

        for path in self.breakpoints.files().into_iter().filter(|p| spec.handles(p)) {
            self.send_breakpoints(sid, &client, path);
        }
        self.send_exception_filters(sid);
        if config_done {
            self.call::<ConfigurationDone>(sid, &client, Empty {}, move |r| Msg::Done {
                sid,
                command: "configurationDone",
                result: r.map(drop),
            });
        }
    }

    fn on_incoming(&mut self, sid: SessionId, incoming: Incoming) {
        match incoming {
            Incoming::Event(event) => self.on_event(sid, event),
            Incoming::Request(request) => {
                let client = self.session_ref(sid).ok().and_then(|s| s.client.clone());
                if let Some(client) = client {
                    let _ = client.respond(&request, false, Value::Null);
                }
                self.notice(
                    Some(sid),
                    Level::Warn,
                    format!("adapter asked for `{}`, which tuitor does not support yet", request.command),
                );
            }
            Incoming::Log(line) => self.push_output(sid, OutputKind::Adapter, &format!("{line}\n")),
            Incoming::Closed => {
                self.breakpoints.unbind_session(sid);
                if let Ok(session) = self.session_mut(sid) {
                    let was_alive = session.is_alive();
                    session.state = SessionState::Exited;
                    session.client = None;
                    if let Some(last) = session.history.last_mut() {
                        last.live = false;
                    }
                    let name = session.name.clone();
                    if was_alive {
                        self.notice(Some(sid), Level::Warn, format!("{name}: adapter disconnected"));
                    }
                }
                self.log_event(sid, "closed", String::new());
            }
        }
    }

    fn on_event(&mut self, sid: SessionId, event: Event) {
        match event {
            Event::Initialized => {
                let Ok(session) = self.session_mut(sid) else { return };
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
                session.epoch += 1;
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
                    session.state = SessionState::Exited;
                    if let Some(last) = session.history.last_mut() {
                        last.live = false;
                    }
                }
                if let Some(client) = client {
                    let args = DisconnectArguments { terminate_debuggee: false };
                    self.call::<Disconnect>(sid, &client, args, move |r| Msg::Done {
                        sid,
                        command: "disconnect",
                        result: r.map(drop).or(Ok(())),
                    });
                }
                let name = self.session_ref(sid).map(|s| s.name.clone()).unwrap_or_default();
                self.notice(Some(sid), Level::Info, format!("{name} terminated"));
                self.log_event(sid, "terminated", String::new());
            }
            Event::Thread(body) => {
                self.log_event(sid, "thread", format!("{} {}", body.reason, body.thread_id));
                self.refresh_threads(sid);
            }
            Event::Output(body) => {
                if let Some(kind) = OutputKind::from_category(body.category.as_deref()) {
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
        for scope in &capture.scopes {
            if session.seen_scopes.insert(scope.name.clone()) && scope.default_open {
                session.expanded.insert(vec![scope.name.clone()]);
            }
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
            scopes: capture.scopes,
            watches: capture.watches,
            live: true,
        });
        let name = session.name.clone();
        self.notice(Some(sid), Level::Info, format!("{name} stopped: {reason} at {location} · #{number}"));
    }

    fn on_children(&mut self, sid: SessionId, number: u32, path: &VarPath, result: DapResult<Vec<Var>>) {
        let vars = match result {
            Ok(vars) => vars,
            Err(err) => {
                return self.notice(
                    Some(sid),
                    Level::Warn,
                    format!("expand {}: {}", path.join("."), short_error(&err)),
                );
            }
        };
        let Some(snapshot) = self.session_mut(sid).ok().and_then(|s| s.snapshot_mut(number)) else { return };
        if let [scope] = path.as_slice() {
            if let Some(scope) = snapshot.scopes.iter_mut().find(|s| &s.name == scope) {
                scope.vars = Some(vars);
            }
        } else if let Some(var) = snapshot.var_mut(path) {
            var.children = Some(vars);
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

    fn send_exception_filters(&self, sid: SessionId) {
        let Ok(session) = self.session_ref(sid) else { return };
        let (Some(client), true) = (session.client.clone(), session.configured) else { return };
        if session.capabilities.exception_breakpoint_filters.is_empty() {
            return;
        }
        let filters = session.exception_filters.iter().filter(|f| f.enabled).map(|f| f.filter.clone()).collect();
        self.call::<SetExceptionBreakpoints>(sid, &client, SetExceptionBreakpointsArguments { filters }, move |r| {
            Msg::Done { sid, command: "setExceptionBreakpoints", result: r.map(drop) }
        });
    }

    fn breakpoints_changed(&mut self, path: &Path) {
        let targets: Vec<(SessionId, Client)> = self
            .sessions
            .iter()
            .filter(|s| s.configured && s.is_alive() && s.handles(path))
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

    fn fail(&mut self, sid: SessionId, text: String) {
        if let Ok(session) = self.session_mut(sid) {
            session.state = SessionState::Exited;
            if let Some(client) = session.client.take() {
                client.kill();
            }
        }
        self.log_event(sid, "failed", text.clone());
        self.notice(Some(sid), Level::Error, text);
    }

    fn session_ref(&self, sid: SessionId) -> Result<&Session> {
        self.session(sid).ok_or_else(|| anyhow!("no session {sid}"))
    }

    fn session_mut(&mut self, sid: SessionId) -> Result<&mut Session> {
        self.sessions.iter_mut().find(|s| s.id == sid).ok_or_else(|| anyhow!("no session {sid}"))
    }

    fn transport(&self, spec: &AdapterSpec) -> Result<Transport> {
        let command = || -> Result<AdapterCommand> {
            let program = spec
                .locate(&self.workspace)
                .map_err(|missing| anyhow!("adapter `{}`: {missing}; {INSTALL_HELP}", spec.name))?;
            Ok(AdapterCommand {
                program: program.display().to_string(),
                args: spec.args.clone(),
                cwd: Some(self.workspace.clone()),
                // Node-based adapters print deprecation warnings that would clutter OUTPUT.
                env: std::iter::once(("NODE_NO_WARNINGS".to_owned(), "1".to_owned()))
                    .chain(spec.env.iter().map(|(k, v)| (k.clone(), v.clone())))
                    .collect(),
            })
        };
        Ok(match spec.transport {
            TransportKind::Stdio => Transport::Stdio(command()?),
            TransportKind::Server => Transport::Server(command()?),
            TransportKind::Tcp => Transport::Tcp(
                spec.address.clone().ok_or_else(|| anyhow!("adapter `{}` needs an `address`", spec.name))?,
            ),
        })
    }

    fn tag_for(&self, spec: &AdapterSpec) -> String {
        if spec.tag == "native" && self.workspace.join("Cargo.toml").exists() {
            "rs".to_owned()
        } else {
            spec.tag.clone()
        }
    }

    fn unique_name(&self, tag: &str) -> String {
        let base = self.workspace.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let name = format!("{base}·{tag}");
        let taken = |candidate: &str| self.sessions.iter().any(|s| s.name == candidate && s.is_alive());
        if !taken(&name) {
            return name;
        }
        (2..).map(|n| format!("{name}#{n}")).find(|c| !taken(c)).unwrap_or(name)
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
