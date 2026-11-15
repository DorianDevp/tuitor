//! Shared sessions. A session is a directory with one append-only journal that every tuitor
//! in it appends to and reads back, a line of JSON each. Peers, a terminal or a command
//! line, append what they ask for and the breakpoints they changed; the session's monitor,
//! the one process that talks to the adapters, appends what happened. Read from the start,
//! the journal is the session as it was, so a later tuitor picks up where an earlier one
//! left off.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::log::{EventLine, OutputLine};
use crate::session::{SessionId, SessionState};
use crate::snapshot::{Snapshot, VarPath};

const JOURNAL: &str = "journal.jsonl";
const MONITOR_LOCK: &str = "monitor.lock";

/// Where named sessions live.
pub fn sessions_dir() -> Option<PathBuf> {
    Some(dirs::state_dir().or_else(dirs::data_local_dir)?.join("tuitor").join("sessions"))
}

/// The directory of session `name`; a name with a `/` is a directory itself.
pub fn session_dir(name: &str) -> Result<PathBuf> {
    if name.contains('/') {
        return Ok(PathBuf::from(name));
    }
    if name.is_empty() || name.starts_with('.') {
        bail!("`{name}` is not a session name");
    }
    sessions_dir().map(|dir| dir.join(name)).context("no state directory for tuitor's sessions")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub time: DateTime<Local>,
    pub peer: String,
    pub body: Body,
}

impl Record {
    pub fn new(peer: &str, body: Body) -> Record {
        Record { time: Local::now(), peer: peer.to_owned(), body }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Body {
    /// A peer is in the session; with the process that keeps it there, if any.
    Join {
        pid: Option<u32>,
    },
    Leave,
    Call {
        id: String,
        call: Call,
    },
    Vote {
        call: String,
        yes: bool,
    },
    /// A call that waited for votes passed, or did not.
    Decided {
        call: String,
        passed: bool,
    },
    Reply {
        call: String,
        ok: bool,
        text: String,
    },
    Note {
        text: String,
        at: Option<At>,
    },
    State(State),
}

/// A frame of a stop, for a note to point at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct At {
    pub session: SessionId,
    pub stop: u32,
    pub frame: usize,
}

/// What a peer asks the monitor to do with an adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Call {
    /// A launch configuration, as launch.json has it.
    Start {
        config: Value,
    },
    Stop {
        session: SessionId,
    },
    Restart {
        session: SessionId,
    },
    /// `continue`, `next`, `step in` or `step out`.
    Resume {
        session: SessionId,
        step: String,
    },
    Pause {
        session: SessionId,
    },
    Evaluate {
        session: SessionId,
        frame: usize,
        expr: String,
    },
    Expand {
        session: SessionId,
        stop: u32,
        frame: usize,
        path: VarPath,
    },
    Collapse {
        session: SessionId,
        path: VarPath,
    },
    ReadFrame {
        session: SessionId,
        stop: u32,
        frame: usize,
    },
    ReadSource {
        session: SessionId,
        reference: i64,
    },
    ExceptionFilter {
        session: SessionId,
        filter: String,
    },
}

impl Call {
    /// Whether it moves the program on, or ends it: such a call waits for every peer's vote.
    pub fn controls(&self) -> bool {
        matches!(self, Call::Stop { .. } | Call::Restart { .. } | Call::Resume { .. } | Call::Pause { .. })
    }

    pub fn describe(&self) -> String {
        match self {
            Call::Start { config } => {
                format!("start {}", config.get("name").and_then(Value::as_str).unwrap_or("a configuration"))
            }
            Call::Stop { session } => format!("stop #{session}"),
            Call::Restart { session } => format!("restart #{session}"),
            Call::Resume { session, step } => format!("{step} #{session}"),
            Call::Pause { session } => format!("pause #{session}"),
            Call::Evaluate { expr, .. } => format!("eval {expr}"),
            Call::Expand { path, .. } => format!("expand {}", path.join(".")),
            Call::Collapse { path, .. } => format!("collapse {}", path.join(".")),
            Call::ReadFrame { stop, frame, .. } => format!("read frame {frame} of stop {stop}"),
            Call::ReadSource { reference, .. } => format!("read source {reference}"),
            Call::ExceptionFilter { filter, .. } => format!("exception filter {filter}"),
        }
    }
}

