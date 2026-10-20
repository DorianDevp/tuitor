//! Read-only views of engine state shaped for the panes: rows of the variable
//! tree, stack with library frames grouped, breakpoints in scope. What one project
//! shows comes from its [`Tab`]; breakpoints can be listed across tabs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde_json::Value;
use tuitor_core::breakpoints::BreakpointId;
use tuitor_core::project::{Configuration, value_text};
use tuitor_core::session::{Session, SessionId, SourceText};
use tuitor_core::snapshot::{Change, Frame, Snapshot, Var, VarPath, diff};

use crate::app::{App, Pane};
use crate::command::BpScope;
use crate::sources::SourceFile;
use crate::tab::{Tab, TabId};

#[derive(Debug, Clone)]
pub struct LocalRow {
    pub depth: usize,
    pub path: VarPath,
    pub name: String,
    pub type_name: String,
    pub value: String,
    pub evaluate_name: Option<String>,
    pub scope: bool,
    pub expandable: bool,
    pub expanded: bool,
    pub change: Option<Change>,
    /// A placeholder row ("not captured at this stop").
    pub note: bool,
}

#[derive(Debug, Clone)]
pub enum StackRow {
    Frame(usize),
    /// Consecutive library frames shown as one line.
    Collapsed {
        first: usize,
        count: usize,
        label: String,
    },
}

#[derive(Debug, Clone)]
pub enum BpRow {
    /// Heads a tab's rows when the breakpoints of every tab are listed.
    Tab(TabId),
    Breakpoint {
        tab: TabId,
        id: BreakpointId,
    },
    Exception {
        tab: TabId,
        session: SessionId,
        filter: String,
        label: String,
    },
}

/// A row of CONFIG the cursor can be on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigRow {
    /// One of the project's configurations, by index.
    Configuration(usize),
    /// A field of the selected configuration.
    Field(String),
    /// A path in `pathMappings` of the selected configuration, by its remote side.
    Mapping(String),
}

/// What SOURCE shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shown {
    /// A file, by its path here, where it may not exist.
    File(PathBuf),
    /// A source the adapter of `session` has itself, with no file here: code generated at
    /// run time, a runtime's internals.
    Adapter { session: SessionId, reference: i64, name: String },
}

/// Breakpoints the BREAKPOINTS scope leaves out.
pub struct Hidden {
    /// In this tab, for adapters the active session does not handle.
    pub here: usize,
    pub other_tabs: usize,
}

impl Tab {
    pub fn focus(&self) -> Pane {
        self.ui.focus.get(&self.ui.view).copied().unwrap_or(crate::app::panes(self.ui.view)[0])
    }

    pub fn set_focus(&mut self, pane: Pane) {
        self.ui.focus.insert(self.ui.view, pane);
    }

    pub fn active(&self) -> Option<&Session> {
        self.engine.session(self.ui.session?)
    }

    /// The sessions as the screen counts them: one per configuration started. The child
    /// sessions an adapter asks for (js-debug runs each Node process or page in one) are
    /// part of their parent's.
    pub fn sessions(&self) -> impl Iterator<Item = &Session> {
        self.engine.sessions().iter().filter(|s| s.parent.is_none())
    }

    /// The part of `sid`'s session to show and act on: where its program runs. Once the
    /// adapter has started children, that is one of them, `sid` itself while it lives;
    /// before that, the session itself.
    pub fn shown(&self, sid: SessionId) -> SessionId {
        let root = self.engine.root(sid);
        let children: Vec<(SessionId, bool)> = self
            .engine
            .sessions()
            .iter()
            .filter(|s| s.id != root && self.engine.root(s.id) == root)
            .map(|s| (s.id, s.is_alive()))
            .collect();
        shown_part(sid, root, &children)
    }

    /// Sessions with anything still running, a parent and its children counted once.
    pub fn live_sessions(&self) -> usize {
        let live: Vec<SessionId> =
            self.engine.sessions().iter().filter(|s| s.is_alive()).map(|s| self.engine.root(s.id)).collect();
        self.sessions().filter(|s| live.contains(&s.id)).count()
    }

