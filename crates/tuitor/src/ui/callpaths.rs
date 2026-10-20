//! PATHS: while the language server is asked, what it is doing; then a line per path, from
//! where execution starts to the function of the SOURCE line, and the breakpoints of the
//! selected path under the functions they are in, with the code of each line.

use std::path::Path;
use std::time::Duration;

use ratatui::layout::Rect;
use tuitor_code::{CallPath, Entry, Root, Why};

use super::paint::{Painter, Panel, Seg, seg, truncate_left, wrap};
use super::syntax::{Lang, highlight};
use super::theme::*;
use crate::app::App;
use crate::callpaths::Paths;

/// Rows of the list of paths, at most.
const LIST: u16 = 8;

pub fn draw(p: &mut Painter<'_>, app: &App, paths: &Paths, screen: Rect) {
    p.dim(screen);
    let width = screen.width.saturating_sub(8).min(150);
    let room = screen.height.saturating_sub(super::HEADER + 4);
    let height = (rows(paths, width.saturating_sub(6)) + 2).clamp(8.min(room), room);
    let rect = Rect::new((screen.width - width) / 2, super::HEADER + 1, width, height);
    p.fill(rect, BG);
    let right = match &paths.found {
        Some(Ok(found)) => {
            let n = found.paths.len();
            vec![seg(format!("{n} path{} · {} requests", if n == 1 { "" } else { "s" }, found.requests), fg(FAINT))]
        }
        _ => vec![seg(format!("{:.1} s", paths.started.elapsed().as_secs_f64()), fg(FAINT))],
    };
    let to = match &paths.found {
        Some(Ok(found)) => format!("to {} · {}", found.target.name, paths.place),
        _ => format!("to {}", paths.place),
    };
    p.panel(rect, &Panel { title: "PATHS", sub: vec![seg(to, fg(DIM))], right, keys: vec![], focus: true });
    let inner = super::paint::inner(rect);
    match &paths.found {
        None => asking(p, app, paths, inner),
        Some(Err(err)) => {
            for (i, line) in wrap(err, usize::from(inner.width.saturating_sub(4))).iter().enumerate() {
                p.text(inner.x + 2, inner.y + 1 + i as u16, line, fg(RED), inner.right());
            }
            footer(p, inner, &[("esc", "close")]);
        }
        Some(Ok(found)) if found.paths.is_empty() => {
            p.text(inner.x + 2, inner.y + 1, "no path found", italic(FAINT), inner.right());
            notes(p, &found.notes, inner, inner.bottom().saturating_sub(1));
            footer(p, inner, &[("esc", "close")]);
        }
        Some(Ok(found)) => {
            let list_rows = (found.paths.len() as u16).min(LIST);
            list(p, paths, Rect::new(inner.x, inner.y, inner.width, list_rows));
            let rule = inner.y + list_rows;
            p.text(inner.x, rule, &"─".repeat(usize::from(inner.width)), fg(GHOST), inner.right());
            let notes_y = notes(p, &found.notes, inner, inner.bottom().saturating_sub(1));
            if let Some(path) = found.paths.get(paths.selected) {
                let area = Rect::new(inner.x, rule + 1, inner.width, notes_y.saturating_sub(rule + 1));
                detail(p, app, paths, path, area);
            }
            let marked = paths.marked.iter().filter(|m| **m).count();
            let enter = match (marked, paths.sets.len()) {
                (_, 0) => "no set colour is free: D in BREAKPOINTS deletes a set".to_owned(),
                (0, _) => "mark a path first".to_owned(),
                (1, _) => "a breakpoint set for the path marked".to_owned(),
                (n, free) => format!("a breakpoint set for each of the {n} paths marked ({free} colours free)"),
            };
            footer(p, inner, &[("⏎", &enter), ("space", "mark"), ("↑/↓", "select"), ("esc", "close")]);
        }
    }
}

/// The rows PATHS needs inside its frame: the list, the longest path's lines, the notes and
/// the keys, so that it keeps its size from one path to the next.
fn rows(paths: &Paths, width: u16) -> u16 {
    let Some(Ok(found)) = &paths.found else { return 6 };
    let list = found.paths.len().min(usize::from(LIST));
    let detail = found.paths.iter().map(|path| path.steps.len() + path.stops.len()).max().unwrap_or(1);
    let notes: usize = found.notes.iter().map(|note| wrap(note, usize::from(width)).len()).sum();
    let notes = if notes > 0 { notes + 1 } else { 0 };
    u16::try_from(list + 1 + detail + notes + 2).unwrap_or(u16::MAX)
}

/// While the server is asked: what it says it is doing.
fn asking(p: &mut Painter<'_>, app: &App, paths: &Paths, inner: Rect) {
    // Again soon, to show what it says next.
    app.ui.animate(Duration::from_millis(100));
    let doing = if paths.progress.is_empty() { "starting…" } else { paths.progress.as_str() };
    p.text(inner.x + 2, inner.y + 1, doing, fg(FG), inner.right());
    let note = "esc stops asking; the language server stays up for the next time";
    p.text(inner.x + 2, inner.y + 3, note, italic(FAINT), inner.right());
    footer(p, inner, &[("esc", "stop")]);
}

