//! Everything captured at one stop. Snapshots are immutable history: the UI can
//! show any of them after the program moved on, and compare neighbours.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Local};

use crate::breakpoints::BreakpointId;

/// Names from the scope down to a nested variable, e.g. `["Locals", "$order", "items"]`.
pub type VarPath = Vec<String>;

#[derive(Debug, Clone)]
pub struct Snapshot {
    /// 1-based stop number within its session.
    pub number: u32,
    pub time: DateTime<Local>,
    pub thread_id: i64,
    pub reason: String,
    pub description: Option<String>,
    pub hit_breakpoints: Vec<BreakpointId>,
    pub frames: Vec<Frame>,
    /// Scopes of the top frame.
    pub scopes: Vec<ScopeVars>,
    pub watches: Vec<WatchValue>,
    /// Variable references are only valid until the debuggee resumes.
    pub live: bool,
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub id: i64,
    pub name: String,
    pub path: Option<PathBuf>,
    pub source_name: Option<String>,
    pub line: u32,
    pub column: u32,
    /// Library / vendor code; collapsed in the stack by default.
    pub library: bool,
}

#[derive(Debug, Clone)]
pub struct ScopeVars {
    pub name: String,
    /// Arguments/locals: fetched and shown expanded unless the user collapsed them.
    pub default_open: bool,
    pub reference: i64,
    pub expensive: bool,
    /// `None` until fetched (expensive scopes are fetched on demand).
    pub vars: Option<Vec<Var>>,
}

#[derive(Debug, Clone)]
pub struct Var {
    pub name: String,
    pub value: String,
    pub type_name: Option<String>,
    pub evaluate_name: Option<String>,
    /// > 0 when the variable has children.
    pub reference: i64,
    pub children: Option<Vec<Var>>,
}

#[derive(Debug, Clone)]
pub struct WatchValue {
    pub expr: String,
    pub value: Result<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Changed { was: String },
    New,
}

impl Frame {
    pub fn location(&self) -> String {
        let file = self
            .path
            .as_deref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .or_else(|| self.source_name.clone())
            .unwrap_or_else(|| "?".to_owned());
        format!("{file}:{}", self.line)
    }
}

/// Paths that look like dependencies rather than the user's code.
pub fn is_library_path(path: &std::path::Path) -> bool {
    const MARKERS: &[&str] =
        &["/vendor/", "/node_modules/", "/.cargo/registry/", "/rustc/", "/usr/include/", "/usr/lib/"];
    let path = path.to_string_lossy();
    MARKERS.iter().any(|m| path.contains(m))
}

impl Snapshot {
    pub fn top(&self) -> Option<&Frame> {
        self.frames.first()
    }

    pub fn watch(&self, expr: &str) -> Option<&Result<String, String>> {
        self.watches.iter().find(|w| w.expr == expr).map(|w| &w.value)
    }

    /// Top-level variables of the first scope that has been fetched.
    pub fn locals(&self) -> &[Var] {
        self.scopes.iter().find_map(|s| s.vars.as_deref()).unwrap_or(&[])
    }

    pub fn var(&self, path: &[String]) -> Option<&Var> {
        let (scope, names) = path.split_first()?;
        let mut vars = self.scopes.iter().find(|s| &s.name == scope)?.vars.as_deref()?;
        let (last, parents) = names.split_last()?;
        for name in parents {
            vars = vars.iter().find(|v| &v.name == name)?.children.as_deref()?;
        }
        vars.iter().find(|v| &v.name == last)
    }

    pub fn var_mut(&mut self, path: &[String]) -> Option<&mut Var> {
        let (scope, names) = path.split_first()?;
        let mut vars = self.scopes.iter_mut().find(|s| &s.name == scope)?.vars.as_mut()?;
        let (last, parents) = names.split_last()?;
        for name in parents {
            vars = vars.iter_mut().find(|v| &v.name == name)?.children.as_mut()?;
        }
        vars.iter_mut().find(|v| &v.name == last)
    }

    /// Value of a top-level variable by name in any fetched scope.
    pub fn local_value(&self, name: &str) -> Option<&str> {
        self.scopes
            .iter()
            .filter_map(|s| s.vars.as_deref())
            .flatten()
            .find(|v| v.name == name)
            .map(|v| v.value.as_str())
    }
}