/// What happened, as recorded: by the monitor, and the breakpoints by whoever changed them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Breakpoints, sets, watches and "my code", as the state file has them.
    Breakpoints(Value),
    Session(SessionMeta),
    /// A new stop, or a stop read further: its frames, variables or watches.
    Stop {
        session: SessionId,
        snapshot: Snapshot,
    },
    Output(OutputLine),
    Event(EventLine),
    Notice {
        session: Option<SessionId>,
        level: String,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: SessionId,
    pub parent: Option<SessionId>,
    pub name: String,
    pub tag: String,
    pub config: Value,
    pub commit: Option<String>,
    pub state: SessionState,
    pub exit_code: Option<i64>,
    pub expanded: Vec<VarPath>,
    pub filters: Vec<FilterState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterState {
    pub filter: String,
    pub label: String,
    pub enabled: bool,
}

/// A session's directory and its journal.
#[derive(Debug, Clone)]
pub struct Journal {
    dir: PathBuf,
}

impl Journal {
    pub fn open(dir: &Path) -> Result<Journal> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(Journal { dir: dir.to_path_buf() })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(JOURNAL)
    }

    /// Appends `record` as one line, written at once under the file's lock, so lines of
    /// several processes never mix.
    pub fn append(&self, record: &Record) -> Result<()> {
        let mut line = serde_json::to_vec(record)?;
        line.push(b'\n');
        let path = self.path();
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        file.lock().with_context(|| format!("locking {}", path.display()))?;
        file.write_all(&line).with_context(|| format!("writing {}", path.display()))?;
        file.unlock()?;
        Ok(())
    }

    /// A reader from the first record.
    pub fn reader(&self) -> Reader {
        Reader { path: self.path(), offset: 0, partial: Vec::new() }
    }

    /// A reader that skips what is in the journal now.
    pub fn reader_at_end(&self) -> Reader {
        let offset = std::fs::metadata(self.path()).map(|m| m.len()).unwrap_or(0);
        Reader { path: self.path(), offset, partial: Vec::new() }
    }

    /// Every record so far.
    pub fn read_all(&self) -> Result<Vec<Record>> {
        self.reader().read()
    }

    /// The monitor's lock, held for as long as the file is kept; `None` while another
    /// process holds it.
    pub fn hold_monitor(&self) -> Result<Option<File>> {
        let path = self.dir.join(MONITOR_LOCK);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(err)) => Err(err).with_context(|| format!("locking {}", path.display())),
        }
    }

    /// Whether a monitor runs for the session.
    pub fn monitor_alive(&self) -> bool {
        matches!(self.hold_monitor(), Ok(None))
    }
}

/// Reads a journal as it grows.
#[derive(Debug)]
pub struct Reader {
    path: PathBuf,
    offset: u64,
    partial: Vec<u8>,
}

impl Reader {
    /// The records appended since the last read; a line still being written waits for the next.
    pub fn read(&mut self) -> Result<Vec<Record>> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err).with_context(|| format!("opening {}", self.path.display())),
        };
        let len = file.metadata()?.len();
        if len < self.offset {
            self.offset = 0;
            self.partial.clear();
        }
        if len == self.offset {
            return Ok(Vec::new());
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).with_context(|| format!("reading {}", self.path.display()))?;
        self.offset += bytes.len() as u64;
        self.partial.extend_from_slice(&bytes);
        let Some(end) = self.partial.iter().rposition(|b| *b == b'\n') else { return Ok(Vec::new()) };
        let rest = self.partial.split_off(end + 1);
        let complete = std::mem::replace(&mut self.partial, rest);
        let mut records = Vec::new();
        for line in complete.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            match serde_json::from_slice(line) {
                Ok(record) => records.push(record),
                Err(err) => tracing::warn!("a journal line that does not read: {err}"),
            }
        }
        Ok(records)
    }
}

/// Who is in the session, from its Join and Leave records.
#[derive(Debug, Default)]
pub struct Peers {
    joined: BTreeMap<String, Option<u32>>,
}

impl Peers {
    pub fn see(&mut self, record: &Record) {
        match record.body {
            Body::Join { pid } => {
                self.joined.insert(record.peer.clone(), pid);
            }
            Body::Leave => {
                self.joined.remove(&record.peer);
            }
            _ => {}
        }
    }

    /// The peers in the session now: a peer kept in by a process is in while it runs.
    pub fn attached(&self) -> BTreeSet<String> {
        self.joined.iter().filter(|(_, pid)| pid.is_none_or(process_alive)).map(|(name, _)| name.clone()).collect()
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    let Ok(pid) = i32::try_from(pid) else { return false };
    matches!(kill(Pid::from_raw(pid), None), Ok(()) | Err(Errno::EPERM))
}

#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    true
}

/// A call that waits for votes.
#[derive(Debug, Clone)]
pub struct Proposal {
    pub id: String,
    pub by: String,
    pub call: Call,
    pub yes: BTreeSet<String>,
    pub no: Option<String>,
}

impl Proposal {
    /// Who still has to vote, of `attached`.
    pub fn waiting_for(&self, attached: &BTreeSet<String>) -> Vec<String> {
        attached.iter().filter(|peer| !self.yes.contains(*peer)).cloned().collect()
    }
}

/// The calls that wait for votes, from the journal.
#[derive(Debug, Default)]
pub struct Proposals {
    open: Vec<Proposal>,
}

impl Proposals {
    pub fn see(&mut self, record: &Record) {
        match &record.body {
            Body::Call { id, call } if call.controls() => self.open.push(Proposal {
                id: id.clone(),
                by: record.peer.clone(),
                call: call.clone(),
                yes: BTreeSet::from([record.peer.clone()]),
                no: None,
            }),
            Body::Vote { call, yes } => {
                if let Some(proposal) = self.open.iter_mut().find(|p| &p.id == call) {
                    if *yes {
                        proposal.yes.insert(record.peer.clone());
                    } else {
                        proposal.no = Some(record.peer.clone());
                    }
                }
            }
            Body::Decided { call, .. } => self.open.retain(|p| &p.id != call),
            _ => {}
        }
    }

