//! Application state and the single place where commands take effect. What belongs to
//! one project lives in its [`Tab`]; `App` keeps what the tabs share: keys, popups,
//! the status line and the commands run recently.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::KeyEvent;
use tokio::sync::mpsc::UnboundedSender;
use tuitor_core::breakpoints::Toggle;
use tuitor_core::config::{Adapters, LaunchConfig, home_relative};
use tuitor_core::doctor::{Fix, Options, ProjectInput, listen_port};
use tuitor_core::flow::Row;
use tuitor_core::project::{Project, parse_value};
use tuitor_core::session::SessionId;
use tuitor_core::store::Store;
use tuitor_core::{Engine, EngineMsg, Level, Step, persist};

use crate::clipboard;
use crate::command::{self, Action, Arg, BpScope, Location, View};
use crate::doctor::Doctor;
use crate::editor::{Editor, Settings};
use crate::flow::{Item, first_symbol, items};
use crate::keymap::{self, Key, Keymap, Lookup};
use crate::model::{BpRow, ConfigRow, Shown, StackRow};
use crate::overlay::{Finder, Overlay, Palette};
use crate::picker::Picker;
use crate::sources::Sources;
use crate::tab::{Tab, TabId, TabUi, Tabs, forward};

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
    Config,
    Flow,
    Outline,
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
            Pane::Config => "config",
            Pane::Flow => "flow",
            Pane::Outline => "outline",
        }
    }
}

pub fn panes(view: View) -> &'static [Pane] {
    use Pane::*;
    match view {
        View::Overview => &[Source, Locals, Stack, Watch, Breakpoints, Output, Events],
        View::Code => &[Source, Stack, Breakpoints, Flow, Outline],
        View::History => &[History, Locals],
        View::Flow => &[Flow],
        View::Project => &[Breakpoints, Config],
    }
}

#[derive(Debug, Clone)]
pub struct Status {
    pub key: Option<String>,
    pub command: Option<String>,
    pub message: String,
    pub level: Level,
}

/// What every tab shares on screen.
pub struct Ui {
    pub overlay: Option<Overlay>,
    pub status: Option<Status>,
    pub pending: Vec<Key>,
    pub recent: Vec<String>,
    /// Time for animations, set by the loop before each frame. `None` draws them
    /// still, which keeps headless screens the same on every run.
    pub clock: Option<Duration>,
    /// Set by the renderer when the frame holds a moving animation: how soon it
    /// changes next, so the loop redraws only then.
    pub animating: Cell<Option<Duration>>,
    /// A tab with a live session that `tab close` was asked for; asking again closes it.
    closing: Option<TabId>,
}

impl Ui {
    /// Asks for the next frame within `after`; the soonest request wins.
    pub fn animate(&self, after: Duration) {
        self.animating.set(Some(self.animating.get().map_or(after, |due| due.min(after))));
    }
}

pub struct App {
    pub tabs: Tabs<Tab>,
    pub keymap: Keymap,
    pub settings: Settings,
    /// The editor `edit` asked for, for the loop to run: it owns the terminal.
    pub editor: Option<Editor>,
    pub ui: Ui,
    pub sources: RefCell<Sources>,
    pub adapters: Adapters,
    pub store: Store,
    /// Projects opened recently, newest first.
    pub recent_projects: Vec<PathBuf>,
    /// Where the engines of all tabs report, each message labelled with its tab.
    inbox: UnboundedSender<(TabId, EngineMsg)>,
    pub quit: bool,
    pub headless: bool,
}

/// Why a command did not happen: an error, or a warning that asks for something first.
pub(crate) struct Refusal {
    level: Level,
    message: String,
}

impl Refusal {
    fn warn(message: String) -> Refusal {
        Refusal { level: Level::Warn, message }
    }
}

impl From<String> for Refusal {
    fn from(message: String) -> Refusal {
        Refusal { level: Level::Error, message }
    }
}

impl From<&str> for Refusal {
    fn from(message: &str) -> Refusal {
        message.to_owned().into()
    }
}

pub(crate) type Outcome = Result<String, Refusal>;

const NO_PROJECT: &str = "no project in this tab; /tab open opens one";

impl App {
    /// An app with one empty tab; [`App::open`] puts projects in tabs.
    pub fn new(
        keymap: Keymap,
        settings: Settings,
        adapters: Adapters,
        store: Store,
        inbox: UnboundedSender<(TabId, EngineMsg)>,
    ) -> App {
        let recent_projects =
            persist::recent_file().and_then(|file| persist::load_recent(&file).ok()).unwrap_or_default();
        App {
            tabs: Tabs::new(Tab::empty(&adapters)),
            keymap,
            settings,
            editor: None,
            ui: Ui {
                overlay: None,
                status: None,
                pending: Vec::new(),
                recent: Vec::new(),
                clock: None,
                animating: Cell::new(None),
                closing: None,
            },
            sources: Default::default(),
            adapters,
            store,
            recent_projects,
            inbox,
            quit: false,
            headless: false,
        }
    }

    pub fn tab(&self) -> &Tab {
        self.tabs.active()
    }

    pub fn tab_mut(&mut self) -> &mut Tab {
        self.tabs.active_mut()
    }

    pub fn focus(&self) -> Pane {
        self.tab().focus()
    }

    // ---- projects ---------------------------------------------------------------------

    /// Shows `project` in a new tab, or in this one if it is empty, and makes it active. A
    /// project with no configuration at all gets one for its kind, and opens on CONFIG to
    /// adjust it, or to add one when its kind is not known. Returns the tab and what is worth
    /// telling: what could not be read, or the configuration made.
    pub fn open(&mut self, mut project: Project) -> (TabId, Outcome) {
        let dir = project.dir.clone();
        let created = project.create_default(&self.store).map_err(|err| format!("{err:#}"));
        let outcome = match (project.errors.first(), &created) {
            (Some(error), _) | (None, Err(error)) => Err(error.clone().into()),
            (None, Ok(Some(index))) => {
                let config = &project.configurations[*index];
                Ok(format!("made “{}” ({}) for it: adjust it in CONFIG, S starts it", config.name(), config.adapter()))
            }
            (None, Ok(None)) => Ok(String::new()),
        };
        let unconfigured = project.configurations.is_empty();
        let (mut tab, rx) = Tab::open(project, &self.adapters);
        if let Ok(Some(index)) = created {
            tab.ui.config_sel = index;
            tab.ui.config.pos = index;
        }
        if unconfigured || matches!(created, Ok(Some(_))) {
            tab.ui.view = View::Project;
            tab.set_focus(Pane::Config);
        }
        let id = if self.tab().project.is_none() { self.tabs.replace_active(tab).0 } else { self.tabs.push(tab) };
        forward(id, rx, self.inbox.clone());
        match persist::recent_file().map(|file| persist::remember_recent(&file, &dir)) {
            Some(Ok(recent)) => self.recent_projects = recent,
            // The list only saves typing; the project is open either way.
            Some(Err(err)) => tracing::warn!("remembering {}: {err:#}", dir.display()),
            None => {}
        }
        (id, outcome)
    }

