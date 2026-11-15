//! A tab in a shared session: its engine is a peer's (see [`tuitor_core::journal`]). Each
//! turn of the loop reads what the journal got since, takes in what the monitor recorded,
//! and records the breakpoints this tuitor changed. A call that waits for votes shows in a
//! popup over the screen until it is decided: `y` or `n` on another's, `w` withdraws one's
//! own, `esc` hides it until `/vote` shows it again.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::layout::Rect;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedReceiver;
use tuitor_core::journal::{At, Body, Journal, Peers, Proposals, Reader, Record, State, session_dir};
use tuitor_core::{Engine, EngineMsg, EngineOptions, Outbox, persist};

use crate::app::{App, Outcome};
use crate::keymap::Key;
use crate::ui::paint::{Painter, Panel, seg};
use crate::ui::theme::{AMBER_BG, DIM, FAINT, FG, WHITE, bold, fg};

/// How often a peer looks at the journal.
pub const SYNC_EVERY: Duration = Duration::from_millis(100);
const MONITOR_CHECK: Duration = Duration::from_secs(2);
const NOTES_KEPT: usize = 50;

/// The session this tuitor is in, with `--session`.
#[derive(Debug, Clone)]
pub struct Joined {
    pub name: String,
    pub workspace: PathBuf,
    pub peer: String,
}

static JOINED: OnceLock<Joined> = OnceLock::new();

pub fn join(joined: Joined) {
    let _ = JOINED.set(joined);
}

pub fn active() -> bool {
    JOINED.get().is_some()
}

/// The peer's name: `--as`, else the user's.
pub fn peer_name(asked: Option<&str>) -> String {
    asked.map(str::to_owned).unwrap_or_else(|| std::env::var("USER").unwrap_or_else(|_| "peer".into()))
}

/// A tab's place in the session.
pub struct Link {
    pub name: String,
    pub peer: String,
    journal: Journal,
    reader: Reader,
    workspace: PathBuf,
    pub peers: Peers,
    pub proposals: Proposals,
    pub notes: Vec<Record>,
    breakpoints: Option<Value>,
    checked: Instant,
    /// The call whose popup `esc` hid.
    hidden: Option<String>,
}

/// What the vote popup asks.
pub enum Prompt {
    /// Another peer's call, waiting for this peer's yes or no.
    Theirs { id: String, by: String, what: String, waiting: Vec<String> },
    /// This peer's call, waiting for the others.
    Mine { id: String, what: String, waiting: Vec<String> },
}

/// The engine of a tab for `dir`: a peer's, keeping nothing in the project's state file, when
/// `dir` is the session's project.
pub fn engine_for(dir: &Path, options: EngineOptions) -> (Engine, UnboundedReceiver<EngineMsg>, Option<Link>) {
    let Some(joined) = JOINED.get().filter(|j| j.workspace == dir) else {
        let (engine, rx) = Engine::new(options);
        return (engine, rx, None);
    };
    let peer_options =
        EngineOptions { adapters: options.adapters.clone(), workspace: options.workspace.clone(), state_file: None };
    match link(joined, peer_options) {
        Ok((engine, rx, link)) => (engine, rx, Some(link)),
        Err(err) => {
            let (mut engine, rx) = Engine::new(options);
            engine.apply(&State::Notice {
                session: None,
                level: "error".into(),
                text: format!("shared session: {err:#}"),
            });
            (engine, rx, None)
        }
    }
}

fn link(joined: &Joined, options: EngineOptions) -> Result<(Engine, UnboundedReceiver<EngineMsg>, Link)> {
    let journal = Journal::open(&session_dir(&joined.name)?)?;
    let mut reader = journal.reader();
    let records = reader.read()?;
    let (mut engine, rx) = Engine::peer(options, Outbox::new(journal.clone(), &joined.peer));
    engine.replay(&records, !journal.monitor_alive());
    let mut link = Link {
        name: joined.name.clone(),
        peer: joined.peer.clone(),
        journal: journal.clone(),
        reader,
        workspace: joined.workspace.clone(),
        peers: Peers::default(),
        proposals: Proposals::default(),
        notes: Vec::new(),
        breakpoints: None,
        checked: Instant::now(),
        hidden: None,
    };
    let mut seeded = false;
    for record in &records {
        link.see(record);
        seeded |= matches!(record.body, Body::State(State::Breakpoints(_)));
    }
    if seeded {
        link.breakpoints = Some(engine.persisted());
    } else if let Some(saved) = persist::state_file(&joined.workspace).and_then(|file| persist::load(&file).ok())
        && let Ok(value) = serde_json::to_value(saved)
    {
        engine.apply(&State::Breakpoints(value));
    }
    journal.append(&Record::new(&joined.peer, Body::Join { pid: Some(std::process::id()) }))?;
    ensure_monitor(&journal, &joined.name, &joined.workspace);
    Ok((engine, rx, link))
}

