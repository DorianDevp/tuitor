//! Application state and the single place where commands take effect.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crossterm::event::KeyEvent;
use tuitor_core::breakpoints::Toggle;
use tuitor_core::config::LaunchConfig;
use tuitor_core::session::SessionId;
use tuitor_core::{Engine, EngineMsg, Level, Step};

use crate::clipboard;
use crate::command::{self, Action, Arg, BpScope, Location, View};
use crate::keymap::{self, Key, Keymap, Lookup};
use crate::model::{BpRow, StackRow};
use crate::overlay::{Finder, Overlay, Palette};
use crate::sources::Sources;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pane {
    Source,
    Locals,
    Stack,
    Watch,
    Breakpoints,
    Output,
    Events,
    History,
}

impl Pane {
    pub fn name(self) -> &'static str {
        match self {
            Pane::Source => "source",
            Pane::Locals => "locals",
            Pane::Stack => "stack",
            Pane::Watch => "watch",
            Pane::Breakpoints => "breakpoints",
            Pane::Output => "output",
            Pane::Events => "events",
            Pane::History => "history",
        }
    }
}

pub fn panes(view: View) -> &'static [Pane] {
    use Pane::*;
    match view {
        View::Overview => &[Source, Locals, Stack, Watch, Breakpoints, Output, Events],
        View::Code => &[Source, Stack, Breakpoints],
        View::History => &[History, Locals],
        View::Breakpoints => &[Breakpoints],
    }
}

/// A selection in a list plus the scroll offset the renderer keeps it visible with.
#[derive(Debug, Default)]
pub struct Cursor {
    pub pos: usize,
    offset: Cell<usize>,
}

impl Cursor {
    pub fn move_by(&mut self, delta: isize, len: usize) {
        if len == 0 {
            self.pos = 0;
            return;
        }
        self.pos = self.pos.saturating_add_signed(delta).min(len - 1);
    }

    pub fn clamp(&mut self, len: usize) {
        self.pos = self.pos.min(len.saturating_sub(1));
    }

    /// First visible row for a window of `height` rows over `len` rows.
    pub fn window(&self, height: usize, len: usize) -> usize {
        let mut offset = self.offset.get().min(len.saturating_sub(height));
        if self.pos < offset {
            offset = self.pos;
        } else if height > 0 && self.pos >= offset + height {
            offset = self.pos + 1 - height;
        }
        self.offset.set(offset);
        offset
    }
}

/// What SOURCE shows: the execution point (follow) or a file the user opened.
#[derive(Debug, Default)]
pub struct SourceState {
    pub browse: Option<PathBuf>,
    /// 1-based line of the cursor.
    pub cursor: u32,
    /// First visible line (1-based), kept by the renderer.
    pub top: Cell<u32>,
    /// Put the cursor at a third of the height on the next render.
    pub recenter: Cell<bool>,
}

#[derive(Debug, Clone)]
pub struct Status {
    pub key: Option<String>,
    pub command: Option<String>,
    pub message: String,
    pub level: Level,
}

pub struct Ui {
    pub view: View,
    focus: HashMap<View, Pane>,
    pub zoom: bool,
    pub session: Option<SessionId>,
    /// Stop number shown per session; absent means the latest (live) stop.
    pub viewing: HashMap<SessionId, u32>,
    pub source: SourceState,
    pub locals: Cursor,
    pub stack: Cursor,
    pub show_vendor: bool,
    pub watch: Cursor,
    pub bps: Cursor,
    pub bp_scope: BpScope,
    /// Lines up from the bottom of OUTPUT; 0 follows new output.
    pub output_back: usize,
    pub events: Cursor,
    pub history_col: usize,
    pub overlay: Option<Overlay>,
    pub status: Option<Status>,
    pub pending: Vec<Key>,
    pub recent: Vec<String>,
    seen_stops: HashMap<SessionId, usize>,
}

pub struct App {
    pub engine: Engine,
    pub keymap: Keymap,
    pub configs: Vec<LaunchConfig>,
    pub ui: Ui,
    pub sources: std::cell::RefCell<Sources>,
    pub quit: bool,
    pub headless: bool,
    files: Option<Vec<String>>,
}

type Outcome = Result<String, String>;