    /// Shows what opening a project said, outside a command line; an error stays over
    /// later news.
    pub fn report(&mut self, outcome: Outcome) {
        let (message, level) = match outcome {
            Ok(message) => (message, Level::Info),
            Err(Refusal { level, message }) => (message, level),
        };
        let error_shown = self.ui.status.as_ref().is_some_and(|s| s.level == Level::Error);
        if !message.is_empty() && (level == Level::Error || !error_shown) {
            self.ui.status = Some(Status { key: None, command: None, message, level });
        }
    }

    /// `/tab open`: a directory as typed, `~` and relative to the current project allowed.
    fn open_dir(&mut self, typed: &str) -> Outcome {
        let path = crate::paths::expand(typed, dirs::home_dir().as_deref(), &self.base_dir());
        let dir = std::fs::canonicalize(&path)
            .ok()
            .filter(|dir| dir.is_dir())
            .ok_or_else(|| format!("no directory {}", home_relative(&path)))?;
        let open = self.tabs.iter().find(|(_, tab)| tab.project.as_ref().is_some_and(|p| p.dir == dir));
        if let Some((id, _)) = open {
            self.tabs.activate(id);
            return Ok(format!("{} is open already", home_relative(&dir)));
        }
        match self.open(Project::open(&dir, &self.store, None)).1 {
            Ok(note) if note.is_empty() => Ok(format!("opened {}", home_relative(&dir))),
            outcome => outcome,
        }
    }

    fn open_picker(&mut self) {
        let open = self.tabs.iter().filter_map(|(_, tab)| Some(tab.project.as_ref()?.dir.clone())).collect();
        let picker = Picker::new(self.recent_projects.clone(), open, dirs::home_dir(), self.base_dir());
        self.ui.overlay = Some(Overlay::Picker(picker));
    }

    /// Where a typed relative path starts: the current project, else where tuitor started.
    fn base_dir(&self) -> PathBuf {
        match &self.tab().project {
            Some(project) => project.dir.clone(),
            None => std::env::current_dir().unwrap_or_default(),
        }
    }

    /// Starts `config` in the active tab.
    pub fn start(&mut self, config: LaunchConfig) {
        let name = config.name.clone();
        let tab = self.tab_mut();
        match tab.engine.start(config) {
            Ok(sid) => tab.ui.session = Some(sid),
            Err(err) => self.execute_error(&format!("{name}: {err:#}")),
        }
    }

