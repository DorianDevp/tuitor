//! Reads the program state at a stop: stack, scopes of the top frame, variables
//! (including paths the user keeps expanded) and watch expressions.

use std::path::{Path, PathBuf};

use tuitor_dap::requests::{Evaluate, Scopes, StackTrace, Threads, Variables};
use tuitor_dap::types::{
    Empty, EvaluateArguments, Scope, ScopesArguments, StackFrame, StackTraceArguments, VariablesArguments,
};
use tuitor_dap::{Client, Error};

use crate::snapshot::{Frame, ScopeVars, Var, VarPath, WatchValue, is_library_path};

const MAX_FRAMES: i64 = 200;

pub(crate) struct Capture {
    pub thread_id: i64,
    pub frames: Vec<Frame>,
    pub scopes: Vec<ScopeVars>,
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
    let frames: Vec<Frame> = trace.stack_frames.into_iter().map(frame).collect();

    let mut scopes = Vec::new();
    let mut watch_values = Vec::with_capacity(watches.len());
    if let Some(top) = frames.first() {
        let response = client.request::<Scopes>(ScopesArguments { frame_id: top.id }).await?;
        for (index, scope) in response.scopes.into_iter().enumerate() {
            let path = vec![scope.name.clone()];
            let open_by_default = is_default_scope(&scope, index) && !seen_scopes.contains(&scope.name);
            let wanted = open_by_default || expanded.contains(&path);
            let vars = if wanted {
                let mut vars = fetch_vars(&client, scope.variables_reference).await.unwrap_or_default();
                expand_paths(&client, &scope.name, &mut vars, &expanded).await;
                Some(vars)
            } else {
                None
            };
            scopes.push(ScopeVars {
                default_open: is_default_scope(&scope, index),
                name: scope.name,
                reference: scope.variables_reference,
                expensive: scope.expensive,
                vars,
            });
        }

        for expr in watches {
            let value = evaluate(&client, &expr, Some(top.id), "watch").await.map_err(|e| short_error(&e));
            watch_values.push(WatchValue { expr, value });
        }
    }

    Ok(Capture { thread_id, frames, scopes, watches: watch_values })
}

pub(crate) async fn fetch_vars(client: &Client, reference: i64) -> Result<Vec<Var>, Error> {
    let response = client.request::<Variables>(VariablesArguments { variables_reference: reference }).await?;
    Ok(response
        .variables
        .into_iter()
        .map(|v| Var {
            name: v.name,
            value: v.value,
            type_name: v.type_name.filter(|t| !t.is_empty()),
            evaluate_name: v.evaluate_name,
            reference: v.variables_reference,
            children: None,
        })
        .collect())
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
    }
}

/// Breakpoints are stored canonicalized; frames must be too for them to match.
pub(crate) fn canonical(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