    /// The stop being looked at: the latest one, or one picked from history.
    pub fn viewed(&self) -> Option<&Snapshot> {
        let session = self.active()?;
        match self.ui.viewing.get(&session.id) {
            Some(number) => session.snapshot(*number),
            None => session.latest(),
        }
    }

    /// The session, stop and frame whose variables LOCALS shows.
    pub fn viewed_frame(&self) -> Option<(SessionId, u32, usize)> {
        Some((self.ui.session?, self.viewed()?.number, self.selected_frame_index()))
    }

    /// True when an older stop is shown instead of the latest one.
    pub fn in_history(&self) -> bool {
        self.ui.session.is_some_and(|sid| self.ui.viewing.contains_key(&sid))
    }

    pub fn previous(&self) -> Option<&Snapshot> {
        let number = self.viewed()?.number;
        self.active()?.snapshot(number.checked_sub(1)?)
    }

    /// What changed in the selected frame since the previous stop.
    pub fn changes(&self) -> HashMap<VarPath, Change> {
        match self.viewed() {
            Some(snapshot) => diff(self.previous(), snapshot, self.selected_frame_index()),
            None => HashMap::new(),
        }
    }

    /// The variables of the frame selected in STACK, as rows.
    pub fn local_rows(&self) -> Vec<LocalRow> {
        let (Some(session), Some(snapshot)) = (self.active(), self.viewed()) else { return Vec::new() };
        let changes = self.changes();
        let mut rows = Vec::new();
        for scope in snapshot.scopes(self.selected_frame_index()).unwrap_or_default() {
            let path = vec![scope.name.clone()];
            let expanded = session.expanded.contains(&path);
            rows.push(LocalRow {
                depth: 0,
                path: path.clone(),
                name: scope.name.clone(),
                type_name: String::new(),
                value: String::new(),
                evaluate_name: None,
                scope: true,
                expandable: true,
                expanded,
                change: None,
                note: false,
            });
            if !expanded {
                continue;
            }
            match &scope.vars {
                Some(vars) => push_vars(&mut rows, session, &changes, vars, &path, 1, snapshot.live),
                None => rows.push(note(&path, 1, snapshot.live)),
            }
        }
        rows
    }

    pub fn selected_local_expr(&self) -> Option<String> {
        let row = self.local_rows().into_iter().nth(self.ui.locals.pos)?;
        if row.scope || row.note {
            return None;
        }
        row.evaluate_name.or(Some(row.name))
    }

    pub fn stack_rows(&self) -> Vec<StackRow> {
        let Some(snapshot) = self.viewed() else { return Vec::new() };
        let frames = &snapshot.frames;
        if self.ui.show_vendor {
            return (0..frames.len()).map(StackRow::Frame).collect();
        }
        let mut rows = Vec::new();
        let mut i = 0;
        while i < frames.len() {
            if i == 0 || !frames[i].library {
                rows.push(StackRow::Frame(i));
                i += 1;
                continue;
            }
            let first = i;
            while i < frames.len() && frames[i].library {
                i += 1;
            }
            let mut packages: Vec<String> = Vec::new();
            for frame in &frames[first..i] {
                let package = frame.path.as_deref().map(package_of).unwrap_or_else(|| "library".into());
                if !packages.contains(&package) {
                    packages.push(package);
                }
            }
            rows.push(StackRow::Collapsed { first, count: i - first, label: packages.join(" · ") });
        }
        rows
    }

    /// Index of the frame selected in STACK; the top frame's, 0, by default.
    pub fn selected_frame_index(&self) -> usize {
        match self.stack_rows().get(self.ui.stack.pos) {
            Some(StackRow::Frame(i)) => *i,
            Some(StackRow::Collapsed { first, .. }) => *first,
            None => 0,
        }
    }

    pub fn selected_frame(&self) -> Option<&Frame> {
        self.viewed()?.frames.get(self.selected_frame_index())
    }