impl App {
    pub fn new(engine: Engine, keymap: Keymap, configs: Vec<LaunchConfig>) -> App {
        App {
            engine,
            keymap,
            configs,
            ui: Ui {
                view: View::Overview,
                focus: HashMap::new(),
                zoom: false,
                session: None,
                viewing: HashMap::new(),
                source: SourceState::default(),
                locals: Cursor::default(),
                stack: Cursor::default(),
                show_vendor: false,
                watch: Cursor::default(),
                bps: Cursor::default(),
                bp_scope: BpScope::Session,
                output_back: 0,
                events: Cursor::default(),
                history_col: 0,
                overlay: None,
                status: None,
                pending: Vec::new(),
                recent: Vec::new(),
                seen_stops: HashMap::new(),
            },
            sources: Default::default(),
            quit: false,
            headless: false,
            files: None,
        }
    }

    pub fn focus(&self) -> Pane {
        self.ui.focus.get(&self.ui.view).copied().unwrap_or(panes(self.ui.view)[0])
    }

    fn set_focus(&mut self, pane: Pane) {
        self.ui.focus.insert(self.ui.view, pane);
    }

    // ---- input ------------------------------------------------------------------------

    pub fn on_key(&mut self, event: KeyEvent) {
        let Some(key) = Key::from_event(event) else { return };
        if self.ui.overlay.is_some() {
            self.overlay_key(key);
            return;
        }
        self.ui.pending.push(key);
        let pending = self.ui.pending.clone();
        let command = match self.keymap.lookup(self.focus().name(), &pending) {
            Lookup::Command(command) => command.to_owned(),
            Lookup::Prefix => return,
            Lookup::None => {
                self.ui.pending.clear();
                return;
            }
        };
        self.ui.pending.clear();
        self.execute(&command, Some(keymap::label(&pending)));
    }

    fn overlay_key(&mut self, key: Key) {
        let Some(overlay) = self.ui.overlay.as_mut() else { return };
        if key.is("esc") || key.is("ctrl-c") {
            self.ui.overlay = None;
            return;
        }
        match overlay {
            Overlay::Palette(palette) => {
                if key.is("enter") {
                    let line = palette.accept();
                    if let Some(line) = line {
                        self.ui.overlay = None;
                        self.execute(&line, None);
                    }
                } else if key.is("tab") {
                    palette.complete();
                } else if key.is("up") || key.is("ctrl-p") {
                    palette.select(-1);
                } else if key.is("down") || key.is("ctrl-n") {
                    palette.select(1);
                } else if key.is("backspace") {
                    if palette.input.is_empty() {
                        self.ui.overlay = None;
                    } else {
                        palette.input.pop();
                        palette.refresh(&self.keymap);
                    }
                } else if let Some(c) = key.text_char() {
                    palette.input.push(c);
                    palette.refresh(&self.keymap);
                }
            }
            Overlay::Finder(finder) => {
                if key.is("enter") {
                    let choice = finder.choice();
                    self.ui.overlay = None;
                    if let Some(choice) = choice {
                        self.open_choice(choice, false);
                    }
                } else if key.is("ctrl-b") {
                    let choice = finder.choice();
                    if let Some(choice) = choice {
                        self.open_choice(choice, true);
                    }
                } else if key.is("up") || key.is("ctrl-p") {
                    finder.select(-1);
                } else if key.is("down") || key.is("ctrl-n") {
                    finder.select(1);
                } else if key.is("backspace") {
                    finder.input.pop();
                    let bps = self.engine.breakpoints();
                    finder.refresh(self.files.as_deref().unwrap_or_default(), bps, self.engine.workspace());
                } else if let Some(c) = key.text_char() {
                    finder.input.push(c);
                    let bps = self.engine.breakpoints();
                    finder.refresh(self.files.as_deref().unwrap_or_default(), bps, self.engine.workspace());
                }
            }
        }
    }

    // ---- engine -----------------------------------------------------------------------

    pub fn on_engine(&mut self, msg: EngineMsg) {
        self.engine.handle(msg);
    }