    pub fn open(&self) -> &[Proposal] {
        &self.open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tuitor-journal-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn appended_records_read_back_in_order_and_a_reader_takes_only_what_is_new() {
        let journal = Journal::open(&temp_dir("order")).expect("journal");
        let mut reader = journal.reader();
        journal.append(&Record::new("a", Body::Join { pid: None })).expect("journal");
        journal.append(&Record::new("b", Body::Note { text: "hi".into(), at: None })).expect("journal");
        let first = reader.read().expect("journal");
        assert_eq!(first.iter().map(|r| r.peer.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        journal.append(&Record::new("a", Body::Leave)).expect("journal");
        let second = reader.read().expect("journal");
        assert_eq!(second.len(), 1);
        assert!(matches!(second[0].body, Body::Leave));
        assert!(reader.read().expect("journal").is_empty());
        assert_eq!(journal.read_all().expect("journal").len(), 3);
    }

    #[test]
    fn a_line_still_being_written_waits_for_its_end() {
        let journal = Journal::open(&temp_dir("partial")).expect("journal");
        let mut reader = journal.reader();
        let mut line = serde_json::to_vec(&Record::new("a", Body::Leave)).expect("journal");
        let tail = line.split_off(10);
        let mut file = OpenOptions::new().create(true).append(true).open(journal.path()).expect("journal");
        file.write_all(&line).expect("journal");
        assert!(reader.read().expect("journal").is_empty());
        file.write_all(&tail).expect("journal");
        file.write_all(b"\n").expect("journal");
        assert_eq!(reader.read().expect("journal").len(), 1);
    }

    #[test]
    fn appends_from_many_threads_keep_every_line_whole() {
        let journal = Journal::open(&temp_dir("threads")).expect("journal");
        let text = "x".repeat(20_000);
        std::thread::scope(|scope| {
            for t in 0..8 {
                let (journal, text) = (journal.clone(), text.clone());
                scope.spawn(move || {
                    for _ in 0..25 {
                        journal
                            .append(&Record::new(&format!("p{t}"), Body::Note { text: text.clone(), at: None }))
                            .expect("journal");
                    }
                });
            }
        });
        let records = journal.read_all().expect("journal");
        assert_eq!(records.len(), 200);
        assert!(records.iter().all(|r| matches!(&r.body, Body::Note { text: t, .. } if t.len() == 20_000)));
    }

    #[test]
    fn one_monitor_holds_the_lock() {
        let journal = Journal::open(&temp_dir("lock")).expect("journal");
        assert!(!journal.monitor_alive());
        let held = journal.hold_monitor().expect("journal");
        assert!(held.is_some());
        assert!(journal.hold_monitor().expect("journal").is_none());
        assert!(journal.monitor_alive());
        drop(held);
        assert!(!journal.monitor_alive());
    }

    #[test]
    fn a_control_call_waits_until_every_attached_peer_said_yes_and_one_no_cancels_it() {
        let call = Call::Resume { session: 1, step: "continue".into() };
        let mut proposals = Proposals::default();
        let mut peers = Peers::default();
        for record in [
            Record::new("dorian", Body::Join { pid: None }),
            Record::new("claude", Body::Join { pid: None }),
            Record::new("dorian", Body::Call { id: "d-1".into(), call: call.clone() }),
            Record::new(
                "dorian",
                Body::Call { id: "d-2".into(), call: Call::Expand { session: 1, stop: 1, frame: 0, path: vec![] } },
            ),
        ] {
            peers.see(&record);
            proposals.see(&record);
        }
        let attached = peers.attached();
        assert_eq!(proposals.open().len(), 1);
        assert_eq!(proposals.open()[0].waiting_for(&attached), ["claude"]);
        proposals.see(&Record::new("claude", Body::Vote { call: "d-1".into(), yes: true }));
        assert!(proposals.open()[0].waiting_for(&attached).is_empty());
        proposals.see(&Record::new("dorian", Body::Call { id: "d-3".into(), call }));
        proposals.see(&Record::new("claude", Body::Vote { call: "d-3".into(), yes: false }));
        assert_eq!(proposals.open()[1].no.as_deref(), Some("claude"));
        proposals.see(&Record::new("monitor", Body::Decided { call: "d-1".into(), passed: true }));
        assert_eq!(proposals.open().len(), 1);
    }

    #[test]
    fn a_peer_kept_in_by_a_process_that_ended_is_not_attached() {
        let mut peers = Peers::default();
        peers.see(&Record::new("tui", Body::Join { pid: Some(std::process::id()) }));
        peers.see(&Record::new("gone", Body::Join { pid: Some(u32::MAX - 7) }));
        peers.see(&Record::new("cli", Body::Join { pid: None }));
        peers.see(&Record::new("left", Body::Join { pid: None }));
        peers.see(&Record::new("left", Body::Leave));
        assert_eq!(peers.attached().into_iter().collect::<Vec<_>>(), ["cli", "tui"]);
    }
}
