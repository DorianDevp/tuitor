//! OUTLINE: the functions of the file in SOURCE with their lines and their part in the
//! flow, then where the function on view was called from and what it calls, as the
//! recorded stacks and the trace show it.

use std::collections::HashSet;
use std::path::Path;

use ratatui::layout::Rect;
use tuitor_core::flow::{Flow, Status};

use super::paint::{Painter, Panel, Seg, seg};
use super::theme::*;
use crate::app::{App, Pane};
use crate::flow::short_function;
use crate::outline::{Symbol, around, owner_name, short_name};

pub fn draw(p: &mut Painter<'_>, app: &App, area: Rect, focus: bool) {
    let tab = app.tab();
    let path = tab.source_path();
    let name = path.as_deref().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned());
    let sub = name.map(|n| vec![seg(n, fg(DIM))]).unwrap_or_default();
    let keys = if focus { vec![("⏎", "jump"), ("b", "bp")] } else { vec![] };
    p.panel(area, &Panel { title: "OUTLINE", sub, right: vec![], keys, focus });
    let inner = super::paint::inner(area);
    let Some(path) = path else { return };
    let Some(source) = app.sources.borrow_mut().get(&path) else { return };
    let symbols = source.outline();
    if symbols.is_empty() {
        p.text(inner.x + 1, inner.y, "no functions found in this file", fg(FAINT), inner.right());
        return;
    }
    let view = tab.flow();
    let flow = view.as_ref().map(|v| &v.flow);
    let shown = app.shown_symbol();

    // The list gets what the calls below leave it, and at least a few lines.
    let calls = flow.zip(shown.as_ref()).map(|(flow, symbol)| relations(app, flow, &path, symbol)).unwrap_or_default();
    let below = (calls.len() + 2).min(usize::from(inner.height) / 2);
    let height = usize::from(inner.height).saturating_sub(below).max(3).min(usize::from(inner.height));
    let offset = tab.ui.outline.window(height, symbols.len());
    let line_w = symbols.iter().map(|s| s.line.to_string().len()).max().unwrap_or(1) as u16 + 1;
    let longest = symbols.iter().map(|s| s.name.chars().count()).max().unwrap_or(0) as u16;
    let name_w = longest.clamp(6, inner.width.saturating_sub(line_w + 12).max(6));
    for (i, symbol) in symbols.iter().enumerate().skip(offset).take(height) {
        let y = inner.y + (i - offset) as u16;
        let selected = if focus { i == tab.ui.outline.pos } else { shown.as_ref() == Some(symbol) };
        if selected {
            p.band(inner.x, y, inner.width, if focus { SEL2 } else { SEL });
        }
        let (glyph, note) = symbol_status(app, flow, &path, symbol);
        p.text(inner.x + 1, y, "ƒ", fg(GHOST), inner.right());
        let name_style = if selected { bold(WHITE) } else { fg(FG) };
        p.text(inner.x + 3, y, &symbol.name, name_style, inner.x + 3 + name_w);
        let x = inner.x + 4 + name_w;
        p.text(x, y, &format!(":{}", symbol.line), fg(FAINT), inner.right());
        let x = x + line_w + 1;
        let x = p.segs(x, y, &[glyph], inner.right());
        p.segs(x + 1, y, &note, inner.right());
    }

    let legend = "│ line reached: stop or trace";
    if usize::from(inner.height) > height.min(symbols.len()) + calls.len() + 2 {
        p.text(inner.x + 1, inner.bottom() - 1, legend, fg(GHOST), inner.right());
    }
    let (Some(symbol), false) = (shown, calls.is_empty()) else { return };
    let top = inner.y + height.min(symbols.len()) as u16 + 1;
    for (y, line) in (top..inner.bottom()).zip(calls) {
        let line = match line {
            Relation::Heading(text) => vec![seg(text, bold(FAINT)), seg(format!("  {}", symbol.name), fg(DIM))],
            Relation::Call(segs) => segs,
        };
        p.segs(inner.x + 1, y, &line, inner.right());
    }
}

/// How a function takes part in the flow: on the viewed stop's stack, run or predicted by
/// the evidence, or `None` when nothing saw it. `is` tells its name as stacks and traces
/// write it.
fn function_status(app: &App, flow: Option<&Flow>, path: &Path, is: &dyn Fn(&str) -> bool) -> Option<Status> {
    let frames = app.tab().viewed().map(|s| s.frames.as_slice()).unwrap_or_default();
    match frames.iter().position(|f| f.path.as_deref() == Some(path) && is(&f.name)) {
        Some(0) => return Some(Status::Executing),
        Some(_) => return Some(Status::Stack),
        None => {}
    }
    let flow = flow?;
    let row = flow.row(path)?;
    let (name, _) = row.functions.iter().find(|(name, _)| is(name))?;
    let later = row.first_calls.get(name).is_some_and(|at| *at >= flow.now);
    Some(if row.status == Status::Predicted || later { Status::Predicted } else { Status::Ran })
}

