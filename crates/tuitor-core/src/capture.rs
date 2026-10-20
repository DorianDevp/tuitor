//! Reads the program state at a stop: stack, scopes of the top frame, variables
//! (including paths the user keeps expanded) and watch expressions; and the scopes of
//! another frame, once it is picked.
//!
//! Requests that do not depend on each other's answers go out together: every one is
//! written before the first answer is awaited, and the answers are taken in the order
//! asked, so a snapshot comes out the same as if they had been asked one by one. An adapter
//! that answers slowly (js-debug: 45 to 90 ms a `variables`) then costs a round per level.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use tuitor_dap::requests::{Evaluate, Scopes, StackTrace, Threads, Variables};
use tuitor_dap::types::{
    Empty, EvaluateArguments, EvaluateResponse, Scope, ScopesArguments, StackFrame, StackTraceArguments, Variable,
    VariablesArguments,
};
use tuitor_dap::{Client, Error, Pending};

use crate::snapshot::{Frame, ScopeVars, Var, VarPath, WatchValue, is_library_path};

const MAX_FRAMES: i64 = 200;
/// `variables` requests per stop for reading children of collapsed arguments and locals
/// ahead, so that a change inside a collapsed object shows.
const PREFETCH_REQUESTS: usize = 30;
/// Variables announcing more children than this are not read ahead: one level of a
/// large array costs more than the hint is worth.
const PREFETCH_MAX_CHILDREN: i64 = 100;

pub(crate) struct Capture {
    pub thread_id: i64,
    /// The top frame with its scopes read.
    pub frames: Vec<Frame>,
    pub watches: Vec<WatchValue>,
}

pub(crate) async fn capture(
    client: Client,
    thread: Option<i64>,
    watches: Vec<String>,
    expanded: Vec<VarPath>,
    seen_scopes: Vec<String>,
) -> Result<Capture, Error> {
    let thread_id = match thread {
        Some(id) => id,
        None => {
            let threads = client.request::<Threads>(Empty {}).await?;
            threads
                .threads
                .first()
                .map(|t| t.id)
                .ok_or(Error::Failed { command: "threads".into(), message: "the adapter reported no threads".into() })?
        }
    };

    let trace =
        client.request::<StackTrace>(StackTraceArguments { thread_id, start_frame: 0, levels: MAX_FRAMES }).await?;
    let mut frames: Vec<Frame> = trace.stack_frames.into_iter().map(frame).collect();

    let mut watch_values = Vec::with_capacity(watches.len());
    if let Some(top) = frames.first_mut() {
        top.scopes = Some(scopes(&client, top.id, &expanded, &seen_scopes).await?);
        // After the variables, all at once: a watch that calls a function may change them.
        let asked: Vec<_> = watches.iter().map(|expr| evaluation(&client, expr, Some(top.id), "watch")).collect();
        for (expr, asked) in watches.into_iter().zip(asked) {
            let value = answer(asked).await.map(|r| r.result).map_err(|e| short_error(&e));
            watch_values.push(WatchValue { expr, value });
        }
    }

    Ok(Capture { thread_id, frames, watches: watch_values })
}

/// The scopes of frame `frame_id`. Those shown by default are read with their variables,
/// one level of their children ahead and the paths in `expanded`, unless the user collapsed
/// them (`seen_scopes`); any other only when the user expanded it.
pub(crate) async fn scopes(
    client: &Client,
    frame_id: i64,
    expanded: &[VarPath],
    seen_scopes: &[String],
) -> Result<Vec<ScopeVars>, Error> {
    let response = client.request::<Scopes>(ScopesArguments { frame_id }).await?;
    let mut scopes: Vec<ScopeVars> = response
        .scopes
        .into_iter()
        .enumerate()
        .map(|(index, scope)| ScopeVars {
            default_open: is_default_scope(&scope, index),
            name: scope.name,
            reference: scope.variables_reference,
            expensive: scope.expensive,
            vars: None,
        })
        .collect();
    let wanted: Vec<usize> = (0..scopes.len())
        .filter(|&i| {
            let scope = &scopes[i];
            (scope.default_open && !seen_scopes.contains(&scope.name)) || expanded.contains(&vec![scope.name.clone()])
        })
        .collect();
    let references: Vec<i64> = wanted.iter().map(|&i| scopes[i].reference).collect();
    for (&i, vars) in wanted.iter().zip(fetch_all(client, &references).await) {
        scopes[i].vars = Some(vars.unwrap_or_default());
    }
    expand_paths(client, &mut scopes, expanded).await;
    prefetch_children(client, &mut scopes).await;
    // Paths kept expanded under a collapsed variable are reachable now that its children
    // were read ahead, and are part of this stop when it is reopened.
    expand_paths(client, &mut scopes, expanded).await;
    Ok(scopes)
}