/// Starts the session's monitor unless one runs.
pub fn ensure_monitor(journal: &Journal, name: &str, workspace: &Path) {
    if journal.monitor_alive() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let mut command = Command::new(exe);
    command.arg("-C").arg(workspace).arg("monitor").arg(name);
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    match command.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(err) => tracing::warn!("starting the session's monitor: {err}"),
    }
}

impl Link {
    fn see(&mut self, record: &Record) {
        self.peers.see(record);
        self.proposals.see(record);
        if matches!(record.body, Body::Note { .. }) {
            self.notes.push(record.clone());
            if self.notes.len() > NOTES_KEPT {
                self.notes.remove(0);
            }
        }
    }

    fn mine(&self, call: &str) -> bool {
        call.starts_with(&format!("{}-{}-", self.peer, std::process::id()))
    }

    /// Takes in what the journal got since the last turn and records this tuitor's breakpoint
    /// changes; true when the engine changed.
    pub fn sync(&mut self, engine: &mut Engine) -> bool {
        let records = self.reader.read().unwrap_or_else(|err| {
            tracing::warn!("reading the session's journal: {err:#}");
            Vec::new()
        });
        let mut changed = false;
        for record in &records {
            self.see(record);
            let said = match &record.body {
                Body::State(State::Breakpoints(_)) if record.peer == self.peer => None,
                Body::State(state @ State::Breakpoints(_)) => {
                    engine.apply(state);
                    self.breakpoints = Some(engine.persisted());
                    changed = true;
                    None
                }
                Body::State(state) => {
                    engine.apply(state);
                    changed = true;
                    None
                }
                Body::Reply { call, ok: false, text } if self.mine(call) => Some(("warn", text.clone())),
                Body::Decided { call, passed: false } if self.mine(call) => Some(("warn", "voted down".to_owned())),
                Body::Note { text, .. } if record.peer != self.peer => {
                    Some(("info", format!("{}: {text}", record.peer)))
                }
                _ => None,
            };
            if let Some((level, text)) = said {
                engine.apply(&State::Notice { session: None, level: level.into(), text });
                changed = true;
            }
        }
        let now = engine.persisted();
        if self.breakpoints.as_ref() != Some(&now) {
            let record = Record::new(&self.peer, Body::State(State::Breakpoints(now.clone())));
            if let Err(err) = self.journal.append(&record) {
                tracing::warn!("recording the breakpoints: {err:#}");
            }
            self.breakpoints = Some(now);
        }
        if self.checked.elapsed() >= MONITOR_CHECK {
            self.checked = Instant::now();
            ensure_monitor(&self.journal, &self.name, &self.workspace);
        }
        changed
    }

    pub fn leave(&self) {
        let _ = self.journal.append(&Record::new(&self.peer, Body::Leave));
    }

    /// The open call this peer has to answer, else its own that waits, oldest first.
    fn open_prompt(&self) -> Option<Prompt> {
        let attached = self.peers.attached();
        let named = |names: Vec<String>| -> Vec<String> {
            names.into_iter().map(|n| if n == self.peer { "you".to_owned() } else { n }).collect()
        };
        let open = self.proposals.open();
        let theirs = open.iter().find(|p| p.by != self.peer && !p.yes.contains(&self.peer) && p.no.is_none());
        if let Some(p) = theirs {
            let waiting = named(p.waiting_for(&attached));
            return Some(Prompt::Theirs { id: p.id.clone(), by: p.by.clone(), what: p.call.describe(), waiting });
        }
        open.iter().find(|p| p.by == self.peer && p.no.is_none()).map(|p| Prompt::Mine {
            id: p.id.clone(),
            what: p.call.describe(),
            waiting: named(p.waiting_for(&attached)),
        })
    }

    /// What the popup shows now: the open prompt unless `esc` hid it.
    pub fn prompt(&self) -> Option<Prompt> {
        self.open_prompt().filter(|prompt| {
            let id = match prompt {
                Prompt::Theirs { id, .. } | Prompt::Mine { id, .. } => id,
            };
            self.hidden.as_ref() != Some(id)
        })
    }

    fn vote(&self, id: &str, yes: bool) -> Result<()> {
        self.journal.append(&Record::new(&self.peer, Body::Vote { call: id.to_owned(), yes }))
    }

    /// What the status line says when nothing else is: who is in, and what waits for votes.
    pub fn banner(&self) -> String {
        match self.open_prompt() {
            Some(Prompt::Theirs { by, what, .. }) => format!("VOTE: {by} proposes {what} · /vote shows it"),
            Some(Prompt::Mine { what, waiting, .. }) => format!("VOTE: {what} waits for {}", waiting.join(", ")),
            None => {
                let names: Vec<String> = self.peers.attached().into_iter().collect();
                format!("shared {}: {}", self.name, names.join(", "))
            }
        }
    }
}

impl App {
    /// Each turn of the loop: every shared tab takes in the journal.
    pub fn sync_shared(&mut self) {
        let mut changed = false;
        for (_, tab) in self.tabs.iter_mut() {
            if let Some(link) = tab.link.as_mut() {
                changed |= link.sync(&mut tab.engine);
            }
        }
        if changed {
            self.after_engine();
        }
    }