    /// Disconnects every tab's sessions; used when quitting.
    pub async fn shutdown(&mut self) {
        let engines = self.tabs.iter_mut().map(|(_, tab)| tab.engine.shutdown());
        futures::future::join_all(engines).await;
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
                    let tab = self.tabs.active();
                    finder.refresh(
                        tab.files.as_deref().unwrap_or_default(),
                        tab.engine.breakpoints(),
                        tab.engine.workspace(),
                    );
                } else if let Some(c) = key.text_char() {
                    finder.input.push(c);
                    let tab = self.tabs.active();
                    finder.refresh(
                        tab.files.as_deref().unwrap_or_default(),
                        tab.engine.breakpoints(),
                        tab.engine.workspace(),
                    );
                }
            }
            Overlay::Doctor(doctor) => {
                if key.is("up") || key.is("k") || key.is("ctrl-p") {
                    doctor.select(-1);
                } else if key.is("down") || key.is("j") || key.is("ctrl-n") {
                    doctor.select(1);
                } else if key.is("enter") {
                    self.doctor_fix();
                } else if key.is("q") {
                    self.ui.overlay = None;
                }
            }
            Overlay::Picker(picker) => {
                if key.is("enter") {
                    // When nothing matches, opening what was typed says why it cannot be opened.
                    let dir = picker.choice().map_or_else(|| picker.input.clone(), home_relative);
                    self.ui.overlay = None;
                    self.execute(&format!("tab open {dir}"), None);
                } else if key.is("tab") {
                    picker.complete();
                } else if key.is("up") || key.is("ctrl-p") {
                    picker.select(-1);
                } else if key.is("down") || key.is("ctrl-n") {
                    picker.select(1);
                } else if key.is("backspace") {
                    picker.input.pop();
                    picker.refresh();
                } else if let Some(c) = key.text_char() {
                    picker.input.push(c);
                    picker.refresh();
                }
            }
        }
    }

    /// Takes in what finished in the background: DOCTOR's checks.
    pub fn poll(&mut self) {
        if let Some(Overlay::Doctor(doctor)) = &mut self.ui.overlay {
            doctor.poll();
        }
    }

    // ---- engines ----------------------------------------------------------------------

    /// A message from the engine of tab `tab`; one for a tab closed since is dropped.
    pub fn on_engine(&mut self, tab: TabId, msg: EngineMsg) {
        if let Some(tab) = self.tabs.get_mut(tab) {
            tab.engine.handle(msg);
        }
    }

    /// Runs after a batch of engine messages: follow new stops, surface notices.
    pub fn after_engine(&mut self) {
        let active = self.tabs.active_id();
        for (id, tab) in self.tabs.iter_mut() {
            let name = tab.name();
            for notice in tab.engine.take_notices() {
                // News from another tab says which tab it is from.
                let message = if id == active { notice.text } else { format!("{name}: {}", notice.text) };
                self.ui.status = Some(Status { key: None, command: None, message, level: notice.level });
            }
            tab.follow_stops();
        }
    }

    // ---- commands ---------------------------------------------------------------------

    /// Runs a command line from a key, the palette or a script.
    pub fn execute(&mut self, line: &str, key: Option<String>) {
        let line = line.trim();
        let shown = format!("/{}", line.trim_start_matches('/'));
        let result = command::parse(line).map_err(Refusal::from).and_then(|inv| {
            if key.is_none() && !matches!(inv.action, Action::Palette(_) | Action::Help) {
                self.remember(&shown);
            }
            self.apply(inv.action, inv.spec)
        });
        let (message, level) = match result {
            Ok(message) => (message, Level::Info),
            Err(Refusal { level, message }) => (message, level),
        };
        let quiet_nav = key.is_some() && level == Level::Info && message.is_empty();
        if !quiet_nav || self.ui.status.is_none() {
            self.ui.status = Some(Status { key, command: Some(shown), message, level });
        }
    }

    /// Shows how opening the editor went, once it is known.
    pub fn edited(&mut self, result: Result<String, String>) {
        let (message, level) = match result {
            Ok(message) => (message, Level::Info),
            Err(message) => (message, Level::Error),
        };
        self.ui.status = Some(Status { key: None, command: Some("/edit".into()), message, level });
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
        // Asking to close a tab is answered by the very next command.
        if action != Action::TabClose {
            self.ui.closing = None;
        }
        match action {
            Action::Continue => self.step(Step::Continue),
            Action::Next => self.step(Step::Over),
            Action::StepIn => self.step(Step::In),
            Action::StepOut => self.step(Step::Out),
            Action::Pause => {
                let tab = self.tab_mut();
                let sid = tab.require_session()?;
                tab.engine.pause(sid).map_err(|e| e.to_string())?;
                Ok("pausing…".into())
            }
            Action::Stop => {
                let tab = self.tab_mut();
                let sid = tab.require_session()?;
                tab.engine.stop(sid).map_err(|e| e.to_string())?;
                Ok("stopping…".into())
            }
            Action::Restart => {
                let tab = self.tab_mut();
                let sid = tab.require_session()?;
                let new = tab.engine.restart(sid).map_err(|e| e.to_string())?;
                tab.ui.session = Some(new);
                Ok("restarting…".into())
            }
            Action::HistoryPrev => self.history_move(-1),
            Action::HistoryNext => self.history_move(1),
            Action::HistoryLive => {
                let tab = self.tab_mut();
                let sid = tab.require_session()?;
                tab.ui.viewing.remove(&sid);
                tab.jump_to_execution();
                Ok("live".into())
            }
            Action::BpToggle(location) => self.bp_toggle(location),
            Action::BpCondition(expr) => self.bp_edit(expr, spec, false),
            Action::BpLog(message) => self.bp_edit(message, spec, true),
            Action::BpDelete => {
                let (tab, id) = self.selected_bp().ok_or("no breakpoint selected")?;
                self.engine_of(tab)?.delete_breakpoint(id).map_err(|e| e.to_string())?;
                Ok(format!("deleted #{id}"))
            }
            Action::BpEnable => match self.bp_rows().get(self.tab().ui.bps.pos).cloned() {
                Some(BpRow::Breakpoint { tab, id }) => {
                    let mut enabled = false;
                    self.engine_of(tab)?
                        .update_breakpoint(id, |b| {
                            b.enabled = !b.enabled;
                            enabled = b.enabled;
                        })
                        .map_err(|e| e.to_string())?;
                    Ok(format!("#{id} {}", if enabled { "enabled" } else { "disabled" }))
                }
                Some(BpRow::Exception { tab, session, filter, label }) => {
                    let engine = self.engine_of(tab)?;
                    let on = engine.toggle_exception_filter(session, &filter).map_err(|e| e.to_string())?;
                    Ok(format!("{label}: {}", if on { "on" } else { "off" }))
                }
                Some(BpRow::Tab(_)) | None => Err("no breakpoint selected".into()),
            },
            Action::BpScope(scope) => {
                let ui = &mut self.tab_mut().ui;
                ui.bp_scope = scope.unwrap_or(match ui.bp_scope {
                    BpScope::Session => BpScope::All,
                    BpScope::All => BpScope::Session,
                });
                ui.bps.pos = 0;
                Ok(match ui.bp_scope {
                    BpScope::Session => "breakpoints of this session".into(),
                    BpScope::All => "breakpoints of every session and tab".into(),
                })
            }
            Action::BpClear => {
                let engine = &mut self.tab_mut().engine;
                let n = engine.breakpoints().len();
                engine.clear_breakpoints();
                Ok(format!("deleted {n} breakpoints"))
            }
            Action::WatchAdd(expr) => {
                let tab = self.tab_mut();
                let expr = match expr {
                    Some(expr) => expr,
                    None => match tab.selected_local_expr() {
                        Some(expr) if tab.focus() == Pane::Locals => expr,
                        _ => return self.prompt(spec),
                    },
                };
                tab.engine.add_watch(&expr);
                // One series from stop to stop, so always where execution is.
                let top = if tab.selected_frame_index() > 0 { ", evaluated in the top frame" } else { "" };
                Ok(format!("watching {expr}{top}"))
            }
            Action::WatchDelete => {
                let tab = self.tab_mut();
                let expr = tab.engine.watches().get(tab.ui.watch.pos).cloned().ok_or("no watch selected")?;
                tab.engine.remove_watch(&expr);
                tab.ui.watch.clamp(tab.engine.watches().len());
                Ok(format!("removed {expr}"))
            }
            Action::Eval(expr) => {
                let Some(expr) = expr else { return self.prompt(spec) };
                let tab = self.tab_mut();
                let sid = tab.require_session()?;
                let frame = tab.selected_frame_index();
                tab.engine.evaluate(sid, frame, &expr).map_err(|e| e.to_string())?;
                Ok("evaluating…".into())
            }
            Action::Copy => {
                let text = self.selected_text().ok_or("nothing to copy here")?;
                if !self.headless {
                    clipboard::copy(&text).map_err(|e| e.to_string())?;
                }
                Ok(format!("copied {}", crate::ui::paint::truncate(&text, 60)))
            }
            Action::View(view, pane) => {
                let tab = self.tab_mut();
                tab.ui.view = view;
                tab.ui.zoom = false;
                if let Some(pane) = pane {
                    tab.set_focus(pane);
                }
                Ok(String::new())
            }
            Action::PaneNext => self.cycle_focus(1),
            Action::PanePrev => self.cycle_focus(-1),
            Action::Zoom => {
                let ui = &mut self.tab_mut().ui;
                ui.zoom = !ui.zoom;
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
            Action::Open => self.open_selection(),
            Action::Edit => self.edit(),
            Action::Follow => {
                self.tab_mut().jump_to_execution();
                Ok("following execution".into())
            }
            Action::Vendor if self.focus() == Pane::Flow => {
                let ui = &mut self.tab_mut().ui;
                ui.flow_vendor = !ui.flow_vendor;
                Ok(if ui.flow_vendor { "library files one by one" } else { "library files by package" }.into())
            }
            Action::Vendor => {
                let tab = self.tab_mut();
                tab.ui.show_vendor = !tab.ui.show_vendor;
                tab.ui.stack.pos = 0;
                tab.jump_to_execution();
                Ok(if tab.ui.show_vendor { "library frames shown" } else { "library frames collapsed" }.into())
            }
            Action::Find(query) => {
                let tab = self.tabs.active_mut();
                let workspace = tab.project.as_ref().ok_or(NO_PROJECT)?.dir.clone();
                let files = tab.files.get_or_insert_with(|| crate::overlay::workspace_files(&workspace));
                let mut finder = Finder::new(query.unwrap_or_default());
                finder.refresh(files, tab.engine.breakpoints(), &workspace);
                self.ui.overlay = Some(Overlay::Finder(finder));
                Ok(String::new())
            }
            Action::SessionNext => self.cycle_session(1),
            Action::SessionPrev => self.cycle_session(-1),
            Action::SessionStart(name) => {
                let Some(name) = name else { return self.prompt(spec) };
                let project = self.tab().project.as_ref().ok_or(NO_PROJECT)?;
                let index = project.find(&name).ok_or_else(|| {
                    let names: Vec<&str> = project.configurations.iter().map(|c| c.name()).collect();
                    format!("no configuration `{name}` (have: {})", names.join(", "))
                })?;
                self.start_configuration(index)
            }
            Action::TabNext => self.cycle_tab(1),
            Action::TabPrev => self.cycle_tab(-1),
            Action::TabNew => {
                self.tabs.push(Tab::empty(&self.adapters));
                self.open_picker();
                Ok(String::new())
            }
            Action::TabOpen(Some(dir)) => self.open_dir(&dir),
            Action::TabOpen(None) => {
                self.open_picker();
                Ok(String::new())
            }
            Action::TabClose => self.close_tab(),
            Action::ConfigStart => {
                let index = self.tab().ui.config_sel;
                self.start_configuration(index)
            }
            Action::ConfigSet(arg) => self.config_set(arg.unwrap_or_default().trim()),
            Action::ConfigUnset(field) => self.config_unset(field),
            Action::ConfigMap(Some(mapping)) => self.config_map(&mapping),
            Action::ConfigAdd(Some(adapter)) => self.config_add(&adapter),
            Action::ConfigMap(None) | Action::ConfigAdd(None) => self.prompt(spec),
            Action::ConfigDelete => self.config_delete(),
            Action::ClearOutput => {
                let tab = self.tab_mut();
                tab.engine.clear_output();
                tab.ui.output_back = 0;
                Ok(String::new())
            }
            Action::Doctor(only) => {
                self.open_doctor(only);
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

    /// DOCTOR, for every adapter, or the one named, and the projects of every tab.
    fn open_doctor(&mut self, only: Option<String>) {
        let projects = self.tabs.iter().filter_map(|(_, tab)| tab.project.as_ref().map(ProjectInput::of)).collect();
        // Ports this tuitor listens on are not in the way of its sessions.
        let sessions = self.tabs.iter().flat_map(|(_, tab)| tab.engine.sessions());
        let own_ports = sessions.filter(|s| s.is_alive()).filter_map(|s| listen_port(&s.config)).collect();
        let only = only.iter().flat_map(|names| names.split_whitespace()).map(str::to_owned).collect();
        let options = Options { only, php: None, own_ports };
        self.ui.overlay = Some(Overlay::Doctor(Doctor::start(self.adapters.clone(), projects, options)));
    }

    /// `enter` in DOCTOR: a tuitor command goes to the command line, to be checked and run, with
    /// the configuration it is for selected; a shell command is copied, as tuitor runs none.
    fn doctor_fix(&mut self) {
        let Some(Overlay::Doctor(doctor)) = &self.ui.overlay else { return };
        let Some((section, finding)) = doctor.selection() else { return };
        let (dir, subject) = (section.dir.clone(), finding.subject.clone());
        let message = match finding.fix.clone() {
            Some(Fix::Command(line)) => {
                self.ui.overlay = None;
                self.select_configuration(dir, &subject);
                self.open_palette(format!("/{line}"));
                return;
            }
            Some(Fix::Shell(command)) => {
                let copied = if self.headless { Ok(()) } else { clipboard::copy(&command) };
                match copied {
                    Ok(()) => format!("copied `{command}`; tuitor runs no shell commands"),
                    Err(err) => err.to_string(),
                }
            }
            Some(Fix::Text(_)) | None => return,
        };
        self.ui.status = Some(Status { key: None, command: Some("/doctor".into()), message, level: Level::Info });
    }

    /// Shows configuration `name` of the project in `dir` in CONFIG, selected.
    fn select_configuration(&mut self, dir: Option<PathBuf>, name: &str) {
        let tab = self.tabs.iter().find(|(_, tab)| tab.project.as_ref().is_some_and(|p| Some(&p.dir) == dir.as_ref()));
        let Some(id) = tab.map(|(id, _)| id) else { return };
        self.tabs.activate(id);
        let tab = self.tab_mut();
        if let Some(index) = tab.project.as_ref().and_then(|p| p.find(name)) {
            tab.ui.config_sel = index;
            tab.ui.config.pos = index;
        }
        tab.ui.view = View::Project;
        tab.set_focus(Pane::Config);
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

    fn engine_of(&mut self, tab: TabId) -> Result<&mut Engine, Refusal> {
        self.tabs.get_mut(tab).map(|tab| &mut tab.engine).ok_or_else(|| "that tab is closed".into())
    }

    fn step(&mut self, step: Step) -> Outcome {
        let tab = self.tab_mut();
        let sid = tab.require_session()?;
        tab.ui.viewing.remove(&sid);
        tab.engine.resume(sid, step).map_err(|e| e.to_string())?;
        Ok(String::new())
    }

    /// Starts configuration `index` of this tab's project.
    fn start_configuration(&mut self, index: usize) -> Outcome {
        let Tab { project, engine, ui, .. } = self.tab_mut();
        let project = project.as_ref().ok_or(NO_PROJECT)?;
        let config = project.configurations.get(index).ok_or("no configuration; /config add ‹type› adds one")?;
        let config = config.launch(&project.variables()).map_err(|e| format!("{e:#}"))?;
        let started = format!("starting {}", config.name);
        ui.session = Some(engine.start(config).map_err(|e| format!("{e:#}"))?);
        Ok(started)
    }

    // ---- configurations: tuitor's own are edited here and saved at once ------------------

    /// What changing a configuration of this tab's project takes: the project, the view
    /// state that says which configuration is selected, and the store to save it in.
    fn selected_project(&mut self) -> Result<(&mut Project, &mut TabUi, &Store), Refusal> {
        let App { tabs, store, .. } = self;
        let Tab { project, ui, .. } = tabs.active_mut();
        Ok((project.as_mut().ok_or(NO_PROJECT)?, ui, store))
    }

    /// `/config set field value`; without a value the palette offers the current one to edit,
    /// without a field the one under the CONFIG cursor.
    fn config_set(&mut self, arg: &str) -> Outcome {
        let (field, value) = arg.split_once(char::is_whitespace).unwrap_or((arg, ""));
        let field = match (field, self.tab().config_row()) {
            ("", Some(ConfigRow::Field(field))) => field,
            ("", _) => {
                self.open_palette("/config set ".into());
                return Ok(String::new());
            }
            (field, _) => field.to_owned(),
        };
        let value = value.trim();
        if value.is_empty() {
            let current = self.tab().config_text(&ConfigRow::Field(field.clone())).unwrap_or_default();
            self.open_palette(format!("/config set {field} {current}"));
            return Ok(String::new());
        }
        let (project, ui, store) = self.selected_project()?;
        project.set(store, ui.config_sel, &field, parse_value(value)).map_err(|e| format!("{e:#}"))?;
        Ok(format!("{field} = {value}"))
    }

    fn config_unset(&mut self, field: Option<String>) -> Outcome {
        let field = match (field, self.tab().config_row()) {
            (Some(field), _) | (None, Some(ConfigRow::Field(field))) => field,
            (None, Some(ConfigRow::Mapping(remote))) => return self.config_map(&format!("{remote}=")),
            (None, _) => return Err("no field selected; /config unset ‹field›".into()),
        };
        let (project, ui, store) = self.selected_project()?;
        project.unset(store, ui.config_sel, &field).map_err(|e| format!("{e:#}"))?;
        let rows = self.tab().config_rows().len();
        self.tab_mut().ui.config.clamp(rows);
        Ok(format!("removed {field}"))
    }

    /// `remote=local`; an empty `local` removes the mapping.
    fn config_map(&mut self, mapping: &str) -> Outcome {
        let (remote, local) =
            mapping.split_once('=').ok_or("expected remote=local, e.g. /config map /var/www/app=.")?;
        let (remote, local) = (remote.trim(), local.trim());
        let (project, ui, store) = self.selected_project()?;
        project.map(store, ui.config_sel, remote, local).map_err(|e| format!("{e:#}"))?;
        let rows = self.tab().config_rows().len();
        self.tab_mut().ui.config.clamp(rows);
        Ok(if local.is_empty() { format!("{remote} is no longer mapped") } else { format!("{remote} → {local}") })
    }

    fn config_add(&mut self, adapter: &str) -> Outcome {
        let (project, ui, store) = self.selected_project()?;
        let index = project.add(store, adapter).map_err(|e| format!("{e:#}"))?;
        ui.config_sel = index;
        ui.config.pos = index;
        let config = &project.configurations[index];
        let (name, adapter) = (config.name().to_owned(), config.adapter().to_owned());
        if self.adapters.get(&adapter).is_none() {
            return Ok(format!("added “{name}”; tuitor has no `{adapter}` adapter yet (tuitor configs lists them)"));
        }
        Ok(format!("added “{name}” ({adapter}); adjust it here, S starts it"))
    }

    fn config_delete(&mut self) -> Outcome {
        let (project, ui, store) = self.selected_project()?;
        let removed = project.delete(store, ui.config_sel).map_err(|e| format!("{e:#}"))?;
        ui.config_sel = ui.config_sel.min(project.configurations.len().saturating_sub(1));
        ui.config.pos = ui.config_sel;
        Ok(format!("deleted “{}”", removed.name()))
    }

    fn history_move(&mut self, delta: i64) -> Outcome {
        let tab = self.tab_mut();
        let sid = tab.require_session()?;
        let session = tab.engine.session(sid).ok_or("no session")?;
        let len = u32::try_from(session.history.len()).unwrap_or(u32::MAX);
        if len == 0 {
            return Err("nothing recorded yet".into());
        }
        let current = i64::from(tab.ui.viewing.get(&sid).copied().unwrap_or(len));
        let target = (current + delta).clamp(1, i64::from(len));
        let target = u32::try_from(target).unwrap_or(len);
        let location = session.snapshot(target).and_then(|s| s.top()).map(|f| f.location()).unwrap_or_default();
        if target == len {
            tab.ui.viewing.remove(&sid);
        } else {
            tab.ui.viewing.insert(sid, target);
        }
        tab.ui.stack.pos = 0;
        tab.jump_to_execution();
        Ok(if target == len {
            format!("#{target} {location} · live")
        } else {
            format!("#{target} {location} · recorded")
        })
    }

    fn bp_toggle(&mut self, location: Option<Location>) -> Outcome {
        let tab = self.tab();
        let (path, line) = match location {
            Some(Location { file: Some(file), line }) => (self.resolve_file(&file)?, line),
            Some(Location { file: None, line }) => (self.breakpoint_file()?, line),
            None if tab.focus() == Pane::Flow => {
                let row = self.flow_file()?;
                let line = self.flow_entry(&row);
                (row.path, line)
            }
            None if tab.focus() == Pane::Outline => {
                let path = tab.source_path().ok_or("no file in SOURCE")?;
                let source = self.sources.borrow_mut().get(&path).ok_or("cannot read the file")?;
                let symbol = source.outline().get(tab.ui.outline.pos).ok_or("no function selected")?;
                (path, crate::outline::body_start(&source.lines, source.lang(), symbol))
            }
            None => (self.breakpoint_file()?, tab.ui.source.cursor.max(1)),
        };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(match self.tab_mut().engine.toggle_breakpoint(&path, line) {
            Toggle::Added(id) => format!("added #{id} at {name}:{line}"),
            Toggle::Removed(id) => format!("removed #{id} at {name}:{line}"),
        })
    }

    /// `e` / `l` on a breakpoint: with an argument it sets the value (`-` clears),
    /// without one it opens the palette pre-filled with the current value.
    fn bp_edit(&mut self, value: Option<String>, spec: &'static command::Spec, log: bool) -> Outcome {
        let (tab, id) = match self.selected_bp() {
            Some(selected) => selected,
            None => {
                let path = match self.breakpoint_file() {
                    Ok(path) => path,
                    Err(_) if self.tab().shown_source().is_none() => return Err("no breakpoint selected".into()),
                    Err(refusal) => return Err(refusal),
                };
                let tab = self.tab_mut();
                let line = tab.ui.source.cursor.max(1);
                let id = match tab.engine.breakpoints().at(&canonical(&path), line) {
                    Some(bp) => bp.id,
                    None => match tab.engine.toggle_breakpoint(&path, line) {
                        Toggle::Added(id) => id,
                        Toggle::Removed(_) => return Err("no breakpoint selected".into()),
                    },
                };
                (self.tabs.active_id(), id)
            }
        };
        let engine = self.engine_of(tab)?;
        let Some(value) = value else {
            let current = engine
                .breakpoints()
                .get(id)
                .and_then(|b| if log { b.log_message.clone() } else { b.condition.clone() });
            self.open_palette(format!("/{} {}", spec.name, current.unwrap_or_default()));
            return Ok(String::new());
        };
        let cleared = value.trim() == "-";
        let value = (!cleared).then(|| value.clone());
        engine
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
        let tab = self.tab_mut();
        let list = panes(tab.ui.view);
        let index = list.iter().position(|p| *p == tab.focus()).unwrap_or(0);
        let next = (index as isize + delta).rem_euclid(list.len() as isize) as usize;
        tab.set_focus(list[next]);
        Ok(String::new())
    }

    /// Moves to another session; a parent and the children it runs its program in are one.
    fn cycle_session(&mut self, delta: isize) -> Outcome {
        let tab = self.tab_mut();
        let ids: Vec<SessionId> = tab.sessions().map(|s| s.id).collect();
        if ids.is_empty() {
            return Err("no sessions".into());
        }
        let current = tab.ui.session.map(|sid| tab.engine.root(sid));
        let index = current.and_then(|root| ids.iter().position(|i| *i == root)).unwrap_or(0);
        let next = ids[(index as isize + delta).rem_euclid(ids.len() as isize) as usize];
        tab.ui.session = Some(tab.shown(next));
        tab.jump_to_execution();
        Ok(tab.engine.session(next).map(|s| s.name.clone()).unwrap_or_default())
    }

    fn cycle_tab(&mut self, delta: isize) -> Outcome {
        if self.tabs.len() == 1 {
            return Err("there is one tab; /tab open opens a project in another".into());
        }
        self.tabs.cycle(delta);
        Ok(self.tab().name())
    }

    /// Closes the active tab, stopping its debuggers; one with a live session is closed
    /// only when asked twice in a row.
    fn close_tab(&mut self) -> Outcome {
        let id = self.tabs.active_id();
        let name = self.tab().name();
        let live = self.tab().live_sessions();
        if live > 0 && self.ui.closing != Some(id) {
            self.ui.closing = Some(id);
            let again = self.keymap.key_for(self.focus().name(), "tab close").unwrap_or_else(|| "/tab close".into());
            let (sessions, them) =
                if live == 1 { ("a live session".into(), "it") } else { (format!("{live} live sessions"), "them") };
            return Err(Refusal::warn(format!("{name} has {sessions}: {again} again stops {them} and closes the tab")));
        }
        self.ui.closing = None;
        let adapters = self.adapters.clone();
        if let Some(mut closed) = self.tabs.close(id, || Tab::empty(&adapters)) {
            tokio::spawn(async move { closed.engine.shutdown().await });
        }
        Ok(format!("closed {name}"))
    }

    fn close(&mut self) -> Outcome {
        let tab = self.tab_mut();
        if tab.ui.zoom {
            tab.ui.zoom = false;
            return Ok(String::new());
        }
        if let Some(sid) = tab.ui.session
            && tab.ui.viewing.remove(&sid).is_some()
        {
            tab.jump_to_execution();
            return Ok("back to the live stop".into());
        }
        if tab.ui.source.browse.is_some() {
            tab.jump_to_execution();
            return Ok("following execution".into());
        }
        Ok(String::new())
    }

    fn nav(&mut self, delta: isize) -> Outcome {
        match self.focus() {
            Pane::Source => {
                let len = self.source_len();
                let tab = self.tab_mut();
                if tab.ui.source.browse.is_none() {
                    tab.ui.source.browse = tab.source_path();
                }
                let line = (tab.ui.source.cursor.max(1) as isize).saturating_add(delta).clamp(1, len.max(1) as isize);
                tab.ui.source.cursor = line as u32;
            }
            Pane::Breakpoints => {
                let len = self.bp_rows().len();
                self.tab_mut().ui.bps.move_by(delta, len);
            }
            Pane::Locals => {
                let tab = self.tab_mut();
                let len = tab.local_rows().len();
                tab.ui.locals.move_by(delta, len);
            }
            Pane::Stack => {
                let tab = self.tab_mut();
                let len = tab.stack_rows().len();
                tab.ui.stack.move_by(delta, len);
                tab.jump_to_execution();
            }
            Pane::Watch => {
                let tab = self.tab_mut();
                let len = tab.engine.watches().len();
                tab.ui.watch.move_by(delta, len);
            }
            Pane::Output => {
                let tab = self.tab_mut();
                let len = tab.engine.output().len();
                tab.ui.output_back = (tab.ui.output_back as isize - delta).clamp(0, len as isize) as usize;
            }
            Pane::Events => {
                let tab = self.tab_mut();
                let len = tab.engine.events().len();
                tab.ui.events.move_by(delta, len);
            }
            Pane::Flow => self.flow_move(delta),
            Pane::Outline => {
                let path = self.tab().source_path();
                let len = path.and_then(|p| self.sources.borrow_mut().get(&p)).map_or(0, |f| f.outline().len());
                self.tab_mut().ui.outline.move_by(delta, len);
            }
            Pane::Config => {
                let tab = self.tab_mut();
                let rows = tab.config_rows();
                tab.ui.config.move_by(delta, rows.len());
                // The configuration under the cursor is the one shown and started.
                if let Some(ConfigRow::Configuration(index)) = rows.get(tab.ui.config.pos) {
                    tab.ui.config_sel = *index;
                }
            }
            Pane::History => {
                let tab = self.tab_mut();
                let Some(sid) = tab.ui.session else { return Ok(String::new()) };
                let len = tab.engine.session(sid).map_or(0, |s| s.history.len());
                if len == 0 {
                    return Ok(String::new());
                }
                let current = tab.ui.viewing.get(&sid).map_or(len, |n| *n as usize) as isize;
                let target = (current + delta).clamp(1, len as isize) as usize;
                if target == len {
                    tab.ui.viewing.remove(&sid);
                } else {
                    tab.ui.viewing.insert(sid, target as u32);
                }
                tab.ui.stack.pos = 0;
                tab.jump_to_execution();
            }
        }
        Ok(String::new())
    }

    fn horizontal(&mut self, delta: isize) -> Outcome {
        let tab = self.tab_mut();
        match tab.focus() {
            Pane::Locals => {
                let rows = tab.local_rows();
                let Some(row) = rows.get(tab.ui.locals.pos) else { return Ok(String::new()) };
                let (sid, number, frame) = tab.viewed_frame().ok_or("nothing to expand")?;
                if delta > 0 {
                    if row.expandable && !row.expanded {
                        tab.engine.expand(sid, number, frame, row.path.clone()).map_err(|e| e.to_string())?;
                    }
                } else if row.expanded {
                    tab.engine.collapse(sid, &row.path);
                } else if row.path.len() > 1 {
                    let parent = row.path[..row.path.len() - 1].to_vec();
                    if let Some(index) = rows.iter().position(|r| r.path == parent) {
                        tab.ui.locals.pos = index;
                    }
                }
            }
            Pane::History => {
                tab.ui.history_col = tab.ui.history_col.saturating_add_signed(delta);
            }
            _ => {}
        }
        Ok(String::new())
    }

    /// `enter`: what it means depends on the focused pane.
    fn open_selection(&mut self) -> Outcome {
        match self.focus() {
            Pane::Locals => {
                let tab = self.tab_mut();
                let row = tab.local_rows().into_iter().nth(tab.ui.locals.pos).ok_or("nothing selected")?;
                let (sid, number, frame) = tab.viewed_frame().ok_or("nothing to expand")?;
                if row.expanded {
                    tab.engine.collapse(sid, &row.path);
                } else if row.expandable {
                    tab.engine.expand(sid, number, frame, row.path).map_err(|e| e.to_string())?;
                }
                Ok(String::new())
            }
            Pane::Stack => {
                let tab = self.tab_mut();
                if let Some(StackRow::Collapsed { .. }) = tab.stack_rows().get(tab.ui.stack.pos) {
                    tab.ui.show_vendor = true;
                }
                tab.jump_to_execution();
                Ok(String::new())
            }
            Pane::Breakpoints => match self.bp_rows().get(self.tab().ui.bps.pos).cloned() {
                Some(BpRow::Tab(id)) => {
                    self.tabs.activate(id);
                    Ok(self.tab().name())
                }
                Some(BpRow::Breakpoint { tab, id }) => {
                    let bp = self.tabs.get(tab).and_then(|t| t.engine.breakpoints().get(id)).ok_or("gone")?;
                    let (path, line) = (bp.path.clone(), bp.line);
                    // A breakpoint of another tab opens in its own tab.
                    self.tabs.activate(tab);
                    self.tab_mut().browse(path, line);
                    Ok(String::new())
                }
                Some(BpRow::Exception { .. }) | None => Err("nothing selected".into()),
            },
            Pane::History => {
                let tab = self.tab_mut();
                tab.ui.view = View::Overview;
                tab.jump_to_execution();
                Ok("showing the selected stop".into())
            }
            Pane::Flow => self.flow_open(),
            Pane::Outline => {
                let tab = self.tab();
                let path = tab.source_path().ok_or("no file in SOURCE")?;
                let source = self.sources.borrow_mut().get(&path).ok_or("cannot read the file")?;
                let symbol = source.outline().get(tab.ui.outline.pos).cloned().ok_or("no function selected")?;
                self.tab_mut().browse(path, symbol.line);
                Ok(String::new())
            }
            // A configuration starts; a field or mapping goes to the palette to be edited.
            Pane::Config => match self.tab().config_row() {
                Some(ConfigRow::Configuration(index)) => self.start_configuration(index),
                Some(ConfigRow::Field(field)) => self.config_set(&field),
                Some(row @ ConfigRow::Mapping(_)) => {
                    let mapping = self.tab().config_text(&row).unwrap_or_default();
                    self.open_palette(format!("/config map {mapping}"));
                    Ok(String::new())
                }
                None => Ok(String::new()),
            },
            _ => Ok(String::new()),
        }
    }

    /// `edit`: the selected breakpoint, or the SOURCE line, for the loop to open in the editor.
    fn edit(&mut self) -> Outcome {
        let (path, line) = match self.selected_bp() {
            Some((tab, id)) => {
                let bp = self.tabs.get(tab).and_then(|t| t.engine.breakpoints().get(id)).ok_or("gone")?;
                (bp.path.clone(), bp.line)
            }
            None => match self.tab().shown_source() {
                Some(Shown::File(path)) => (path, self.tab().ui.source.cursor.max(1)),
                Some(Shown::Adapter { name, .. }) => {
                    return Err(Refusal::warn(format!("{name} is the adapter's own source, with no file to edit")));
                }
                None => return Err("no file in SOURCE".into()),
            },
        };
        if !path.is_file() {
            return Err(format!("not on this machine: {}", path.display()).into());
        }
        let editor = Editor::new(&self.settings, |name| std::env::var(name).ok(), &path, line)?;
        let message = format!("opening {} in {}", editor.place, editor.name());
        self.editor = Some(editor);
        Ok(message)
    }

    fn open_choice(&mut self, choice: crate::overlay::Choice, toggle: bool) {
        use crate::overlay::Choice;
        let tab = self.tab_mut();
        let (path, line) = match choice {
            Choice::File { path, line } => (path, line),
            Choice::Breakpoint(id) => match tab.engine.breakpoints().get(id) {
                Some(bp) => (bp.path.clone(), Some(bp.line)),
                None => return,
            },
        };
        if toggle {
            let line = line.unwrap_or(1);
            let message = match tab.engine.toggle_breakpoint(&path, line) {
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
        tab.ui.view = View::Code;
        tab.browse(path, line.unwrap_or(1));
    }

    /// The file in SOURCE, to set a breakpoint in. Adapters take breakpoints by path, so a
    /// source the adapter has itself takes none.
    fn breakpoint_file(&self) -> Result<PathBuf, Refusal> {
        match self.tab().shown_source() {
            Some(Shown::File(path)) => Ok(path),
            Some(Shown::Adapter { name, .. }) => {
                Err(Refusal::warn(format!("breakpoints go by file, and {name} is the adapter's own source")))
            }
            None => Err("no file in SOURCE".into()),
        }
    }

    /// The file under the FLOW cursor; for a line of library files, the first of them.
    pub fn flow_row(&self) -> Option<Row> {
        let tab = self.tab();
        let view = tab.flow()?;
        let items = items(&view.flow, tab.ui.flow_vendor);
        let item = items.get(tab.flow_cursor(&items))?;
        Some(view.flow.rows[*item.rows().first()?].clone())
    }

    /// The file under the FLOW cursor, when it is one here: js-debug names Node's internals
    /// `<node_internals>/…`, which only the adapter has.
    fn flow_file(&self) -> Result<Row, Refusal> {
        let row = self.flow_row().ok_or("no file selected in FLOW")?;
        if !row.path.is_file() {
            return Err(format!("{} is no file here; STACK shows the adapter's source", row.path.display()).into());
        }
        Ok(row)
    }

    /// Where a breakpoint on entering the file of `row` goes: the first line of the body of
    /// the first function seen in it, as OUTLINE finds it, else the first line reached.
    pub fn flow_entry(&self, row: &Row) -> u32 {
        let source = self.sources.borrow_mut().get(&row.path);
        let entry = source.as_ref().and_then(|source| {
            let symbol = first_symbol(row, source.outline())?;
            Some(crate::outline::body_start(&source.lines, source.lang(), symbol))
        });
        entry.or_else(|| row.lines.keys().next().copied()).unwrap_or(1)
    }

    /// Moves FLOW's cursor; in CODE, SOURCE shows the file it lands on.
    fn flow_move(&mut self, delta: isize) {
        let tab = self.tab();
        let Some(view) = tab.flow() else { return };
        let items = items(&view.flow, tab.ui.flow_vendor);
        let from = tab.flow_cursor(&items);
        let ui = &mut self.tab_mut().ui;
        ui.flow_follow = false;
        ui.flow.pos = from;
        ui.flow.move_by(delta, items.len());
        if ui.view == View::Code
            && let Some(row) = self.flow_row()
        {
            let line = row.stack_line.unwrap_or_else(|| self.flow_entry(&row));
            let source = &mut self.tab_mut().ui.source;
            source.browse = Some(row.path);
            source.cursor = line;
            source.recenter.set(true);
        }
    }

    /// `enter` in FLOW: a line of library files opens up; a file opens in CODE where the
    /// stack is in it, else where it is entered.
    fn flow_open(&mut self) -> Outcome {
        let tab = self.tab();
        let view = tab.flow().ok_or("nothing in FLOW yet")?;
        let items = items(&view.flow, tab.ui.flow_vendor);
        match items.get(tab.flow_cursor(&items)) {
            Some(Item::Library { rows, .. }) => {
                let first = rows[0];
                let ui = &mut self.tab_mut().ui;
                ui.flow_vendor = true;
                ui.flow_follow = false;
                // Every file is a line of its own now, so the row is the line.
                ui.flow.pos = first;
                Ok("library files one by one".into())
            }
            Some(Item::File(_)) => {
                let row = self.flow_file()?;
                let line = row.stack_line.unwrap_or_else(|| self.flow_entry(&row));
                let tab = self.tab_mut();
                tab.ui.view = View::Code;
                tab.ui.zoom = false;
                tab.browse(row.path, line);
                Ok(String::new())
            }
            None => Err("nothing selected".into()),
        }
    }

    fn resolve_file(&self, file: &str) -> Result<PathBuf, String> {
        let path = Path::new(file);
        let path = if path.is_absolute() { path.to_path_buf() } else { self.tab().engine.workspace().join(path) };
        if path.is_file() { Ok(path) } else { Err(format!("no such file: {}", path.display())) }
    }

    fn source_len(&self) -> u32 {
        self.shown_lines().map_or(0, |f| u32::try_from(f.lines.len()).unwrap_or(u32::MAX))
    }
}

impl Tab {
    pub fn require_session(&self) -> Result<SessionId, String> {
        self.ui.session.filter(|sid| self.engine.session(*sid).is_some()).ok_or_else(|| "no active session".into())
    }

    /// Points SOURCE at the selected frame of the viewed stop, with the cursor on its line
    /// and that line centred, and has the frame's variables read for LOCALS, and its source
    /// if the adapter has it, if they were not. Every automatic change of the view calls
    /// this (a new stop, another recorded stop, frame or session, `.`), so it drops whatever
    /// the user was browsing.
    pub fn jump_to_execution(&mut self) {
        self.ui.source.browse = None;
        if let Some(frame) = self.selected_frame() {
            self.ui.source.cursor = frame.line.max(1);
            self.ui.source.recenter.set(true);
        }
        if let Some((sid, number, frame)) = self.viewed_frame() {
            self.engine.read_frame(sid, number, frame);
        }
        if let Some(Shown::Adapter { session, reference, .. }) = self.shown_source() {
            self.engine.read_source(session, reference);
        }
    }

    /// Shows `path` at `line` in SOURCE, in the CODE view when PROJECT, which has no SOURCE,
    /// is on screen.
    pub fn browse(&mut self, path: PathBuf, line: u32) {
        self.ui.source.browse = Some(path);
        self.ui.source.cursor = line;
        self.ui.source.recenter.set(true);
        if self.ui.view == View::Project {
            self.ui.view = View::Code;
        }
        self.set_focus(Pane::Source);
    }

    /// Follows new stops: the session that stopped becomes active unless the active one is
    /// stopped too, and SOURCE goes to where it stopped. A session started from a
    /// configuration gives way to the child its adapter runs the program in, once there is one.
    pub fn follow_stops(&mut self) {
        self.flows.record(self.engine.sessions());
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
                self.ui.flow_follow = true;
                self.jump_to_execution();
            }
        }
        // The part that runs the program is shown. Its stops were followed above, so SOURCE
        // has nothing new to go to.
        self.ui.session = self.ui.session.map(|sid| self.shown(sid));
    }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
