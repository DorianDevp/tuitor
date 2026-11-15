//! `tuitor monitor <session>`: the one process of a shared session that talks to the adapters,
//! as conmon is for a podman container. It runs an engine with no screen, does what peers ask
//! in the order the journal has it, waits for every peer's yes before a call that moves the
//! program on, and records what happened. It decides nothing itself, and ends once no
//! session runs and no peer is in.

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::Result;
use tuitor_core::config::Adapters;
use tuitor_core::journal::{Body, Call, Journal, Peers, Proposals, Record, State, session_dir};
use tuitor_core::{Engine, EngineOptions, Level};

use crate::signals::Signals;

pub const NAME: &str = "monitor";
const TICK: Duration = Duration::from_millis(100);
const FOLLOW_EVERY: Duration = Duration::from_millis(500);
const IDLE_END: Duration = Duration::from_secs(3);

pub async fn run(name: &str, workspace: PathBuf, adapters: Adapters, mut signals: Signals) -> Result<ExitCode> {
    let journal = Journal::open(&session_dir(name)?)?;
    let Some(_lock) = journal.hold_monitor()? else { return Ok(ExitCode::SUCCESS) };
    let mut reader = journal.reader();
    let history = reader.read()?;
    let (mut engine, mut rx) = Engine::new(EngineOptions { adapters, workspace, state_file: None });
    engine.replay(&history, false);
    let mut seen = engine.published();
    engine.replay(&[], true);

    let mut monitor = Monitor { journal, peers: Peers::default(), proposals: Proposals::default() };
    let mut answered: HashSet<String> = HashSet::new();
    for record in &history {
        monitor.peers.see(record);
        monitor.proposals.see(record);
        if let Body::Reply { call, .. } | Body::Decided { call, .. } = &record.body {
            answered.insert(call.clone());
        }
    }
    for record in &history {
        if let Body::Call { id, call } = &record.body
            && !call.controls()
            && !answered.contains(id)
        {
            monitor.run(&mut engine, id, call);
        }
    }

    let mut tick = tokio::time::interval(TICK);
    let mut followed = Instant::now();
    let mut idle_since: Option<Instant> = None;
    loop {
        tokio::select! {
            Some(msg) = rx.recv() => {
                engine.handle(msg);
                while let Ok(msg) = rx.try_recv() {
                    engine.handle(msg);
                }
            }
            _ = tick.tick() => {}
            code = signals.recv() => {
                engine.shutdown().await;
                return Ok(code);
            }
        }
        for record in reader.read()? {
            monitor.peers.see(&record);
            monitor.proposals.see(&record);
            match &record.body {
                Body::Call { id, call } if !call.controls() => monitor.run(&mut engine, id, call),
                Body::State(state @ State::Breakpoints(_)) if record.peer != NAME => {
                    engine.apply(state);
                    seen.saw_breakpoints(engine.persisted());
                }
                _ => {}
            }
        }
        monitor.decide(&mut engine);
        if followed.elapsed() >= FOLLOW_EVERY {
            followed = Instant::now();
            engine.follow_files();
        }
        for notice in engine.take_notices() {
            let level = match notice.level {
                Level::Info => "info",
                Level::Warn => "warn",
                Level::Error => "error",
            };
            monitor.record(Body::State(State::Notice {
                session: notice.session,
                level: level.into(),
                text: notice.text,
            }));
        }
        for state in engine.changes(&mut seen) {
            monitor.record(Body::State(state));
        }
        let busy = engine.sessions().iter().any(|s| s.is_alive()) || !monitor.peers.attached().is_empty();
        match (busy, idle_since) {
            (true, _) => idle_since = None,
            (false, None) => idle_since = Some(Instant::now()),
            (false, Some(since)) if since.elapsed() >= IDLE_END => break,
            (false, Some(_)) => {}
        }
    }
    engine.shutdown().await;
    Ok(ExitCode::SUCCESS)
}

struct Monitor {
    journal: Journal,
    peers: Peers,
    proposals: Proposals,
}

impl Monitor {
    fn record(&self, body: Body) {
        if let Err(err) = self.journal.append(&Record::new(NAME, body)) {
            tracing::warn!("recording in the session's journal: {err:#}");
        }
    }

    fn run(&self, engine: &mut Engine, id: &str, call: &Call) {
        let (ok, text) = match engine.run_call(call) {
            Ok(text) => (true, text),
            Err(err) => (false, format!("{}: {err:#}", call.describe())),
        };
        self.record(Body::Reply { call: id.to_owned(), ok, text });
    }

    /// Runs each call that every attached peer said yes to; drops each that one said no to.
    fn decide(&self, engine: &mut Engine) {
        let attached = self.peers.attached();
        for proposal in self.proposals.open().to_vec() {
            if let Some(no) = &proposal.no {
                self.record(Body::Decided { call: proposal.id.clone(), passed: false });
                let text = format!("{} said no to {}", no, proposal.call.describe());
                self.record(Body::Reply { call: proposal.id.clone(), ok: false, text });
            } else if self.peers.is_su(&proposal.by) || proposal.waiting_for(&attached).is_empty() {
                self.record(Body::Decided { call: proposal.id.clone(), passed: true });
                self.run(engine, &proposal.id, &proposal.call);
            }
        }
    }
}
