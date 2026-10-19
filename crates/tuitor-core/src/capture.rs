//! Reads the program state at a stop: stack, scopes of the top frame, variables
//! (including paths the user keeps expanded) and watch expressions; and the scopes of
//! another frame, once it is picked.

use std::path::{Path, PathBuf};

use tuitor_dap::requests::{Evaluate, Scopes, StackTrace, Threads, Variables};
use tuitor_dap::types::{
    Empty, EvaluateArguments, Scope, ScopesArguments, StackFrame, StackTraceArguments, Variable, VariablesArguments,
};
use tuitor_dap::{Client, Error};

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
        for expr in watches {
            let value = evaluate(&client, &expr, Some(top.id), "watch").await.map_err(|e| short_error(&e));
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
    let mut scopes = Vec::new();
    let mut prefetch_budget = PREFETCH_REQUESTS;
    for (index, scope) in response.scopes.into_iter().enumerate() {
        let path = vec![scope.name.clone()];
        let default_open = is_default_scope(&scope, index);
        let wanted = (default_open && !seen_scopes.contains(&scope.name)) || expanded.contains(&path);
        let vars = if wanted {
            let mut vars = fetch_vars(client, scope.variables_reference).await.unwrap_or_default();
            expand_paths(client, &scope.name, &mut vars, expanded).await;
            if default_open {
                prefetch_children(client, &mut vars, &mut prefetch_budget).await;
                // Paths kept expanded under a collapsed variable are reachable now that its
                // children were read ahead, and are part of this stop when it is reopened.
                expand_paths(client, &scope.name, &mut vars, expanded).await;
            }
            Some(vars)
        } else {
            None
        };
        scopes.push(ScopeVars {
            default_open,
            name: scope.name,
            reference: scope.variables_reference,
            expensive: scope.expensive,
            vars,
        });
    }
    Ok(scopes)
}

pub(crate) async fn fetch_vars(client: &Client, reference: i64) -> Result<Vec<Var>, Error> {
    let response = client.request::<Variables>(VariablesArguments { variables_reference: reference }).await?;
    Ok(response.variables.into_iter().map(var).collect())
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
    let args = EvaluateArguments { expression: expr.to_owned(), frame_id, context: context.to_owned() };
    Ok(client.request::<Evaluate>(args).await?.result)
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

/// Loads children for every expanded path inside this scope, parents first.
async fn expand_paths(client: &Client, scope: &str, vars: &mut [Var], expanded: &[VarPath]) {
    let mut paths: Vec<&[String]> = expanded.iter().filter(|p| p.len() > 1 && p[0] == scope).map(|p| &p[1..]).collect();
    paths.sort_by_key(|p| p.len());
    for path in paths {
        let Some(var) = find_mut(vars, path) else { continue };
        if var.reference > 0 && var.children.is_none() {
            var.children = fetch_vars(client, var.reference).await.ok();
        }
    }
}

/// Reads one level of children of top-level variables that are not expanded, so that
/// `diff` can mark an object whose fields changed even while it is collapsed. Children
/// are shown only when the user expands the variable. Each read takes one request out
/// of `budget`, in order, so the cost per stop stays bounded.
async fn prefetch_children(client: &Client, vars: &mut [Var], budget: &mut usize) {
    for var in vars.iter_mut().filter(|v| worth_prefetching(v)) {
        if !take(budget) {
            return;
        }
        var.children = fetch_vars(client, var.reference).await.ok();
        // A pointer or reference has one child, what it points to (gdb: `*p` or `value`).
        // Its fields are read too, so a change made through a pointer shows like one in
        // a struct.
        if let Some([pointee]) = var.children.as_deref_mut()
            && worth_prefetching(pointee)
            && take(budget)
        {
            pointee.children = fetch_vars(client, pointee.reference).await.ok();
        }
    }
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
