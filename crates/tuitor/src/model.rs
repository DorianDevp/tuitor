//! Read-only views of engine state shaped for the panes: rows of the variable
//! tree, stack with library frames grouped, breakpoints in scope.

use std::collections::HashMap;
use std::path::PathBuf;

use tuitor_core::breakpoints::BreakpointId;
use tuitor_core::session::{Session, SessionId};
use tuitor_core::snapshot::{Change, Frame, Snapshot, Var, VarPath, diff};

use crate::app::{App, Pane};
use crate::command::BpScope;

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
    Breakpoint(BreakpointId),
    Exception { session: SessionId, filter: String, label: String },
}

impl App {
    pub fn active(&self) -> Option<&Session> {
        self.engine.session(self.ui.session?)
    }

    /// The stop being looked at: the latest one, or one picked from history.
    pub fn viewed(&self) -> Option<&Snapshot> {
        let session = self.active()?;
        match self.ui.viewing.get(&session.id) {
            Some(number) => session.snapshot(*number),
            None => session.latest(),
        }
    }

    pub fn viewed_ids(&self) -> Option<(SessionId, u32)> {
        Some((self.ui.session?, self.viewed()?.number))
    }

    /// True when an older stop is shown instead of the latest one.
    pub fn in_history(&self) -> bool {
        self.ui.session.is_some_and(|sid| self.ui.viewing.contains_key(&sid))
    }

    pub fn previous(&self) -> Option<&Snapshot> {
        let number = self.viewed()?.number;
        self.active()?.snapshot(number.checked_sub(1)?)
    }

    pub fn changes(&self) -> HashMap<VarPath, Change> {
        match self.viewed() {
            Some(snapshot) => diff(self.previous(), snapshot),
            None => HashMap::new(),
        }
    }

    pub fn local_rows(&self) -> Vec<LocalRow> {
        let (Some(session), Some(snapshot)) = (self.active(), self.viewed()) else { return Vec::new() };
        let changes = self.changes();
        let mut rows = Vec::new();
        for scope in &snapshot.scopes {
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

    pub fn selected_frame(&self) -> Option<&Frame> {
        let snapshot = self.viewed()?;
        let index = match self.stack_rows().get(self.ui.stack.pos) {
            Some(StackRow::Frame(i)) => *i,
            Some(StackRow::Collapsed { first, .. }) => *first,
            None => 0,
        };
        snapshot.frames.get(index)
    }

    /// File shown in SOURCE.
    pub fn source_path(&self) -> Option<PathBuf> {
        self.ui.source.browse.clone().or_else(|| self.selected_frame()?.path.clone())
    }

    pub fn bp_rows(&self) -> Vec<BpRow> {
        let session = self.active();
        let in_scope = |path: &std::path::Path| match (self.ui.bp_scope, session) {
            (BpScope::All, _) | (BpScope::Session, None) => true,
            (BpScope::Session, Some(s)) => s.handles(path),
        };
        let mut rows: Vec<BpRow> =
            self.engine.breakpoints().iter().filter(|b| in_scope(&b.path)).map(|b| BpRow::Breakpoint(b.id)).collect();
        let sessions: Vec<&Session> = match self.ui.bp_scope {
            BpScope::Session => session.into_iter().collect(),
            BpScope::All => self.engine.sessions().iter().filter(|s| s.is_alive()).collect(),
        };
        for s in sessions {
            for f in &s.exception_filters {
                rows.push(BpRow::Exception { session: s.id, filter: f.filter.clone(), label: f.label.clone() });
            }
        }
        rows
    }

    /// Breakpoints that exist but are hidden by the session scope.
    pub fn hidden_bp_count(&self) -> usize {
        let shown = self.bp_rows().iter().filter(|r| matches!(r, BpRow::Breakpoint(_))).count();
        self.engine.breakpoints().len() - shown
    }

    pub fn selected_bp_id(&self) -> Option<BreakpointId> {
        if self.focus() != Pane::Breakpoints {
            return None;
        }
        match self.bp_rows().get(self.ui.bps.pos)? {
            BpRow::Breakpoint(id) => Some(*id),
            BpRow::Exception { .. } => None,
        }
    }

    /// Text for `copy`, depending on the focused pane.
    pub fn selected_text(&self) -> Option<String> {
        match self.focus() {
            Pane::Locals => {
                let row = self.local_rows().into_iter().nth(self.ui.locals.pos)?;
                (!row.scope && !row.note).then_some(row.value)
            }
            Pane::Watch => {
                let expr = self.engine.watches().get(self.ui.watch.pos)?;
                self.viewed()?.watch(expr)?.clone().ok()
            }
            Pane::Source => {
                let path = self.source_path()?;
                let file = self.sources.borrow_mut().get(&path)?;
                file.lines
                    .get(usize::try_from(self.ui.source.cursor).ok()?.checked_sub(1)?)
                    .map(|l| l.trim().to_owned())
            }
            Pane::Stack => {
                let frame = self.selected_frame()?;
                Some(match &frame.path {
                    Some(path) => format!("{}:{}", path.display(), frame.line),
                    None => frame.name.clone(),
                })
            }
            Pane::Breakpoints => {
                let bp = self.engine.breakpoints().get(self.selected_bp_id()?)?;
                Some(format!("{}:{}", bp.path.display(), bp.line))
            }
            Pane::Output => {
                let len = self.engine.output().len();
                let index = len.checked_sub(1 + self.ui.output_back)?;
                self.engine.output().iter().nth(index).map(|l| l.text.clone())
            }
            Pane::Events | Pane::History => None,
        }
    }
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
    use super::package_of;
    use std::path::Path;

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