    /// What SOURCE shows: the file browsed, else the selected frame's source. That is the
    /// adapter's own text when the adapter has it and the frame's path names no file here.
    pub fn shown_source(&self) -> Option<Shown> {
        if let Some(path) = &self.ui.source.browse {
            return Some(Shown::File(path.clone()));
        }
        let frame = self.selected_frame()?;
        match (frame.source_reference, &frame.path) {
            (Some(reference), path) if path.as_deref().is_none_or(|p| !p.is_file()) => {
                let path = path.as_ref().map(|p| p.display().to_string());
                let name = frame.source_name.clone().or(path).unwrap_or_else(|| format!("source {reference}"));
                Some(Shown::Adapter { session: self.ui.session?, reference, name })
            }
            (_, path) => path.clone().map(Shown::File),
        }
    }

    /// File shown in SOURCE.
    pub fn source_path(&self) -> Option<PathBuf> {
        match self.shown_source()? {
            Shown::File(path) => Some(path),
            Shown::Adapter { .. } => None,
        }
    }

    /// The configuration CONFIG shows and `S` starts.
    pub fn selected_configuration(&self) -> Option<&Configuration> {
        self.project.as_ref()?.configurations.get(self.ui.config_sel)
    }

    /// The configurations, then the fields of the selected one, each mapping on a row.
    pub fn config_rows(&self) -> Vec<ConfigRow> {
        let Some(project) = &self.project else { return Vec::new() };
        let mut rows: Vec<ConfigRow> = (0..project.configurations.len()).map(ConfigRow::Configuration).collect();
        if let Some(config) = self.selected_configuration() {
            for field in field_order(config) {
                match config.fields.get(field) {
                    Some(Value::Object(mappings)) if field == "pathMappings" && !mappings.is_empty() => {
                        rows.extend(mappings.keys().map(|remote| ConfigRow::Mapping(remote.clone())));
                    }
                    _ => rows.push(ConfigRow::Field(field.to_owned())),
                }
            }
        }
        rows
    }

    pub fn config_row(&self) -> Option<ConfigRow> {
        self.config_rows().into_iter().nth(self.ui.config.pos)
    }

    /// What `copy` and editing take from a row: a field's value, a mapping as `remote=local`.
    pub fn config_text(&self, row: &ConfigRow) -> Option<String> {
        let config = self.selected_configuration()?;
        match row {
            ConfigRow::Configuration(index) => {
                Some(self.project.as_ref()?.configurations.get(*index)?.name().to_owned())
            }
            ConfigRow::Field(field) => config.fields.get(field).map(value_text),
            ConfigRow::Mapping(remote) => {
                let local = config.fields.get("pathMappings")?.get(remote)?;
                Some(format!("{remote}={}", value_text(local)))
            }
        }
    }
}

/// Which part of a session to show: `root`, or one of its `children` (`(id, alive)`, oldest
/// first) once there are any. `sid` is the part shown so far, and stays while it lives.
fn shown_part(sid: SessionId, root: SessionId, children: &[(SessionId, bool)]) -> SessionId {
    match children.iter().rev().find(|(_, alive)| *alive) {
        Some(_) if children.contains(&(sid, true)) => sid,
        Some((live, _)) => *live,
        // Once the program is over, its stops are read where they happened.
        None if children.iter().any(|(id, _)| *id == sid) => sid,
        None => children.last().map_or(root, |(id, _)| *id),
    }
}

/// Fields as launch.json orders them: what the configuration is first, then the rest.
pub fn field_order(config: &Configuration) -> Vec<&str> {
    const FIRST: [&str; 3] = ["name", "type", "request"];
    let rest = config.fields.keys().map(String::as_str).filter(|field| !FIRST.contains(field));
    FIRST.into_iter().filter(|field| config.fields.contains_key(*field)).chain(rest).collect()
}

impl App {
    pub fn bp_rows(&self) -> Vec<BpRow> {
        let current = self.tab();
        match current.ui.bp_scope {
            BpScope::Session => tab_bp_rows(self.tabs.active_id(), current, true),
            BpScope::All => {
                let grouped = self.tabs.len() > 1;
                let mut rows = Vec::new();
                for (id, tab) in self.tabs.iter() {
                    let tab_rows = tab_bp_rows(id, tab, false);
                    if grouped && !tab_rows.is_empty() {
                        rows.push(BpRow::Tab(id));
                    }
                    rows.extend(tab_rows);
                }
                rows
            }
        }
    }

