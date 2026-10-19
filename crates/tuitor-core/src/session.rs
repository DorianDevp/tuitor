//! One debug adapter connection and everything recorded while it ran.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use tuitor_dap::Client;
use tuitor_dap::types::{Capabilities, Thread};

use crate::breakpoints::BreakpointId;
use crate::config::{AdapterSpec, LaunchConfig};
use crate::log::OutputKind;
use crate::snapshot::{ScopeVars, Snapshot, VarPath};
use crate::tmpdir::TempDir;

pub type SessionId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Adapter starting, initializing or waiting for the debuggee.
    Starting,
    Running,
    Stopped,
    Exited,
}

#[derive(Debug, Clone)]
pub struct ExceptionFilter {
    pub filter: String,
    pub label: String,
    pub enabled: bool,
}

/// Stop information that arrives with the `stopped` event and waits for the capture.
#[derive(Debug, Clone)]
pub(crate) struct PendingStop {
    pub reason: String,
    pub description: Option<String>,
    pub hits: Vec<BreakpointId>,
}

pub struct Session {
    pub id: SessionId,
    /// The session whose adapter asked for this one with `startDebugging`. js-debug runs
    /// each Node process or browser page in such a child; the parent only launches them.
    pub parent: Option<SessionId>,
    pub name: String,
    pub tag: String,
    pub adapter: AdapterSpec,
    pub config: LaunchConfig,
    pub state: SessionState,
    pub exit_code: Option<i64>,
    pub capabilities: Capabilities,
    pub threads: Vec<Thread>,
    /// Every stop, oldest first. Numbers are 1-based indices.
    pub history: Vec<Snapshot>,
    pub exception_filters: Vec<ExceptionFilter>,
    /// Variable paths the user expanded; re-fetched at every stop.
    pub expanded: BTreeSet<VarPath>,
    /// Scope names already shown once; their expansion is now up to the user.
    pub(crate) seen_scopes: BTreeSet<String>,
    pub(crate) client: Option<Client>,
    /// `TMPDIR` of the adapter process tuitor started for this session, until it has exited.
    pub(crate) tmp: Option<TempDir>,
    pub(crate) stopped_thread: Option<i64>,
    /// Bumped whenever execution moves; results computed for an older epoch are dropped.
    pub(crate) epoch: u64,
    /// Frames of the live stop whose scopes are being read.
    pub(crate) reading: BTreeSet<usize>,
    pub(crate) launch_sent: bool,
    pub(crate) configure_pending: bool,
    pub(crate) pending_stop: Option<PendingStop>,
    /// Partial output lines per kind, until their newline arrives.
    pub(crate) partial_output: HashMap<OutputKind, String>,
    pub(crate) configured: bool,
    /// Requests sent by `configure` that the adapter has not answered; `configurationDone` waits for them.
    pub(crate) configuration_unanswered: usize,
    /// The adapter answered `configurationDone` (or does not support it). Until then the
    /// debuggee cannot run, so output sent before is the adapter's own.
    pub(crate) configuration_done: bool,
}

impl Session {
    pub(crate) fn new(id: SessionId, name: String, adapter: AdapterSpec, config: LaunchConfig, tag: String) -> Self {
        Self {
            id,
            parent: None,
            name,
            tag,
            adapter,
            config,
            state: SessionState::Starting,
            exit_code: None,
            capabilities: Capabilities::default(),
            threads: Vec::new(),
            history: Vec::new(),
            exception_filters: Vec::new(),
            expanded: BTreeSet::new(),
            seen_scopes: BTreeSet::new(),
            client: None,
            tmp: None,
            stopped_thread: None,
            epoch: 0,
            reading: BTreeSet::new(),
            launch_sent: false,
            configure_pending: false,
            pending_stop: None,
            partial_output: HashMap::new(),
            configured: false,
            configuration_unanswered: 0,
            configuration_done: false,
        }
    }

    pub fn latest(&self) -> Option<&Snapshot> {
        self.history.last()
    }

    pub fn snapshot(&self, number: u32) -> Option<&Snapshot> {
        self.history.get(usize::try_from(number).ok()?.checked_sub(1)?)
    }

    pub(crate) fn snapshot_mut(&mut self, number: u32) -> Option<&mut Snapshot> {
        self.history.get_mut(usize::try_from(number).ok()?.checked_sub(1)?)
    }

    pub fn is_stopped(&self) -> bool {
        self.state == SessionState::Stopped
    }

    pub fn is_alive(&self) -> bool {
        self.state != SessionState::Exited
    }

    pub fn handles(&self, path: &Path) -> bool {
        self.adapter.handles(path)
    }

    /// The temporary directory of the adapter process tuitor started for this session,
    /// while the process runs.
    pub fn temp_dir(&self) -> Option<&Path> {
        self.tmp.as_ref().map(TempDir::path)
    }

    /// The latest snapshot while its variable references are still valid.
    pub fn live(&self) -> Option<&Snapshot> {
        self.latest().filter(|s| s.live && self.is_stopped())
    }

    /// Whether the scopes of frame `frame` of the live stop are on their way.
    pub fn reading_frame(&self, frame: usize) -> bool {
        self.reading.contains(&frame)
    }

    /// Execution moved: what was being read for the stop before is of no use.
    pub(crate) fn moved_on(&mut self) {
        self.epoch += 1;
        self.reading.clear();
    }

    /// Remembers scopes shown for the first time; those shown by default start expanded,
    /// and from then on it is up to the user.
    pub(crate) fn see_scopes(&mut self, scopes: &[ScopeVars]) {
        for scope in scopes {
            if self.seen_scopes.insert(scope.name.clone()) && scope.default_open {
                self.expanded.insert(vec![scope.name.clone()]);
            }
        }
    }

    pub(crate) fn mark_exited(&mut self) {
        self.state = SessionState::Exited;
        if let Some(last) = self.history.last_mut() {
            last.live = false;
        }
    }

    pub(crate) fn mark_running(&mut self) {
        self.state = SessionState::Running;
        self.moved_on();
        if let Some(last) = self.history.last_mut() {
            last.live = false;
        }
    }
}