    /// Runs after a batch of engine messages: follow new stops, surface notices.
    pub fn after_engine(&mut self) {
        for notice in self.engine.take_notices() {
            self.ui.status = Some(Status { key: None, command: None, message: notice.text, level: notice.level });
        }

        let mut newest: Option<SessionId> = None;
        for session in self.engine.sessions() {
            let seen = self.ui.seen_stops.entry(session.id).or_default();
            if session.history.len() > *seen {
                *seen = session.history.len();
                newest = Some(session.id);
            }
        }
        let active_busy = self.active().is_some_and(|s| s.is_stopped() && s.is_alive());
        if self.ui.session.is_none() || self.active().is_none() {
            self.ui.session = self.engine.sessions().iter().rev().find(|s| s.is_alive()).map(|s| s.id);
        }
        if let Some(sid) = newest {
            if !active_busy || self.ui.session == Some(sid) {
                self.ui.session = Some(sid);
            }
            if self.ui.session == Some(sid) {
                self.ui.viewing.remove(&sid);
                self.ui.stack.pos = 0;
                if self.ui.source.browse.is_none() {
                    self.follow_execution();
                }
            }
        }
    }

    // ---- commands ---------------------------------------------------------------------

    /// Runs a command line from a key, the palette or a script.
    pub fn execute(&mut self, line: &str, key: Option<String>) {
        let line = line.trim();
        let shown = format!("/{}", line.trim_start_matches('/'));
        let result = command::parse(line).and_then(|inv| {
            if key.is_none() && !matches!(inv.action, Action::Palette(_) | Action::Help) {
                self.remember(&shown);
            }
            self.apply(inv.action, inv.spec)
        });
        let (message, level) = match result {
            Ok(message) => (message, Level::Info),
            Err(message) => (message, Level::Error),
        };
        let quiet_nav = key.is_some() && level == Level::Info && message.is_empty();
        if !quiet_nav || self.ui.status.is_none() {
            self.ui.status = Some(Status { key, command: Some(shown), message, level });
        }
    }

    /// Shows an error that did not come from a command (startup, configuration).
    pub fn execute_error(&mut self, message: &str) {
        self.ui.status = Some(Status { key: None, command: None, message: message.to_owned(), level: Level::Error });
    }

    fn remember(&mut self, line: &str) {
        self.ui.recent.retain(|l| l != line);
        self.ui.recent.insert(0, line.to_owned());
        self.ui.recent.truncate(8);
    }