fn glyph(status: Option<Status>) -> Seg<'static> {
    match status {
        Some(Status::Executing) => seg("▶", bold(AMBER)),
        Some(Status::Stack) => seg("▷", fg(FG)),
        Some(Status::Ran) => seg("✓", fg(FAINT)),
        Some(Status::Predicted) => seg("◇", fg(GHOST)),
        None => seg(" ", fg(GHOST)),
    }
}

/// A function's glyph, and its breakpoint, or that nothing saw it run.
fn symbol_status(app: &App, flow: Option<&Flow>, path: &Path, symbol: &Symbol) -> (Seg<'static>, Vec<Seg<'static>>) {
    let status = function_status(app, flow, path, &|name| same(name, symbol));
    let bp = app.tab().engine.breakpoints().in_file(path).find(|b| (symbol.line..=symbol.end).contains(&b.line));
    let mut note = Vec::new();
    if let Some(bp) = bp {
        note.push(seg("●", bold(RED_DIM)));
        note.push(seg(format!(":{} ", bp.line), fg(DIM)));
    }
    if status.is_none() {
        note.push(seg("unseen", fg(GHOST)));
    }
    (glyph(status), note)
}

/// Whether a function name from a stack or a trace stands for `symbol`.
pub fn same(function: &str, symbol: &Symbol) -> bool {
    short_name(function) == symbol.name
        && match (owner_name(function), &symbol.container) {
            (Some(owner), Some(container)) => owner == *container,
            _ => true,
        }
}

enum Relation {
    Heading(&'static str),
    Call(Vec<Seg<'static>>),
}

/// CALLED FROM and CALLS of `symbol` in the file at `path`.
fn relations(app: &App, flow: &Flow, path: &Path, symbol: &Symbol) -> Vec<Relation> {
    let status = |name: &str, file: &Path| {
        let mut glyph = glyph(function_status(app, Some(flow), file, &|n| n == name));
        glyph.text.to_mut().push(' ');
        glyph
    };
    let file = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let callers: Vec<Relation> = flow
        .edges
        .iter()
        .filter(|e| e.callee_path == path && same(&e.callee, symbol))
        .filter(|e| seen.insert((e.caller.clone(), e.site.clone())))
        .map(|e| {
            Relation::Call(vec![
                status(&e.caller, &e.site.path),
                seg(short_function(&e.caller, &e.site.path), fg(FG)),
                seg(format!(" {}:{}", file(&e.site.path), e.site.line), fg(DIM)),
            ])
        })
        .collect();
    if !callers.is_empty() {
        out.push(Relation::Heading("CALLED FROM"));
        out.extend(callers);
    }
    let mut seen = HashSet::new();
    let calls: Vec<Relation> = flow
        .edges
        .iter()
        .filter(|e| e.site.path == path && (symbol.line..=symbol.end).contains(&e.site.line))
        .filter(|e| seen.insert((e.callee.clone(), e.site.line)))
        .map(|e| {
            Relation::Call(vec![
                status(&e.callee, &e.callee_path),
                seg(short_function(&e.callee, &e.callee_path), fg(FG)),
                seg(format!(" :{}", e.site.line), fg(DIM)),
            ])
        })
        .collect();
    if !calls.is_empty() {
        out.push(Relation::Heading("CALLS"));
        out.extend(calls);
    }
    out
}

impl App {
    /// The function OUTLINE is about: the one under its cursor while it has focus, else
    /// the one around the SOURCE cursor.
    pub fn shown_symbol(&self) -> Option<Symbol> {
        let tab = self.tab();
        let source = self.sources.borrow_mut().get(&tab.source_path()?)?;
        let symbols = source.outline();
        if tab.focus() == Pane::Outline {
            return symbols.get(tab.ui.outline.pos).cloned();
        }
        around(symbols, tab.ui.source.cursor).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::same;
    use crate::outline::Symbol;

    #[test]
    fn names_match_their_symbol_and_class() {
        let add = Symbol { name: "add".into(), container: Some("Money".into()), line: 3, end: 5 };
        assert!(same("App\\Domain\\Money->add", &add));
        assert!(same("add", &add));
        assert!(!same("App\\Domain\\Ledger->add", &add));
        assert!(!same("App\\Domain\\Money->sub", &add));
    }
}