    pub fn hidden_bps(&self) -> Hidden {
        let rows = self.bp_rows();
        let shown =
            |id: TabId| rows.iter().filter(|r| matches!(r, BpRow::Breakpoint { tab, .. } if *tab == id)).count();
        let mut hidden = Hidden { here: 0, other_tabs: 0 };
        for (id, tab) in self.tabs.iter() {
            let count = tab.engine.breakpoints().len() - shown(id);
            if id == self.tabs.active_id() {
                hidden.here += count;
            } else {
                hidden.other_tabs += count;
            }
        }
        hidden
    }

    /// The lines SOURCE shows, read from the file or given by the adapter; `None` while they
    /// cannot be had.
    pub fn shown_lines(&self) -> Option<Rc<SourceFile>> {
        let tab = self.tab();
        match tab.shown_source()? {
            Shown::File(path) => self.sources.borrow_mut().get(&path),
            Shown::Adapter { session, reference, .. } => {
                let Some(SourceText::Read(text)) = tab.engine.session(session)?.source(reference) else { return None };
                Some(self.sources.borrow_mut().adapter((self.tabs.active_id(), session, reference), text))
            }
        }
    }

    /// The breakpoint under the BREAKPOINTS cursor, and the tab it belongs to.
    pub fn selected_bp(&self) -> Option<(TabId, BreakpointId)> {
        let tab = self.tab();
        if tab.focus() != Pane::Breakpoints {
            return None;
        }
        match self.bp_rows().get(tab.ui.bps.pos)? {
            BpRow::Breakpoint { tab, id } => Some((*tab, *id)),
            BpRow::Tab(_) | BpRow::Exception { .. } => None,
        }
    }

    /// Text for `copy`, depending on the focused pane.
    pub fn selected_text(&self) -> Option<String> {
        let tab = self.tab();
        match tab.focus() {
            Pane::Locals => {
                let row = tab.local_rows().into_iter().nth(tab.ui.locals.pos)?;
                (!row.scope && !row.note).then_some(row.value)
            }
            Pane::Watch => {
                let expr = tab.engine.watches().get(tab.ui.watch.pos)?;
                tab.viewed()?.watch(expr)?.clone().ok()
            }
            Pane::Source => {
                let file = self.shown_lines()?;
                file.lines.get(usize::try_from(tab.ui.source.cursor).ok()?.checked_sub(1)?).map(|l| l.trim().to_owned())
            }
            Pane::Stack => {
                let frame = tab.selected_frame()?;
                Some(match &frame.path {
                    Some(path) => format!("{}:{}", path.display(), frame.line),
                    None => frame.name.clone(),
                })
            }
            Pane::Breakpoints => {
                let (tab, id) = self.selected_bp()?;
                let bp = self.tabs.get(tab)?.engine.breakpoints().get(id)?;
                Some(format!("{}:{}", bp.path.display(), bp.line))
            }
            Pane::Output => {
                let len = tab.engine.output().len();
                let index = len.checked_sub(1 + tab.ui.output_back)?;
                tab.engine.output().iter().nth(index).map(|l| l.text.clone())
            }
            Pane::Config => tab.config_text(&tab.config_row()?),
            Pane::Events | Pane::History => None,
        }
    }
}

/// A tab's breakpoints and exception filters: those of its active session, or of all of them.
fn tab_bp_rows(id: TabId, tab: &Tab, session_only: bool) -> Vec<BpRow> {
    let session = tab.active();
    let in_scope = |path: &Path| !session_only || session.is_none_or(|s| s.handles(path));
    let mut rows: Vec<BpRow> = tab
        .engine
        .breakpoints()
        .iter()
        .filter(|b| in_scope(&b.path))
        .map(|b| BpRow::Breakpoint { tab: id, id: b.id })
        .collect();
    let sessions: Vec<&Session> = if session_only {
        session.into_iter().collect()
    } else {
        // The part that runs the program: a parent whose children run it has filters of its
        // own, but no code they could stop.
        tab.sessions().filter_map(|s| tab.engine.session(tab.shown(s.id))).filter(|s| s.is_alive()).collect()
    };
    for s in sessions {
        for f in &s.exception_filters {
            rows.push(BpRow::Exception { tab: id, session: s.id, filter: f.filter.clone(), label: f.label.clone() });
        }
    }
    rows
}