    fn apply(&mut self, action: Action, spec: &'static command::Spec) -> Outcome {
        match action {
            Action::Continue => self.step(Step::Continue),
            Action::Next => self.step(Step::Over),
            Action::StepIn => self.step(Step::In),
            Action::StepOut => self.step(Step::Out),
            Action::Pause => {
                let sid = self.require_session()?;
                self.engine.pause(sid).map_err(|e| e.to_string())?;
                Ok("pausing…".into())
            }
            Action::Stop => {
                let sid = self.require_session()?;
                self.engine.stop(sid).map_err(|e| e.to_string())?;
                Ok("stopping…".into())
            }
            Action::Restart => {
                let sid = self.require_session()?;
                let new = self.engine.restart(sid).map_err(|e| e.to_string())?;
                self.ui.session = Some(new);
                Ok("restarting…".into())
            }
            Action::HistoryPrev => self.history_move(-1),
            Action::HistoryNext => self.history_move(1),
            Action::HistoryLive => {
                let sid = self.require_session()?;
                self.ui.viewing.remove(&sid);
                self.follow_execution();
                Ok("live".into())
            }
            Action::BpToggle(location) => self.bp_toggle(location),
            Action::BpCondition(expr) => self.bp_edit(expr, spec, false),
            Action::BpLog(message) => self.bp_edit(message, spec, true),
            Action::BpDelete => {
                let id = self.selected_bp_id().ok_or("no breakpoint selected")?;
                self.engine.delete_breakpoint(id).map_err(|e| e.to_string())?;
                Ok(format!("deleted #{id}"))
            }
            Action::BpEnable => match self.bp_rows().get(self.ui.bps.pos).cloned() {
                Some(BpRow::Breakpoint(id)) => {
                    let mut enabled = false;
                    self.engine
                        .update_breakpoint(id, |b| {
                            b.enabled = !b.enabled;
                            enabled = b.enabled;
                        })
                        .map_err(|e| e.to_string())?;
                    Ok(format!("#{id} {}", if enabled { "enabled" } else { "disabled" }))
                }
                Some(BpRow::Exception { session, filter, label }) => {
                    let on = self.engine.toggle_exception_filter(session, &filter).map_err(|e| e.to_string())?;
                    Ok(format!("{label}: {}", if on { "on" } else { "off" }))
                }
                None => Err("no breakpoint selected".into()),
            },
            Action::BpScope(scope) => {
                self.ui.bp_scope = scope.unwrap_or(match self.ui.bp_scope {
                    BpScope::Session => BpScope::All,
                    BpScope::All => BpScope::Session,
                });
                self.ui.bps.pos = 0;
                Ok(match self.ui.bp_scope {
                    BpScope::Session => "breakpoints of this session".into(),
                    BpScope::All => "breakpoints of all sessions".into(),
                })
            }
            Action::BpClear => {
                let n = self.engine.breakpoints().len();
                self.engine.clear_breakpoints();
                Ok(format!("deleted {n} breakpoints"))
            }
            Action::WatchAdd(expr) => {
                let expr = match expr {
                    Some(expr) => expr,
                    None => match self.selected_local_expr() {
                        Some(expr) if self.focus() == Pane::Locals => expr,
                        _ => return self.prompt(spec),
                    },
                };
                self.engine.add_watch(&expr);
                Ok(format!("watching {expr}"))
            }
            Action::WatchDelete => {
                let expr = self.engine.watches().get(self.ui.watch.pos).cloned().ok_or("no watch selected")?;
                self.engine.remove_watch(&expr);
                self.ui.watch.clamp(self.engine.watches().len());
                Ok(format!("removed {expr}"))
            }
            Action::Eval(expr) => {
                let Some(expr) = expr else { return self.prompt(spec) };
                let sid = self.require_session()?;
                self.engine.evaluate(sid, &expr).map_err(|e| e.to_string())?;
                Ok("evaluating…".into())
            }
            Action::Copy => {
                let text = self.selected_text().ok_or("nothing to copy here")?;
                if !self.headless {
                    clipboard::copy(&text).map_err(|e| e.to_string())?;
                }
                Ok(format!("copied {}", crate::ui::paint::truncate(&text, 60)))
            }
            Action::View(view) => {
                self.ui.view = view;
                self.ui.zoom = false;
                Ok(String::new())
            }
            Action::PaneNext => self.cycle_focus(1),
            Action::PanePrev => self.cycle_focus(-1),
            Action::Zoom => {
                self.ui.zoom = !self.ui.zoom;
                Ok(String::new())
            }
            Action::Close => self.close(),
            Action::Up => self.nav(-1),
            Action::Down => self.nav(1),
            Action::PageUp => self.nav(-15),
            Action::PageDown => self.nav(15),
            Action::Top => self.nav(isize::MIN / 2),
            Action::Bottom => self.nav(isize::MAX / 2),
            Action::Left => self.horizontal(-1),
            Action::Right => self.horizontal(1),
            Action::Open => self.open(),
            Action::Follow => {
                self.ui.source.browse = None;
                self.follow_execution();
                Ok("following execution".into())
            }
            Action::Vendor => {
                self.ui.show_vendor = !self.ui.show_vendor;
                self.ui.stack.pos = 0;
                Ok(if self.ui.show_vendor { "library frames shown" } else { "library frames collapsed" }.into())
            }
            Action::Find(query) => {
                let files = self.files.get_or_insert_with(|| crate::overlay::workspace_files(self.engine.workspace()));
                let mut finder = Finder::new(query.unwrap_or_default());
                finder.refresh(files, self.engine.breakpoints(), self.engine.workspace());
                self.ui.overlay = Some(Overlay::Finder(finder));
                Ok(String::new())
            }
            Action::SessionNext => self.cycle_session(1),
            Action::SessionPrev => self.cycle_session(-1),
            Action::SessionStart(name) => {
                let Some(name) = name else { return self.prompt(spec) };
                let config = self.find_config(&name)?;
                let sid = self.engine.start(config.clone()).map_err(|e| format!("{e:#}"))?;
                self.ui.session = Some(sid);
                Ok(format!("starting {}", config.name))
            }
            Action::ClearOutput => {
                self.engine.clear_output();
                self.ui.output_back = 0;
                Ok(String::new())
            }
            Action::Palette(text) => {
                let mut input = text.unwrap_or_default();
                if let Some((spec, None)) = command::split(&input)
                    && spec.arg != Arg::None
                {
                    input.push(' ');
                }
                self.open_palette(input);
                Ok(String::new())
            }
            Action::Help => {
                self.open_palette(String::new());
                Ok(String::new())
            }
            Action::Quit => {
                self.quit = true;
                Ok("bye".into())
            }
        }
    }

