//! The engine in a shared session (see [`crate::journal`]). A peer's engine has no adapter: it
//! asks the monitor for what needs one and takes in what the monitor recorded. The monitor's
//! engine runs the adapters as any engine does and says what changed, to be recorded.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeSet, HashMap};
use std::hash::{Hash, Hasher};

use anyhow::{Result, anyhow, bail};
use serde_json::Value;
use tokio::sync::mpsc;

use super::{Engine, EngineMsg, EngineOptions, Level, Notice, Step};
use crate::breakpoints::Breakpoints;
use crate::config::LaunchConfig;
use crate::journal::{Body, Call, FilterState, Journal, Record, SessionMeta, State};
use crate::persist::Persisted;
use crate::session::{ExceptionFilter, Session, SessionId};

/// Where a peer's engine writes what it asks for.
pub struct Outbox {
    journal: Journal,
    peer: String,
    prefix: String,
    next: u64,
    asked: Vec<Call>,
}

impl Outbox {
    pub fn new(journal: Journal, peer: &str) -> Outbox {
        Outbox {
            journal,
            peer: peer.to_owned(),
            prefix: format!("{peer}-{}", std::process::id()),
            next: 0,
            asked: Vec::new(),
        }
    }

    pub fn peer(&self) -> &str {
        &self.peer
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// Appends `call` and returns its id.
    pub fn call(&mut self, call: Call) -> Result<String> {
        self.next += 1;
        let id = format!("{}-{}", self.prefix, self.next);
        self.journal.append(&Record::new(&self.peer, Body::Call { id: id.clone(), call }))?;
        Ok(id)
    }
}

/// What a monitor's engine has recorded already, to tell what changed since.
#[derive(Debug, Default)]
pub struct Published {
    breakpoints: Option<Value>,
    sessions: HashMap<SessionId, Seen>,
    output: u64,
    events: u64,
}

#[derive(Debug)]
struct Seen {
    meta: SessionMeta,
    stops: usize,
    last: u64,
}

impl Published {
    /// The breakpoints as recorded by another, so that they are not recorded again.
    pub fn saw_breakpoints(&mut self, value: Value) {
        self.breakpoints = Some(value);
    }
}

impl Engine {
    /// An engine whose sessions run in a shared session's monitor.
    pub fn peer(options: EngineOptions, outbox: Outbox) -> (Engine, mpsc::UnboundedReceiver<EngineMsg>) {
        let (mut engine, rx) = Engine::new(options);
        engine.link = Some(outbox);
        (engine, rx)
    }

    pub fn outbox(&self) -> Option<&Outbox> {
        self.link.as_ref()
    }

    pub(super) fn ask(&mut self, call: Call) -> Result<()> {
        let outbox = self.link.as_mut().ok_or_else(|| anyhow!("not in a shared session"))?;
        outbox.call(call).map(drop)
    }

    /// Asks once: for what is read once, a frame's scopes or a source.
    pub(super) fn ask_once(&mut self, call: Call) {
        let Some(outbox) = self.link.as_mut() else { return };
        if outbox.asked.contains(&call) {
            return;
        }
        outbox.asked.push(call.clone());
        if let Err(err) = outbox.call(call) {
            self.notices.push(Notice { session: None, level: Level::Warn, text: format!("{err:#}") });
        }
    }

    /// The breakpoints, sets, watches and "my code", as the state file keeps them.
    pub fn persisted(&self) -> Value {
        let state = Persisted {
            breakpoints: self.breakpoints.saved().to_vec(),
            sets: self.breakpoints.sets().to_vec(),
            active_set: Some(self.breakpoints.active_set()),
            watches: self.watches.clone(),
            my_code: self.my_code,
        };
        serde_json::to_value(state).unwrap_or_default()
    }

    /// Runs what a peer asked for; says what it did.
    pub fn run_call(&mut self, call: &Call) -> Result<String> {
        Ok(match call {
            Call::Start { config } => {
                let config = LaunchConfig::from_value(config.clone())?;
                let name = config.name.clone();
                let sid = self.start(config)?;
                format!("starting {name} as #{sid}")
            }
            Call::Stop { session } => {
                self.stop(*session)?;
                format!("stopping #{session}")
            }
            Call::Restart { session } => format!("restarted as #{}", self.restart(*session)?),
            Call::Resume { session, step } => {
                let step = step_named(step)?;
                self.resume(*session, step)?;
                step.name().to_owned()
            }
            Call::Pause { session } => {
                self.pause(*session)?;
                "pausing".to_owned()
            }
            Call::Evaluate { session, frame, expr } => {
                self.evaluate(*session, *frame, expr)?;
                "evaluating".to_owned()
            }
            Call::Expand { session, stop, frame, path } => {
                self.expand(*session, *stop, *frame, path.clone())?;
                format!("expanded {}", path.join("."))
            }
            Call::Collapse { session, path } => {
                self.collapse(*session, path);
                format!("collapsed {}", path.join("."))
            }
            Call::ReadFrame { session, stop, frame } => {
                self.read_frame(*session, *stop, *frame);
                format!("reading frame {frame}")
            }
            Call::ReadSource { session, reference } => {
                self.read_source(*session, *reference);
                "reading the source".to_owned()
            }
            Call::ExceptionFilter { session, filter } => {
                let on = self.toggle_exception_filter(*session, filter)?;
                format!("{filter} {}", if on { "on" } else { "off" })
            }
        })
    }