/// The variables under each of `references`, asked for together, in the same order.
async fn fetch_all(client: &Client, references: &[i64]) -> Vec<Result<Vec<Var>, Error>> {
    let asked: Vec<_> = references
        .iter()
        .map(|&reference| client.call::<Variables>(VariablesArguments { variables_reference: reference }))
        .collect();
    let mut out = Vec::with_capacity(asked.len());
    for asked in asked {
        out.push(answer(asked).await.map(|r| r.variables.into_iter().map(var).collect()));
    }
    out
}

/// The answer to a request [`Client::call`] wrote, or why it could not be written.
async fn answer<T: DeserializeOwned>(asked: Result<Pending<T>, Error>) -> Result<T, Error> {
    asked?.await
}

/// A variable as a snapshot keeps it; its children are read separately.
pub(crate) fn var(v: Variable) -> Var {
    let child_count = match (v.named_variables, v.indexed_variables) {
        (None, None) => None,
        (named, indexed) => Some(named.unwrap_or(0) + indexed.unwrap_or(0)),
    };
    Var {
        name: v.name,
        value: v.value,
        type_name: v.type_name.filter(|t| !t.is_empty()),
        evaluate_name: v.evaluate_name,
        reference: v.variables_reference,
        child_count,
        children: None,
    }
}

pub(crate) async fn evaluate(
    client: &Client,
    expr: &str,
    frame_id: Option<i64>,
    context: &str,
) -> Result<String, Error> {
    Ok(answer(evaluation(client, expr, frame_id, context)).await?.result)
}

/// Writes an `evaluate` request now.
fn evaluation(
    client: &Client,
    expr: &str,
    frame_id: Option<i64>,
    context: &str,
) -> Result<Pending<EvaluateResponse>, Error> {
    let args = EvaluateArguments { expression: expr.to_owned(), frame_id, context: context.to_owned() };
    client.call::<Evaluate>(args)
}

/// Arguments and locals are read at every stop; registers, globals, superglobals
/// and the like only once the user expands them. Adapters mark these
/// inconsistently (gdb calls registers cheap), so names are checked too.
fn is_default_scope(scope: &Scope, index: usize) -> bool {
    if scope.expensive {
        return false;
    }
    if let Some(hint) = scope.presentation_hint.as_deref() {
        return matches!(hint, "arguments" | "locals" | "returnValue");
    }
    let name = scope.name.to_ascii_lowercase();
    index == 0 || matches!(name.as_str(), "locals" | "local" | "arguments" | "args" | "return value")
}

/// Adapter errors for a bad watch expression are long; keep the first line.
pub(crate) fn short_error(err: &Error) -> String {
    let text = match err {
        Error::Failed { message, .. } => message.clone(),
        other => other.to_string(),
    };
    text.lines().next().unwrap_or_default().to_owned()
}

/// Reads the children of every path in `expanded` that are not read, a level at a time,
/// parents first, each level's together.
async fn expand_paths(client: &Client, scopes: &mut [ScopeVars], expanded: &[VarPath]) {
    let mut paths: Vec<&VarPath> = expanded.iter().filter(|p| p.len() > 1).collect();
    paths.sort_by_key(|p| p.len());
    for level in paths.chunk_by(|a, b| a.len() == b.len()) {
        let unread: Vec<(&VarPath, i64)> = level
            .iter()
            .filter_map(|path| {
                let var = find_var(scopes, path)?;
                (var.reference > 0 && var.children.is_none()).then_some((*path, var.reference))
            })
            .collect();
        let references: Vec<i64> = unread.iter().map(|(_, reference)| *reference).collect();
        for ((path, _), children) in unread.iter().zip(fetch_all(client, &references).await) {
            if let Some(var) = find_var(scopes, path) {
                var.children = children.ok();
            }
        }
    }
}