    fn open_palette(&mut self, input: String) {
        let mut palette = Palette::new(input, self.ui.recent.clone());
        palette.refresh(&self.keymap);
        self.ui.overlay = Some(Overlay::Palette(palette));
    }

    /// Opens the palette pre-filled so the user can type the missing argument.
    fn prompt(&mut self, spec: &'static command::Spec) -> Outcome {
        self.open_palette(format!("/{} ", spec.name));
        Ok(String::new())
    }

    fn step(&mut self, step: Step) -> Outcome {
        let sid = self.require_session()?;
        self.ui.viewing.remove(&sid);
        self.engine.resume(sid, step).map_err(|e| e.to_string())?;
        Ok(String::new())
    }

    fn history_move(&mut self, delta: i64) -> Outcome {
        let sid = self.require_session()?;
        let session = self.engine.session(sid).ok_or("no session")?;
        let len = u32::try_from(session.history.len()).unwrap_or(u32::MAX);
        if len == 0 {
            return Err("nothing recorded yet".into());
        }
        let current = i64::from(self.ui.viewing.get(&sid).copied().unwrap_or(len));
        let target = (current + delta).clamp(1, i64::from(len));
        let target = u32::try_from(target).unwrap_or(len);
        let location = session.snapshot(target).and_then(|s| s.top()).map(|f| f.location()).unwrap_or_default();
        if target == len {
            self.ui.viewing.remove(&sid);
        } else {
            self.ui.viewing.insert(sid, target);
        }
        self.ui.stack.pos = 0;
        self.follow_execution();
        Ok(if target == len {
            format!("#{target} {location} · live")
        } else {
            format!("#{target} {location} · recorded")
        })
    }

    fn bp_toggle(&mut self, location: Option<Location>) -> Outcome {
        let (path, line) = match location {
            Some(Location { file: Some(file), line }) => (self.resolve_file(&file)?, line),
            Some(Location { file: None, line }) => (self.source_path().ok_or("no file in SOURCE")?, line),
            None => {
                let path = self.source_path().ok_or("no file in SOURCE")?;
                (path, self.ui.source.cursor.max(1))
            }
        };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(match self.engine.toggle_breakpoint(&path, line) {
            Toggle::Added(id) => format!("added #{id} at {name}:{line}"),
            Toggle::Removed(id) => format!("removed #{id} at {name}:{line}"),
        })
    }

    /// `e` / `l` on a breakpoint: with an argument it sets the value (`-` clears),
    /// without one it opens the palette pre-filled with the current value.
    fn bp_edit(&mut self, value: Option<String>, spec: &'static command::Spec, log: bool) -> Outcome {
        let id = match self.selected_bp_id() {
            Some(id) => id,
            None => {
                let path = self.source_path().ok_or("no breakpoint selected")?;
                let line = self.ui.source.cursor.max(1);
                match self.engine.breakpoints().at(&tuitor_core_canonical(&path), line) {
                    Some(bp) => bp.id,
                    None => match self.engine.toggle_breakpoint(&path, line) {
                        Toggle::Added(id) => id,
                        Toggle::Removed(_) => return Err("no breakpoint selected".into()),
                    },
                }
            }
        };
        let Some(value) = value else {
            let current = self
                .engine
                .breakpoints()
                .get(id)
                .and_then(|b| if log { b.log_message.clone() } else { b.condition.clone() });
            self.open_palette(format!("/{} {}", spec.name, current.unwrap_or_default()));
            return Ok(String::new());
        };
        let cleared = value.trim() == "-";
        let value = (!cleared).then(|| value.clone());
        self.engine
            .update_breakpoint(id, |b| if log { b.log_message = value } else { b.condition = value })
            .map_err(|e| e.to_string())?;
        Ok(match (log, cleared) {
            (false, true) => format!("#{id} condition cleared"),
            (true, true) => format!("#{id} is a breakpoint again"),
            (false, false) => format!("#{id} stops when this holds"),
            (true, false) => format!("#{id} logs instead of stopping"),
        })
    }