    /// Takes in what was recorded.
    pub fn apply(&mut self, state: &State) {
        match state {
            State::Breakpoints(value) => self.apply_breakpoints(value),
            State::Session(meta) => self.apply_session(meta),
            State::Stop { session, snapshot } => {
                if let Ok(session) = self.session_mut(*session) {
                    let number = usize::try_from(snapshot.number).unwrap_or(0);
                    if number == session.history.len() + 1 {
                        session.history.push(snapshot.clone());
                    } else if (1..=session.history.len()).contains(&number) {
                        session.history[number - 1] = snapshot.clone();
                    }
                }
            }
            State::Output(line) => self.output.push(line.clone()),
            State::Event(line) => self.events.push(line.clone()),
            State::Notice { session, level, text } => {
                let level = match level.as_str() {
                    "error" => Level::Error,
                    "warn" => Level::Warn,
                    _ => Level::Info,
                };
                self.notices.push(Notice { session: *session, level, text: text.clone() });
            }
        }
    }

    /// Takes in every state of `records`. The sessions of a journal no monitor runs any more
    /// ended with it.
    pub fn replay(&mut self, records: &[Record], ended: bool) {
        for record in records {
            if let Body::State(state) = &record.body {
                self.apply(state);
            }
        }
        if ended {
            for session in self.sessions.iter_mut().filter(|s| s.client.is_none() && s.is_alive()) {
                session.mark_exited();
            }
        }
    }

    fn apply_breakpoints(&mut self, value: &Value) {
        let Ok(state) = serde_json::from_value::<Persisted>(value.clone()) else { return };
        let mut files: BTreeSet<_> = self.breakpoints.files().into_iter().collect();
        let added: Vec<String> = state.watches.iter().filter(|w| !self.watches.contains(w)).cloned().collect();
        self.breakpoints = Breakpoints::from_saved(state.breakpoints, &state.sets, state.active_set);
        self.watches = state.watches.into_iter().filter(|w| !added.contains(w)).collect();
        self.my_code = state.my_code;
        files.extend(self.breakpoints.files());
        for file in files {
            self.breakpoints_changed(&file);
        }
        for watch in added {
            self.add_watch(&watch);
        }
    }

    fn apply_session(&mut self, meta: &SessionMeta) {
        if self.session_ref(meta.id).is_err() {
            let Ok(config) = LaunchConfig::from_value(meta.config.clone()) else { return };
            let Some(adapter) = self.adapters.get(&config.adapter).cloned() else { return };
            let mut session = Session::new(meta.id, meta.name.clone(), adapter, config, meta.tag.clone());
            session.saved = true;
            self.sessions.push(session);
            self.next_session = self.next_session.max(meta.id + 1);
        }
        let Ok(session) = self.session_mut(meta.id) else { return };
        session.parent = meta.parent;
        session.name.clone_from(&meta.name);
        session.commit.clone_from(&meta.commit);
        session.state = meta.state;
        session.exit_code = meta.exit_code;
        session.expanded = meta.expanded.iter().cloned().collect();
        session.exception_filters = meta
            .filters
            .iter()
            .map(|f| ExceptionFilter { filter: f.filter.clone(), label: f.label.clone(), enabled: f.enabled })
            .collect();
    }

    /// What changed since `seen`, as states to record; `seen` is brought up to now.
    pub fn changes(&self, seen: &mut Published) -> Vec<State> {
        let mut states = Vec::new();
        let breakpoints = self.persisted();
        if seen.breakpoints.as_ref() != Some(&breakpoints) {
            states.push(State::Breakpoints(breakpoints.clone()));
            seen.breakpoints = Some(breakpoints);
        }
        for session in &self.sessions {
            let meta = meta_of(session);
            let last = session.history.last().map_or(0, fingerprint);
            let stops = session.history.len();
            let (meta_changed, from, last_changed) = match seen.sessions.get(&session.id) {
                None => (true, 0, false),
                Some(old) => (old.meta != meta, old.stops.min(stops), old.stops == stops && old.last != last),
            };
            if meta_changed {
                states.push(State::Session(meta.clone()));
            }
            for snapshot in &session.history[from..] {
                states.push(State::Stop { session: session.id, snapshot: snapshot.clone() });
            }
            if last_changed && let Some(snapshot) = session.history.last() {
                states.push(State::Stop { session: session.id, snapshot: snapshot.clone() });
            }
            seen.sessions.insert(session.id, Seen { meta, stops, last });
        }
        states.extend(self.output.since(seen.output).cloned().map(State::Output));
        seen.output = self.output.pushed();
        states.extend(self.events.since(seen.events).cloned().map(State::Event));
        seen.events = self.events.pushed();
        states
    }