fn push_vars(
    rows: &mut Vec<LocalRow>,
    session: &Session,
    changes: &HashMap<VarPath, Change>,
    vars: &[Var],
    parent: &VarPath,
    depth: usize,
    live: bool,
) {
    for var in vars {
        let mut path = parent.clone();
        path.push(var.name.clone());
        let expandable = var.reference > 0 || var.children.is_some();
        let expanded = expandable && session.expanded.contains(&path);
        rows.push(LocalRow {
            depth,
            path: path.clone(),
            name: var.name.clone(),
            type_name: var.type_name.clone().unwrap_or_default(),
            value: var.value.clone(),
            evaluate_name: var.evaluate_name.clone(),
            scope: false,
            expandable,
            expanded,
            change: changes.get(&path).cloned(),
            note: false,
        });
        if expanded {
            match &var.children {
                Some(children) => push_vars(rows, session, changes, children, &path, depth + 1, live),
                None => rows.push(note(&path, depth + 1, live)),
            }
        }
    }
}

fn note(parent: &VarPath, depth: usize, live: bool) -> LocalRow {
    LocalRow {
        depth,
        path: parent.clone(),
        name: String::new(),
        type_name: String::new(),
        value: if live { "loading…".into() } else { "not captured at this stop".into() },
        evaluate_name: None,
        scope: false,
        expandable: false,
        expanded: false,
        change: None,
        note: true,
    }
}

/// `…/vendor/symfony/http-kernel/HttpKernel.php` → `symfony/http-kernel`.
pub fn package_of(path: &std::path::Path) -> String {
    let text = path.to_string_lossy();
    let after = |marker: &str, parts: usize| {
        text.split_once(marker).map(|(_, rest)| rest.split('/').take(parts).collect::<Vec<_>>().join("/"))
    };
    after("/vendor/", 2)
        .or_else(|| {
            let rest = text.rsplit_once("/node_modules/")?.1;
            let parts = if rest.starts_with('@') { 2 } else { 1 };
            Some(rest.split('/').take(parts).collect::<Vec<_>>().join("/"))
        })
        .or_else(|| after("/.cargo/registry/src/", 2).map(|p| p.split('/').nth(1).unwrap_or(&p).to_owned()))
        .or_else(|| text.contains("/rustc/").then(|| "std".to_owned()))
        .unwrap_or_else(|| "library".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{package_of, shown_part};
    use std::path::Path;

    #[test]
    fn a_session_shows_the_child_its_program_runs_in() {
        // Before the adapter starts a child, the session itself.
        assert_eq!(shown_part(1, 1, &[]), 1);
        assert_eq!(shown_part(1, 1, &[(2, true)]), 2);
        // A live child that is looked at stays; one that ended gives way to one that lives.
        assert_eq!(shown_part(2, 1, &[(2, true), (3, true)]), 2);
        assert_eq!(shown_part(2, 1, &[(2, false), (3, true)]), 3);
        // Once everything ended, the child stays, with the stops recorded in it.
        assert_eq!(shown_part(2, 1, &[(2, false), (3, false)]), 2);
        assert_eq!(shown_part(1, 1, &[(2, false), (3, false)]), 3);
    }

    #[test]
    fn package_names() {
        assert_eq!(package_of(Path::new("/app/vendor/symfony/http-kernel/HttpKernel.php")), "symfony/http-kernel");
        assert_eq!(package_of(Path::new("/w/node_modules/@angular/core/fesm.js")), "@angular/core");
        assert_eq!(package_of(Path::new("/w/node_modules/zone.js/dist/zone.js")), "zone.js");
        assert_eq!(
            package_of(Path::new("/h/.cargo/registry/src/index.crates.io-6f17d22bba15001f/tokio-1.40.0/src/rt.rs")),
            "tokio-1.40.0"
        );
    }
}