    fn cycle_focus(&mut self, delta: isize) -> Outcome {
        let list = panes(self.ui.view);
        let index = list.iter().position(|p| *p == self.focus()).unwrap_or(0);
        let next = (index as isize + delta).rem_euclid(list.len() as isize) as usize;
        self.set_focus(list[next]);
        Ok(String::new())
    }

    fn cycle_session(&mut self, delta: isize) -> Outcome {
        let ids: Vec<SessionId> = self.engine.sessions().iter().map(|s| s.id).collect();
        if ids.is_empty() {
            return Err("no sessions".into());
        }
        let index = self.ui.session.and_then(|sid| ids.iter().position(|i| *i == sid)).unwrap_or(0);
        let next = ids[(index as isize + delta).rem_euclid(ids.len() as isize) as usize];
        self.ui.session = Some(next);
        self.ui.source.browse = None;
        self.follow_execution();
        let name = self.engine.session(next).map(|s| s.name.clone()).unwrap_or_default();
        Ok(name)
    }

    fn close(&mut self) -> Outcome {
        if self.ui.zoom {
            self.ui.zoom = false;
            return Ok(String::new());
        }
        if let Some(sid) = self.ui.session
            && self.ui.viewing.remove(&sid).is_some()
        {
            self.follow_execution();
            return Ok("back to the live stop".into());
        }
        if self.ui.source.browse.take().is_some() {
            self.follow_execution();
            return Ok("following execution".into());
        }
        Ok(String::new())
    }

    fn nav(&mut self, delta: isize) -> Outcome {
        match self.focus() {
            Pane::Source => {
                if self.ui.source.browse.is_none() {
                    self.ui.source.browse = self.source_path();
                }
                let len = self.source_len();
                let line = (self.ui.source.cursor.max(1) as isize).saturating_add(delta).clamp(1, len.max(1) as isize);
                self.ui.source.cursor = line as u32;
            }
            Pane::Locals => {
                let len = self.local_rows().len();
                self.ui.locals.move_by(delta, len);
            }
            Pane::Stack => {
                let len = self.stack_rows().len();
                self.ui.stack.move_by(delta, len);
                self.follow_execution();
            }
            Pane::Watch => {
                let len = self.engine.watches().len();
                self.ui.watch.move_by(delta, len);
            }
            Pane::Breakpoints => {
                let len = self.bp_rows().len();
                self.ui.bps.move_by(delta, len);
            }
            Pane::Output => {
                let len = self.engine.output().len();
                self.ui.output_back = (self.ui.output_back as isize - delta).clamp(0, len as isize) as usize;
            }
            Pane::Events => {
                let len = self.engine.events().len();
                self.ui.events.move_by(delta, len);
            }
            Pane::History => {
                let Some(sid) = self.ui.session else { return Ok(String::new()) };
                let len = self.engine.session(sid).map_or(0, |s| s.history.len());
                if len == 0 {
                    return Ok(String::new());
                }
                let current = self.ui.viewing.get(&sid).map_or(len, |n| *n as usize) as isize;
                let target = (current + delta).clamp(1, len as isize) as usize;
                if target == len {
                    self.ui.viewing.remove(&sid);
                } else {
                    self.ui.viewing.insert(sid, target as u32);
                }
            }
        }
        Ok(String::new())
    }

    fn horizontal(&mut self, delta: isize) -> Outcome {
        match self.focus() {
            Pane::Locals => {
                let rows = self.local_rows();
                let Some(row) = rows.get(self.ui.locals.pos) else { return Ok(String::new()) };
                let (sid, number) = self.viewed_ids().ok_or("nothing to expand")?;
                if delta > 0 {
                    if row.expandable && !row.expanded {
                        self.engine.expand(sid, number, row.path.clone()).map_err(|e| e.to_string())?;
                    }
                } else if row.expanded {
                    self.engine.collapse(sid, &row.path);
                } else if row.path.len() > 1 {
                    let parent = row.path[..row.path.len() - 1].to_vec();
                    if let Some(index) = rows.iter().position(|r| r.path == parent) {
                        self.ui.locals.pos = index;
                    }
                }
            }
            Pane::History => {
                self.ui.history_col = self.ui.history_col.saturating_add_signed(delta);
            }
            _ => {}
        }
        Ok(String::new())
    }