    /// Everything as recorded already: for a monitor that read the journal.
    pub fn published(&self) -> Published {
        let mut seen = Published::default();
        let _ = self.changes(&mut seen);
        seen
    }
}

fn meta_of(session: &Session) -> SessionMeta {
    SessionMeta {
        id: session.id,
        parent: session.parent,
        name: session.name.clone(),
        tag: session.tag.clone(),
        config: Value::Object(session.config.arguments.clone()),
        commit: session.commit.clone(),
        state: session.state,
        exit_code: session.exit_code,
        expanded: session.expanded.iter().cloned().collect(),
        filters: session
            .exception_filters
            .iter()
            .map(|f| FilterState { filter: f.filter.clone(), label: f.label.clone(), enabled: f.enabled })
            .collect(),
    }
}

fn fingerprint(snapshot: &crate::snapshot::Snapshot) -> u64 {
    let mut hasher = DefaultHasher::new();
    serde_json::to_vec(snapshot).unwrap_or_default().hash(&mut hasher);
    hasher.finish()
}

fn step_named(name: &str) -> Result<Step> {
    Ok(match name {
        "continue" => Step::Continue,
        "next" => Step::Over,
        "step in" => Step::In,
        "step out" => Step::Out,
        other => bail!("no step `{other}`"),
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::Adapters;
    use crate::snapshot::Snapshot;

    fn engine() -> Engine {
        let options =
            EngineOptions { adapters: Adapters::default(), workspace: PathBuf::from("/tmp"), state_file: None };
        Engine::new(options).0
    }

    fn snapshot(number: u32, line: u32) -> Snapshot {
        serde_json::from_value(serde_json::json!({
            "number": number, "time": chrono::Local::now(), "thread_id": 1, "reason": "breakpoint",
            "description": null, "hit_breakpoints": [], "sets": [], "silent": false,
            "frames": [{"id": 1, "name": "main", "path": "/tmp/a.php", "source_name": null,
                        "source_reference": null, "line": line, "column": 1, "library": false, "scopes": null}],
            "watches": [], "live": true
        }))
        .expect("shared")
    }

    fn meta(id: SessionId, state: crate::session::SessionState) -> SessionMeta {
        SessionMeta {
            id,
            parent: None,
            name: "php".into(),
            tag: "php".into(),
            config: serde_json::json!({"name": "Listen", "type": "php", "request": "launch"}),
            commit: None,
            state,
            exit_code: None,
            expanded: vec![vec!["Locals".into()]],
            filters: vec![],
        }
    }

    #[test]
    fn a_peer_takes_in_what_the_monitor_recorded_and_the_monitor_records_only_what_changed() {
        use crate::session::SessionState::{Running, Stopped};
        let mut monitor = engine();
        monitor.apply(&State::Session(meta(4, Stopped)));
        monitor.apply(&State::Stop { session: 4, snapshot: snapshot(1, 10) });
        let mut seen = Published::default();
        let first = monitor.changes(&mut seen);
        assert!(first.iter().any(|s| matches!(s, State::Session(m) if m.id == 4)));
        assert!(first.iter().any(|s| matches!(s, State::Stop { snapshot, .. } if snapshot.number == 1)));
        assert!(monitor.changes(&mut seen).is_empty());

        monitor.apply(&State::Stop { session: 4, snapshot: snapshot(1, 11) });
        monitor.apply(&State::Session(meta(4, Running)));
        let second = monitor.changes(&mut seen);
        assert_eq!(second.len(), 2);

        let mut peer = engine();
        for state in first.iter().chain(&second) {
            peer.apply(state);
        }
        let session = peer.session(4).expect("shared");
        assert_eq!(session.state, Running);
        assert_eq!(session.history.len(), 1);
        assert_eq!(session.history[0].frames[0].line, 11);
        assert_eq!(peer.next_session, 5);
    }

    #[test]
    fn breakpoints_go_from_one_engine_to_another_with_their_ids_and_watches() {
        let mut one = engine();
        one.toggle_breakpoint(std::path::Path::new("/tmp/a.php"), 3);
        one.add_watch("$order");
        let mut other = engine();
        other.apply(&State::Breakpoints(one.persisted()));
        assert_eq!(other.persisted(), one.persisted());
        assert_eq!(other.watches(), ["$order"]);
    }

    #[test]
    fn a_replayed_journal_whose_monitor_is_gone_has_its_live_sessions_ended() {
        let mut engine = engine();
        let records =
            vec![Record::new("monitor", Body::State(State::Session(meta(1, crate::session::SessionState::Stopped))))];
        engine.replay(&records, true);
        assert!(!engine.session(1).expect("shared").is_alive());
    }
}