/// A line per path: marked or not, where it starts, its functions.
fn list(p: &mut Painter<'_>, paths: &Paths, area: Rect) {
    let all = paths.paths();
    let offset = paths.selected.saturating_sub(usize::from(area.height).saturating_sub(1));
    for (i, path) in all.iter().enumerate().skip(offset).take(usize::from(area.height)) {
        let y = area.y + (i - offset) as u16;
        let selected = i == paths.selected;
        if selected {
            p.band(area.x, y, area.width, SEL2);
            p.text(area.x + 1, y, "›", bold(AMBER), area.right());
        }
        // A path marked in the colour of the set it is to be.
        match paths.set_of(i) {
            Some(set) => p.text(area.x + 3, y, "■", fg(set_color(set, true)), area.right()),
            None => p.text(area.x + 3, y, "□", fg(FAINT), area.right()),
        };
        p.text(area.x + 5, y, &format!("{:>2}", i + 1), fg(FAINT), area.right());
        p.text(area.x + 8, y, root(path.root), fg(DIM), area.right());
        let lines = format!("{} lines", path.stops.len());
        let end = area.right().saturating_sub(lines.len() as u16 + 2);
        p.text(end + 1, y, &lines, fg(FAINT), area.right());
        p.segs(area.x + 20, y, &chain(path, selected), end.saturating_sub(1));
    }
}

fn root(root: Root) -> &'static str {
    match root {
        Root::Entry(Entry::Script) => "script",
        Root::Entry(Entry::Main) => "main",
        Root::Entry(Entry::Controller) => "controller",
        Root::Entry(Entry::Test) => "test",
        Root::Uncalled => "no caller",
        Root::Cut => "cut short",
    }
}

/// `A → B ⇢ C`: a dashed arrow, in amber, for a call only a run showed.
fn chain(path: &CallPath, selected: bool) -> Vec<Seg<'static>> {
    let mut segs = Vec::new();
    for (i, step) in path.steps.iter().enumerate() {
        if i > 0 {
            segs.push(if step.observed { seg(" ⇢ ", bold(AMBER_DIM)) } else { seg(" → ", fg(FAINT)) });
        }
        let last = i + 1 == path.steps.len();
        let style = if last || selected { fg(WHITE) } else { fg(FG) };
        segs.push(seg(step.name.clone(), style));
    }
    segs
}

/// The functions of the selected path, each with its lines.
fn detail(p: &mut Painter<'_>, app: &App, paths: &Paths, path: &CallPath, area: Rect) {
    let Some(tab) = app.tabs.get(paths.tab) else { return };
    let workspace = tab.engine.workspace();
    let mut y = area.y;
    let (x_line, x_why, x_code) = (area.x + 4, area.x + 10, area.x + 32);
    for (k, step) in path.steps.iter().enumerate() {
        if y >= area.bottom() {
            p.text(area.x + 2, area.bottom().saturating_sub(1), "…", fg(FAINT), area.right());
            return;
        }
        let x = p.text(area.x + 2, y, &step.name, bold(FG), area.right());
        let rel = step.path.strip_prefix(workspace).unwrap_or(&step.path);
        let at = format!("  {}:{}", rel.display(), step.line);
        let x = p.text(x, y, &truncate_left(&at, usize::from(area.right().saturating_sub(x))), fg(FAINT), area.right());
        if step.observed {
            p.text(x, y, "  ⇢ called from the step above at run time", italic(AMBER_DIM), area.right());
        }
        y += 1;
        for stop in path.stops.iter().filter(|stop| stop.step == k) {
            if y >= area.bottom() {
                break;
            }
            let canonical = std::fs::canonicalize(&stop.path).unwrap_or_else(|_| stop.path.clone());
            let set = tab.engine.breakpoints().at(&canonical, stop.line).is_some();
            p.text(
                area.x + 2,
                y,
                if set { "●" } else { "○" },
                if set { bold(RED_DIM) } else { fg(FAINT) },
                area.right(),
            );
            p.text(x_line, y, &format!("{:>4}", stop.line), fg(FAINT), area.right());
            let (why, style) = reason(&stop.why);
            p.text(x_why, y, &why, style, x_code.saturating_sub(1));
            code(p, app, &stop.path, stop.line, Rect::new(x_code, y, area.right().saturating_sub(x_code), 1));
            y += 1;
        }
    }
}

fn reason(why: &Why) -> (String, ratatui::style::Style) {
    match why {
        Why::Target => ("the line asked about".into(), bold(AMBER)),
        Why::Calls(name) => (format!("calls {name}"), fg(FG)),
        Why::Entry => ("entry".into(), fg(DIM)),
        Why::Writes(name) => (format!("writes {name}"), fg(DIM)),
        Why::Uses(name) => (format!("uses {name}"), fg(DIM)),
    }
}

/// Line `line` of `path`, coloured, without its indentation.
fn code(p: &mut Painter<'_>, app: &App, path: &Path, line: u32, area: Rect) {
    let Some(file) = app.sources.borrow_mut().get(path) else { return };
    let Some(text) = usize::try_from(line).ok().and_then(|n| file.lines.get(n.checked_sub(1)?)) else { return };
    let mut x = area.x;
    for (piece, style) in highlight(text.trim_start(), Lang::from_path(path)) {
        x = p.text(x, area.y, &piece, style, area.right());
    }
}

/// The notes, wrapped, above the footer; returns the row they start at.
fn notes(p: &mut Painter<'_>, notes: &[String], inner: Rect, bottom: u16) -> u16 {
    let room = usize::from(inner.width.saturating_sub(4));
    let lines: Vec<String> = notes.iter().flat_map(|note| wrap(note, room)).collect();
    let top = bottom.saturating_sub(lines.len() as u16 + u16::from(!lines.is_empty()));
    for (i, line) in lines.iter().enumerate() {
        p.text(inner.x + 2, top + 1 + i as u16, line, italic(FAINT), inner.right());
    }
    top
}

fn footer(p: &mut Painter<'_>, inner: Rect, actions: &[(&str, &str)]) {
    let y = inner.bottom().saturating_sub(1);
    let mut x = inner.x + 1;
    for (key, label) in actions {
        x = p.text(x, y, key, bold(KEY), inner.right());
        x = p.text(x, y, &format!(" {label}   "), fg(FAINT), inner.right());
    }
}