    fn open(&mut self) -> Outcome {
        match self.focus() {
            Pane::Locals => {
                let row = self.local_rows().into_iter().nth(self.ui.locals.pos).ok_or("nothing selected")?;
                let (sid, number) = self.viewed_ids().ok_or("nothing to expand")?;
                if row.expanded {
                    self.engine.collapse(sid, &row.path);
                } else if row.expandable {
                    self.engine.expand(sid, number, row.path).map_err(|e| e.to_string())?;
                }
                Ok(String::new())
            }
            Pane::Stack => {
                if let Some(StackRow::Collapsed { .. }) = self.stack_rows().get(self.ui.stack.pos) {
                    self.ui.show_vendor = true;
                }
                self.ui.source.browse = None;
                self.follow_execution();
                Ok(String::new())
            }
            Pane::Breakpoints => {
                let id = self.selected_bp_id().ok_or("nothing selected")?;
                let bp = self.engine.breakpoints().get(id).ok_or("gone")?;
                let (path, line) = (bp.path.clone(), bp.line);
                self.browse(path, line);
                Ok(String::new())
            }
            Pane::History => {
                self.ui.view = View::Overview;
                self.follow_execution();
                Ok("showing the selected stop".into())
            }
            _ => Ok(String::new()),
        }
    }

    fn open_choice(&mut self, choice: crate::overlay::Choice, toggle: bool) {
        use crate::overlay::Choice;
        let (path, line) = match choice {
            Choice::File { path, line } => (path, line),
            Choice::Breakpoint(id) => match self.engine.breakpoints().get(id) {
                Some(bp) => (bp.path.clone(), Some(bp.line)),
                None => return,
            },
        };
        if toggle {
            let line = line.unwrap_or(1);
            let message = match self.engine.toggle_breakpoint(&path, line) {
                Toggle::Added(id) => format!("added #{id} at line {line}"),
                Toggle::Removed(id) => format!("removed #{id} at line {line}"),
            };
            self.ui.status = Some(Status {
                key: Some("^b".into()),
                command: Some("/bp toggle".into()),
                message,
                level: Level::Info,
            });
            return;
        }
        self.ui.view = View::Code;
        self.browse(path, line.unwrap_or(1));
    }

    fn browse(&mut self, path: PathBuf, line: u32) {
        self.ui.source.browse = Some(path);
        self.ui.source.cursor = line;
        self.ui.source.recenter.set(true);
        if self.ui.view != View::Breakpoints {
            self.set_focus(Pane::Source);
        } else {
            self.ui.view = View::Code;
            self.set_focus(Pane::Source);
        }
    }

    /// Points SOURCE at the selected frame of the viewed stop.
    pub fn follow_execution(&mut self) {
        if self.ui.source.browse.is_some() {
            return;
        }
        if let Some(frame) = self.selected_frame() {
            self.ui.source.cursor = frame.line.max(1);
            self.ui.source.recenter.set(true);
        }
    }

    fn require_session(&self) -> Result<SessionId, String> {
        self.ui.session.filter(|sid| self.engine.session(*sid).is_some()).ok_or_else(|| "no active session".into())
    }

    fn find_config(&self, name: &str) -> Result<LaunchConfig, String> {
        let wanted = name.to_lowercase();
        self.configs
            .iter()
            .find(|c| c.name.to_lowercase() == wanted)
            .or_else(|| self.configs.iter().find(|c| c.name.to_lowercase().contains(&wanted)))
            .cloned()
            .ok_or_else(|| {
                let names: Vec<&str> = self.configs.iter().map(|c| c.name.as_str()).collect();
                format!("no configuration `{name}` (have: {})", names.join(", "))
            })
    }

    fn resolve_file(&self, file: &str) -> Result<PathBuf, String> {
        let path = Path::new(file);
        let path = if path.is_absolute() { path.to_path_buf() } else { self.engine.workspace().join(path) };
        if path.is_file() { Ok(path) } else { Err(format!("no such file: {}", path.display())) }
    }

    fn source_len(&self) -> u32 {
        self.source_path()
            .and_then(|p| self.sources.borrow_mut().get(&p))
            .map_or(0, |f| u32::try_from(f.lines.len()).unwrap_or(u32::MAX))
    }
}

fn tuitor_core_canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