/// Reads one level of children of the top-level variables of the scopes shown by default
/// that are not expanded, so that `diff` can mark an object whose fields changed even while
/// it is collapsed. Children are shown only when the user expands the variable. Each read
/// takes one request out of a budget, in order, so the cost per stop stays bounded.
async fn prefetch_children(client: &Client, scopes: &mut [ScopeVars]) {
    let mut budget = PREFETCH_REQUESTS;
    let mut read: Vec<(usize, usize)> = Vec::new();
    for (s, scope) in scopes.iter().enumerate().filter(|(_, scope)| scope.default_open) {
        for (v, var) in scope.vars.iter().flatten().enumerate() {
            if worth_prefetching(var) && take(&mut budget) {
                read.push((s, v));
            }
        }
    }
    read_children(client, scopes, &read, false).await;
    // A pointer or reference has one child, what it points to (gdb: `*p` or `value`). Its
    // fields are read too, so a change made through a pointer shows like one in a struct.
    let mut pointers = Vec::new();
    for (s, v) in read {
        let one = top_var(scopes, s, v, false).is_some_and(|var| matches!(var.children.as_deref(), Some([_])));
        if one && top_var(scopes, s, v, true).is_some_and(|pointee| worth_prefetching(pointee)) && take(&mut budget) {
            pointers.push((s, v));
        }
    }
    read_children(client, scopes, &pointers, true).await;
}

/// Reads the children of top-level variables `at` (scope, variable), or with `pointee`
/// those of their one child, together.
async fn read_children(client: &Client, scopes: &mut [ScopeVars], at: &[(usize, usize)], pointee: bool) {
    let mut references = Vec::with_capacity(at.len());
    for &(s, v) in at {
        references.extend(top_var(scopes, s, v, pointee).map(|var| var.reference));
    }
    for (&(s, v), children) in at.iter().zip(fetch_all(client, &references).await) {
        if let Some(var) = top_var(scopes, s, v, pointee) {
            var.children = children.ok();
        }
    }
}

/// Top-level variable `v` of scope `s`, or with `pointee` its only child.
fn top_var(scopes: &mut [ScopeVars], s: usize, v: usize, pointee: bool) -> Option<&mut Var> {
    let var = scopes.get_mut(s)?.vars.as_mut()?.get_mut(v)?;
    if pointee { var.children.as_mut()?.first_mut() } else { Some(var) }
}

/// The variable at `path`, from its scope's name down.
fn find_var<'a>(scopes: &'a mut [ScopeVars], path: &[String]) -> Option<&'a mut Var> {
    let (scope, names) = path.split_first()?;
    let vars = scopes.iter_mut().find(|s| &s.name == scope)?.vars.as_deref_mut()?;
    find_mut(vars, names)
}

fn worth_prefetching(var: &Var) -> bool {
    var.reference > 0 && var.children.is_none() && var.child_count.is_none_or(|n| n <= PREFETCH_MAX_CHILDREN)
}

fn take(budget: &mut usize) -> bool {
    let Some(left) = budget.checked_sub(1) else { return false };
    *budget = left;
    true
}

fn find_mut<'a>(vars: &'a mut [Var], path: &[String]) -> Option<&'a mut Var> {
    let (first, rest) = path.split_first()?;
    let var = vars.iter_mut().find(|v| &v.name == first)?;
    if rest.is_empty() { Some(var) } else { find_mut(var.children.as_deref_mut()?, rest) }
}

fn frame(f: StackFrame) -> Frame {
    let source = f.source.unwrap_or_default();
    let path = source.path.as_deref().map(canonical);
    let library = f.presentation_hint.as_deref() == Some("subtle")
        || source.presentation_hint.as_deref() == Some("deemphasize")
        || path.as_deref().is_some_and(is_library_path);
    Frame {
        id: f.id,
        name: f.name,
        path,
        source_name: source.name,
        source_reference: source.source_reference.filter(|r| *r > 0),
        line: u32::try_from(f.line).unwrap_or(0),
        column: u32::try_from(f.column).unwrap_or(0),
        library,
        scopes: None,
    }
}

/// Breakpoints are stored canonicalized; frames must be too for them to match.
pub(crate) fn canonical(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