    /// A key while the vote popup shows: `y` and `n` answer another's call, `w` withdraws
    /// one's own, `esc` hides it. Other keys go on as usual; true when the popup took it.
    pub fn vote_key(&mut self, key: &Key) -> bool {
        if self.ui.overlay.is_some() {
            return false;
        }
        let Some(link) = self.tab_mut().link.as_mut() else { return false };
        let Some(prompt) = link.prompt() else { return false };
        let (said, result) = match (&prompt, key) {
            (Prompt::Theirs { id, what, .. }, key) if key.is("y") => (format!("yes to {what}"), link.vote(id, true)),
            (Prompt::Theirs { id, what, .. }, key) if key.is("n") => (format!("no to {what}"), link.vote(id, false)),
            (Prompt::Mine { id, what, .. }, key) if key.is("w") => (format!("withdrew {what}"), link.vote(id, false)),
            (Prompt::Theirs { id, .. } | Prompt::Mine { id, .. }, key) if key.is("esc") => {
                link.hidden = Some(id.clone());
                ("hidden; /vote shows it again".to_owned(), Ok(()))
            }
            _ => return false,
        };
        let outcome = result.map(|()| said).map_err(|e| format!("{e:#}").into());
        self.report(outcome);
        true
    }

    /// `/vote [yes|no]`: answers the call that waits for this peer's vote; without an answer,
    /// shows its popup again.
    pub fn vote(&mut self, answer: Option<String>) -> Outcome {
        let yes = match answer.as_deref().map(str::trim) {
            Some("yes" | "y") => true,
            Some("no" | "n") => false,
            None | Some("") => {
                let link = self.tab_mut().link.as_mut().ok_or("not in a shared session")?;
                link.hidden = None;
                return Ok(if link.open_prompt().is_some() {
                    String::new()
                } else {
                    "nothing waits for a vote".into()
                });
            }
            Some(other) => return Err(format!("/vote yes or /vote no, not `{other}`").into()),
        };
        let link = self.tab().link.as_ref().ok_or("not in a shared session")?;
        let Some(Prompt::Theirs { id, what, .. }) = link.open_prompt() else {
            return Err("nothing waits for your vote".into());
        };
        link.vote(&id, yes).map_err(|e| format!("{e:#}"))?;
        Ok(format!("{} {what}", if yes { "yes to" } else { "no to" }))
    }

    /// `/note ‹text›`: a note every peer sees, at the stop and frame on screen.
    pub fn note(&mut self, text: Option<String>) -> Outcome {
        let text = text.filter(|t| !t.trim().is_empty()).ok_or("/note ‹text›")?;
        let tab = self.tab();
        let at = tab.viewed_frame().map(|(session, stop, frame)| At { session, stop, frame });
        let link = tab.link.as_ref().ok_or("not in a shared session")?;
        let body = Body::Note { text: text.trim().to_owned(), at };
        link.journal.append(&Record::new(&link.peer, body)).map_err(|e| format!("{e:#}"))?;
        Ok("noted".into())
    }

    pub fn shared_banner(&self) -> Option<String> {
        self.tab().link.as_ref().map(Link::banner)
    }
}

/// The vote popup, over the middle of the screen, while a call waits for votes.
pub fn draw_vote(p: &mut Painter<'_>, app: &App, screen: Rect) {
    let Some(prompt) = app.tab().link.as_ref().and_then(Link::prompt) else { return };
    let (title, lines, keys): (&str, Vec<String>, Vec<(&str, &str)>) = match &prompt {
        Prompt::Theirs { by, what, waiting, .. } => (
            "VOTE",
            vec![format!("{by} proposes"), what.clone(), format!("waiting for: {}", waiting.join(", "))],
            vec![("y", "yes"), ("n", "no"), ("esc", "later")],
        ),
        Prompt::Mine { what, waiting, .. } => (
            "YOUR CALL WAITS",
            vec!["you proposed".to_owned(), what.clone(), format!("waiting for: {}", waiting.join(", "))],
            vec![("w", "withdraw"), ("esc", "hide")],
        ),
    };
    let width = 56.min(screen.width.saturating_sub(4));
    let height = 7;
    let rect = Rect::new((screen.width - width) / 2, screen.height.saturating_sub(height) / 2, width, height);
    p.fill(rect, AMBER_BG);
    p.panel(rect, &Panel { title, sub: vec![seg("shared session", fg(DIM))], keys, focus: true, ..Panel::default() });
    let inner = crate::ui::paint::inner(rect);
    let styles = [fg(FG), bold(WHITE), fg(FAINT)];
    for (i, (line, style)) in lines.iter().zip(styles).enumerate() {
        let y = inner.y + 1 + u16::try_from(i).unwrap_or(0);
        p.text(inner.x + 2, y, line, style, inner.right());
    }
}
