//! Everything captured at one stop. Snapshots are immutable history: the UI can
//! show any of them after the program moved on, and compare neighbours.

use std::collections::{BTreeSet, HashMap, HashSet};
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
    /// How many children the adapter announced, if it did.
    pub child_count: Option<i64>,
    pub children: Option<Vec<Var>>,
}

#[derive(Debug, Clone)]
pub struct WatchValue {
    pub expr: String,
    pub value: Result<String, String>,
}

/// How a variable differs from the previous stop. `Inside`: its value reads the same,
/// but loaded children (at any depth) changed, appeared or disappeared. Objects whose
/// value is just a class name only ever change this way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Changed { was: String },
    New,
    Inside,
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

    /// Rows shown expanded whose children were not read, with the reference to read them by.
    /// A row is shown expanded when its path and every path above it are in `expanded`. Only
    /// `root` and the rows above and below it count; `[]` means every row.
    pub fn unloaded(&self, root: &[String], expanded: &BTreeSet<VarPath>) -> Vec<(VarPath, i64)> {
        let mut out = Vec::new();
        for scope in &self.scopes {
            let mut path = vec![scope.name.clone()];
            if !on_branch(&path, root) || !expanded.contains(&path) {
                continue;
            }
            match &scope.vars {
                Some(vars) => unloaded_vars(&mut path, vars, root, expanded, &mut out),
                None => out.push((path, scope.reference)),
            }
        }
        out
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

/// [`Snapshot::unloaded`] among `vars`, the children of `path`.
fn unloaded_vars(
    path: &mut VarPath,
    vars: &[Var],
    root: &[String],
    expanded: &BTreeSet<VarPath>,
    out: &mut Vec<(VarPath, i64)>,
) {
    for var in vars {
        path.push(var.name.clone());
        if on_branch(path, root) && expanded.contains(path) {
            match &var.children {
                Some(children) => unloaded_vars(path, children, root, expanded, out),
                None if var.reference > 0 => out.push((path.clone(), var.reference)),
                None => {}
            }
        }
        path.pop();
    }
}

/// Whether one path starts with the other.
fn on_branch(path: &[String], root: &[String]) -> bool {
    path.iter().zip(root).all(|(a, b)| a == b)
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

/// Records changes below `path` and tells whether there were any, so that parents can
/// be marked [`Change::Inside`].
fn diff_vars(path: &mut VarPath, cur: &[Var], prev: &[Var], out: &mut HashMap<VarPath, Change>) -> bool {
    // Indexed by name, as an expanded array can have thousands of elements. Reversed, so
    // that of equal names (shadowed locals) the first one is compared.
    let old_by_name: HashMap<&str, &Var> = prev.iter().rev().map(|v| (v.name.as_str(), v)).collect();
    let names: HashSet<&str> = cur.iter().map(|v| v.name.as_str()).collect();
    let mut any = prev.iter().any(|old| !names.contains(old.name.as_str())); // one disappeared
    for var in cur {
        path.push(var.name.clone());
        match old_by_name.get(var.name.as_str()) {
            None => {
                out.insert(path.clone(), Change::New);
                any = true;
            }
            Some(old) => {
                let inside = match (&var.children, &old.children) {
                    (Some(children), Some(old_children)) => diff_vars(path, children, old_children, out),
                    _ => false,
                };
                let changed = old.value != var.value;
                // The value itself changing says more than anything below it.
                if changed {
                    out.insert(path.clone(), Change::Changed { was: old.value.clone() });
                } else if inside {
                    out.insert(path.clone(), Change::Inside);
                }
                any |= changed || inside;
            }
        }
        path.pop();
    }
    any
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
            child_count: None,
            children: None,
        }
    }

    fn object(name: &str, value: &str, children: Vec<Var>) -> Var {
        Var { reference: 1, children: Some(children), ..var(name, value) }
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
    fn object_whose_field_changed_is_marked_inside() {
        let prev = snap("orderTotal", vec![object("$line", "Line", vec![var("sku", "KB-01"), var("qty", "1")])]);
        let cur = snap("orderTotal", vec![object("$line", "Line", vec![var("sku", "MS-02"), var("qty", "1")])]);
        let d = diff(Some(&prev), &cur);
        assert_eq!(d.get(&path(&["Locals", "$line"])), Some(&Change::Inside));
        assert_eq!(d.get(&path(&["Locals", "$line", "sku"])), Some(&Change::Changed { was: "KB-01".into() }));
        assert!(!d.contains_key(&path(&["Locals", "$line", "qty"])));
    }

    #[test]
    fn inside_reaches_every_ancestor() {
        let order = |city: &str| {
            let address = object("address", "Address", vec![var("city", city)]);
            object("$order", "Order", vec![var("id", "1001"), object("customer", "Customer", vec![address])])
        };
        let d = diff(Some(&snap("f", vec![order("London")])), &snap("f", vec![order("Paris")]));
        for parent in
            [&["Locals", "$order"][..], &["Locals", "$order", "customer"], &["Locals", "$order", "customer", "address"]]
        {
            assert_eq!(d.get(&path(parent)), Some(&Change::Inside), "{parent:?}");
        }
        assert!(!d.contains_key(&path(&["Locals", "$order", "id"])));
    }

    #[test]
    fn own_value_change_wins_over_inside() {
        let prev = snap("f", vec![object("line", "0x10", vec![var("qty", "1")])]);
        let cur = snap("f", vec![object("line", "0x20", vec![var("qty", "2")])]);
        let d = diff(Some(&prev), &cur);
        assert_eq!(d.get(&path(&["Locals", "line"])), Some(&Change::Changed { was: "0x10".into() }));
        assert_eq!(d.get(&path(&["Locals", "line", "qty"])), Some(&Change::Changed { was: "1".into() }));
    }

    #[test]
    fn children_that_appear_or_disappear_mark_the_parent() {
        let prev = snap("f", vec![object("$o", "Order", vec![var("a", "1"), var("b", "2")])]);
        let fewer = snap("f", vec![object("$o", "Order", vec![var("a", "1")])]);
        let more = snap("f", vec![object("$o", "Order", vec![var("a", "1"), var("b", "2"), var("c", "3")])]);
        assert_eq!(diff(Some(&prev), &fewer).get(&path(&["Locals", "$o"])), Some(&Change::Inside));
        let d = diff(Some(&prev), &more);
        assert_eq!(d.get(&path(&["Locals", "$o"])), Some(&Change::Inside));
        assert_eq!(d.get(&path(&["Locals", "$o", "c"])), Some(&Change::New));
    }

    #[test]
    fn same_or_unloaded_children_mark_nothing() {
        let loaded = || object("$o", "Order", vec![var("a", "1")]);
        assert!(diff(Some(&snap("f", vec![loaded()])), &snap("f", vec![loaded()])).is_empty());
        // Children read at only one of the stops cannot be compared.
        let unloaded = var("$o", "Order");
        assert!(diff(Some(&snap("f", vec![unloaded.clone()])), &snap("f", vec![loaded()])).is_empty());
        assert!(diff(Some(&snap("f", vec![loaded()])), &snap("f", vec![unloaded])).is_empty());
    }

    #[test]
    fn different_function_has_no_deltas() {
        let prev = snap("controller", vec![var("$id", "812")]);
        let cur = snap("assign", vec![var("$id", "1")]);
        assert!(diff(Some(&prev), &cur).is_empty());
    }

    #[test]
    fn unloaded_rows_are_expanded_ones_shown_without_children() {
        // `customer` stayed expanded while `$order` was collapsed; `$order` was then read ahead.
        let customer = Var { reference: 7, ..var("customer", "Customer") };
        let s = snap("f", vec![object("$order", "Order", vec![var("id", "1001"), customer])]);
        let mut expanded = BTreeSet::from([path(&["Locals"]), path(&["Locals", "$order", "customer"])]);
        assert!(s.unloaded(&[], &expanded).is_empty(), "hidden under a collapsed parent");

        expanded.insert(path(&["Locals", "$order"]));
        let customer = vec![(path(&["Locals", "$order", "customer"]), 7)];
        assert_eq!(s.unloaded(&path(&["Locals", "$order"]), &expanded), customer);
        assert_eq!(s.unloaded(&[], &expanded), customer);
        assert!(s.unloaded(&path(&["Locals", "$other"]), &expanded).is_empty());
    }

    #[test]
    fn expanded_scope_without_variables_is_unloaded() {
        let mut s = snap("f", vec![]);
        s.scopes[0].vars = None;
        let expanded = BTreeSet::from([path(&["Locals"]), path(&["Locals", "$order"])]);
        assert_eq!(s.unloaded(&[], &expanded), [(path(&["Locals"]), 1)]);
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