/// What changed since `prev`. Only meaningful within the same function: after
/// stepping into another frame every variable is simply new, so nothing is marked.
pub fn diff(prev: Option<&Snapshot>, cur: &Snapshot) -> HashMap<VarPath, Change> {
    let mut changes = HashMap::new();
    let Some(prev) = prev else { return changes };
    if prev.top().map(|f| &f.name) != cur.top().map(|f| &f.name) {
        return changes;
    }
    for scope in &cur.scopes {
        let (Some(vars), Some(old)) = (&scope.vars, prev.scopes.iter().find(|s| s.name == scope.name)) else {
            continue;
        };
        let Some(old_vars) = &old.vars else { continue };
        diff_vars(&mut vec![scope.name.clone()], vars, old_vars, &mut changes);
    }
    changes
}

fn diff_vars(path: &mut VarPath, cur: &[Var], prev: &[Var], out: &mut HashMap<VarPath, Change>) {
    for var in cur {
        path.push(var.name.clone());
        match prev.iter().find(|p| p.name == var.name) {
            None => {
                out.insert(path.clone(), Change::New);
            }
            Some(old) => {
                if old.value != var.value {
                    out.insert(path.clone(), Change::Changed { was: old.value.clone() });
                }
                if let (Some(children), Some(old_children)) = (&var.children, &old.children) {
                    diff_vars(path, children, old_children, out);
                }
            }
        }
        path.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(name: &str, value: &str) -> Var {
        Var {
            name: name.into(),
            value: value.into(),
            type_name: None,
            evaluate_name: None,
            reference: 0,
            children: None,
        }
    }

    fn snap(function: &str, vars: Vec<Var>) -> Snapshot {
        Snapshot {
            number: 1,
            time: Local::now(),
            thread_id: 1,
            reason: "step".into(),
            description: None,
            hit_breakpoints: vec![],
            frames: vec![Frame {
                id: 1,
                name: function.into(),
                path: None,
                source_name: None,
                line: 1,
                column: 1,
                library: false,
            }],
            scopes: vec![ScopeVars {
                name: "Locals".into(),
                default_open: true,
                reference: 1,
                expensive: false,
                vars: Some(vars),
            }],
            watches: vec![],
            live: true,
        }
    }

    fn path(names: &[&str]) -> VarPath {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn marks_changed_and_new_variables() {
        let prev = snap("assign", vec![var("$total", "239.97"), var("$i", "3")]);
        let cur = snap("assign", vec![var("$total", "359.96"), var("$i", "3"), var("$owner", "null")]);
        let d = diff(Some(&prev), &cur);
        assert_eq!(d.get(&path(&["Locals", "$total"])), Some(&Change::Changed { was: "239.97".into() }));
        assert_eq!(d.get(&path(&["Locals", "$owner"])), Some(&Change::New));
        assert!(!d.contains_key(&path(&["Locals", "$i"])));
    }

    #[test]
    fn compares_loaded_children() {
        let mut order = var("$order", "Order");
        order.children = Some(vec![var("status", "Pending")]);
        let mut order2 = var("$order", "Order");
        order2.children = Some(vec![var("status", "Assigned")]);
        let d = diff(Some(&snap("f", vec![order])), &snap("f", vec![order2]));
        assert!(d.contains_key(&path(&["Locals", "$order", "status"])));
    }

    #[test]
    fn different_function_has_no_deltas() {
        let prev = snap("controller", vec![var("$id", "812")]);
        let cur = snap("assign", vec![var("$id", "1")]);
        assert!(diff(Some(&prev), &cur).is_empty());
    }

    #[test]
    fn finds_nested_var_by_path() {
        let mut order = var("$order", "Order");
        order.children = Some(vec![var("status", "Pending")]);
        let s = snap("f", vec![order]);
        assert_eq!(s.var(&path(&["Locals", "$order", "status"])).map(|v| v.value.as_str()), Some("Pending"));
        assert!(s.var(&path(&["Locals", "$nope"])).is_none());
    }
}
