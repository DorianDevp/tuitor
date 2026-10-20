//! PATHS: while the language server is asked, what it is doing; then the call trees through
//! the SOURCE line, a stage a row, with the marks each stage is in, and below them the lines
//! worth a breakpoint in the stage under the cursor, with the code of each.

use std::path::Path;
use std::time::Duration;

use ratatui::layout::Rect;
use ratatui::style::Style;
use tuitor_code::{Cut, Entry, Found, Root, Why};
use tuitor_core::config::project_relative;

use super::paint::{Painter, Panel, seg, truncate_left, wrap};
use super::syntax::{Lang, highlight};
use super::theme::*;
use crate::app::{App, PATHS_PANE};
use crate::callpaths::Paths;

pub fn draw(p: &mut Painter<'_>, app: &App, paths: &Paths, screen: Rect) {
    p.dim(screen);
    let width = screen.width.saturating_sub(6).min(170);
    let room = screen.height.saturating_sub(super::HEADER + 3);
    let height = if paths.tree().is_some() { room } else { 8.min(room) };
    let rect = Rect::new((screen.width - width) / 2, super::HEADER + 1, width, height);
    p.fill(rect, BG);
    let right = match &paths.found {
        Some(Ok(found)) => {
            let (trees, ends) = (found.roots().count(), found.paths().len());
            let text = format!(
                "{trees} place{} execution starts · {ends} end-to-end path{} · {} requests",
                s(trees),
                s(ends),
                found.requests
            );
            vec![seg(text, fg(FAINT))]
        }
        _ => vec![seg(format!("{:.1} s", paths.started.elapsed().as_secs_f64()), fg(FAINT))],
    };
    let sub = match &paths.found {
        Some(Ok(found)) => vec![
            seg("through ", fg(DIM)),
            seg("◆ ", bold(AMBER)),
            seg(found.target().name.clone(), fg(WHITE)),
            seg(format!(" · {}", paths.place), fg(DIM)),
        ],
        _ => vec![seg(format!("through {}", paths.place), fg(DIM))],
    };
    p.panel(rect, &Panel { title: "PATHS", sub, right, keys: vec![], focus: true });
    let inner = super::paint::inner(rect);
    match &paths.found {
        None => asking(p, app, paths, inner),
        Some(Err(err)) => {
            for (i, line) in wrap(err, usize::from(inner.width.saturating_sub(4))).iter().enumerate() {
                p.text(inner.x + 2, inner.y + 1 + i as u16, line, fg(RED), inner.right());
            }
            footer(p, inner, &[("esc", "close")]);
        }
        Some(Ok(found)) if found.stages.is_empty() => {
            p.text(inner.x + 2, inner.y + 1, "no path found", italic(FAINT), inner.right());
            let notes = notes(found, usize::from(inner.width.saturating_sub(4)));
            let top = inner.bottom().saturating_sub(1 + notes.len() as u16);
            for (i, (line, style)) in notes.iter().enumerate() {
                p.text(inner.x + 2, top + i as u16, line, *style, inner.right());
            }
            footer(p, inner, &[("esc", "close")]);
        }
        Some(Ok(found)) => body(p, app, paths, found, inner),
    }
}

fn s(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
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

/// The marks, the trees, the lines of the stage under the cursor, the notes and the keys.
fn body(p: &mut Painter<'_>, app: &App, paths: &Paths, found: &Found, inner: Rect) {
    let foot = inner.bottom().saturating_sub(1);
    let notes = notes(found, usize::from(inner.width.saturating_sub(4)));
    let notes_h = notes.len() as u16;
    let lines = paths.lines_here().map(|(stops, _)| stops.len()).unwrap_or(0) as u16;
    // Below the marks, the trees take what they need, leaving the lines of a stage room for
    // theirs; the lines get what the trees leave.
    let free = foot.saturating_sub(inner.y + notes_h + 2);
    let wanted = (lines + 2).clamp(4, (free * 2 / 5).max(4)).min(free.saturating_sub(3));
    let tree_h = (paths.rows().len() as u16).clamp(3.min(free), free.saturating_sub(wanted));
    let detail_h = free.saturating_sub(tree_h);
    marks(p, app, paths, inner.x + 1, inner.y, inner.right());
    tree(p, app, paths, found, Rect::new(inner.x, inner.y + 1, inner.width, tree_h));
    let rule = inner.y + 1 + tree_h;
    p.text(inner.x, rule, &"─".repeat(usize::from(inner.width)), fg(GHOST), inner.right());
    detail(p, app, paths, found, Rect::new(inner.x, rule + 1, inner.width, detail_h));
    let top = foot.saturating_sub(notes_h);
    for (i, (line, style)) in notes.iter().enumerate() {
        p.text(inner.x + 2, top + i as u16, line, *style, inner.right());
    }
    let take = match paths.marks.len() {
        0 => "mark first".to_owned(),
        1 => "a breakpoint set of the mark".to_owned(),
        n => format!("a breakpoint set of each of the {n} marks"),
    };
    let key = |command: &str| app.keymap.key_for(PATHS_PANE, command).unwrap_or_else(|| format!("/{command}"));
    footer(p, inner, &[(&key("paths take"), &take), (&key("close"), "close")]);
}

/// The marks, in their colours, the current one first in bold; or how to make one.
fn marks(p: &mut Painter<'_>, app: &App, paths: &Paths, x: u16, y: u16, right: u16) {
    let key = |command: &str| app.keymap.key_for(PATHS_PANE, command).unwrap_or_else(|| format!("/{command}"));
    if paths.marks.is_empty() {
        let segs = [
            seg("nothing marked: ", fg(DIM)),
            seg(key("paths mark"), bold(KEY)),
            seg(" marks the end-to-end path through the stage under the cursor, ", fg(FAINT)),
            seg(key("paths stage"), bold(KEY)),
            seg(" a stage, ", fg(FAINT)),
            seg(key("paths branch"), bold(KEY)),
            seg(" a stage and all below it; each mark is a breakpoint set", fg(FAINT)),
        ];
        p.segs(x, y, &segs, right);
        return;
    }
    let found = paths.tree();
    let mut segs = vec![seg("marks ", fg(DIM))];
    for (i, mark) in paths.marks.iter().enumerate() {
        let current = paths.current == Some(i);
        let lines = found.map_or(0, |found| found.selection(&mark.stages).len());
        let name = paths.name(mark.set);
        segs.push(seg(if current { " ›" } else { "  " }, bold(AMBER)));
        segs.push(seg("■ ", fg(set_color(mark.set, true))));
        segs.push(seg(name, if current { bold(WHITE) } else { fg(FG) }));
        let what = format!(" {} stage{}, {} line{}", mark.stages.len(), s(mark.stages.len()), lines, s(lines));
        segs.push(seg(what, fg(FAINT)));
    }
    p.segs(x, y, &segs, right);
}

/// A row per stage on screen, each tree under the place it starts.
fn tree(p: &mut Painter<'_>, app: &App, paths: &Paths, found: &Found, area: Rect) {
    let rows = paths.rows();
    let height = usize::from(area.height);
    let at = rows.iter().position(|s| *s == paths.cursor).unwrap_or(0);
    let mut offset = paths.offset.get();
    if at < offset {
        offset = at;
    } else if at >= offset + height {
        offset = at + 1 - height;
    }
    offset = offset.min(rows.len().saturating_sub(height));
    paths.offset.set(offset);
    let workspace = app.tabs.get(paths.tab).map(|tab| tab.engine.workspace().to_path_buf()).unwrap_or_default();
    let current = paths.current.and_then(|c| paths.marks.get(c));
    let dots_w = (paths.marks.len() * 2) as u16;
    let x_dots = area.right().saturating_sub(dots_w + 1);
    let x_place = x_dots.saturating_sub(42).max(area.x + area.width / 2);
    for (row, &stage) in rows.iter().enumerate().skip(offset).take(height) {
        let y = area.y + (row - offset) as u16;
        let at = &found.stages[stage];
        let selected = stage == paths.cursor;
        if selected {
            p.band(area.x, y, area.width, SEL2);
            p.text(area.x + 1, y, "›", bold(AMBER), area.right());
        }
        // In the current mark or not.
        match current {
            Some(mark) if mark.stages.contains(&stage) => {
                p.text(area.x + 3, y, "■", fg(set_color(mark.set, true)), area.right())
            }
            Some(_) => p.text(area.x + 3, y, "□", fg(FAINT), area.right()),
            None => p.text(area.x + 3, y, "·", fg(GHOST), area.right()),
        };
        let mut segs = vec![seg(guides(found, stage), fg(GHOST))];
        let fold = match (at.children.is_empty(), paths.folded.contains(&stage)) {
            (true, _) => "  ",
            (false, true) => "▸ ",
            (false, false) => "▾ ",
        };
        segs.push(seg(fold, fg(DIM)));
        if at.observed {
            segs.push(seg("⇢ ", bold(AMBER_DIM)));
        }
        if at.anchor {
            segs.push(seg("◆ ", bold(AMBER)));
        }
        let on_path = found.on_path(stage);
        let style = match (at.anchor, selected, on_path, at.parent.is_none()) {
            (true, ..) => bold(WHITE),
            (_, true, ..) => fg(WHITE),
            (_, _, _, true) => bold(FG),
            (_, _, true, _) => fg(FG),
            _ => fg(DIM),
        };
        segs.push(seg(found.function(stage).name.clone(), style));
        if at.sites.len() > 1 {
            segs.push(seg(format!(" ×{}", at.sites.len()), fg(FAINT)));
        }
        if let Some(root) = at.root {
            segs.push(seg(format!("  {}", root_name(root)), fg(DIM)));
        }
        match at.cut {
            Some(Cut::Recursion) => segs.push(seg("  ↻ again", fg(FAINT))),
            Some(Cut::Deep | Cut::Spent) => segs.push(seg("  …", fg(FAINT))),
            None => {}
        }
        let forks = found.forks(stage);
        if forks > 1 {
            segs.push(seg("  ⋔", fg(AMBER_DIM)));
            segs.push(seg(format!(" {forks} ways"), italic(AMBER_DIM)));
        }
        p.segs(area.x + 6, y, &segs, x_place.saturating_sub(1));
        let function = found.function(stage);
        let place = format!("{}:{}", project_relative(&function.path, &workspace), function.line);
        let room = usize::from(x_dots.saturating_sub(x_place + 1));
        p.text(x_place, y, &truncate_left(&place, room), fg(GHOST), x_dots);
        for (k, mark) in paths.marks.iter().enumerate() {
            if mark.stages.contains(&stage) {
                let style = if paths.current == Some(k) {
                    bold(set_color(mark.set, true))
                } else {
                    fg(set_color(mark.set, true))
                };
                p.text(x_dots + 1 + 2 * k as u16, y, "●", style, area.right());
            }
        }
    }
    if rows.len() > offset + height {
        p.text(area.right().saturating_sub(3), area.bottom().saturating_sub(1), "↓", fg(FAINT), area.right());
    }
}

/// The lines drawn left of a stage: `│ ` under a stage with calls after it, `├ ` before one
/// with a call after it, `└ ` before the last.
fn guides(found: &Found, stage: usize) -> String {
    let chain = found.chain(stage);
    let last = |s: usize| found.stages[s].parent.is_none_or(|p| found.stages[p].children.last() == Some(&s));
    let mut out = String::new();
    for &above in chain.iter().skip(1).take(chain.len().saturating_sub(2)) {
        out.push_str(if last(above) { "  " } else { "│ " });
    }
    if chain.len() > 1 {
        out.push_str(if last(stage) { "└ " } else { "├ " });
    }
    out
}

fn root_name(root: Root) -> &'static str {
    match root {
        Root::Entry(Entry::Script) => "script",
        Root::Entry(Entry::Main) => "main",
        Root::Entry(Entry::Controller) => "controller",
        Root::Entry(Entry::Test) => "test",
        Root::Uncalled => "no caller: a framework, a container or an event calls it",
        Root::Cut => "cut short: its callers were not asked for",
    }
}

/// The stage under the cursor: where it is, who calls it, the marks it is in, and its lines.
fn detail(p: &mut Painter<'_>, app: &App, paths: &Paths, found: &Found, area: Rect) {
    let Some(tab) = app.tabs.get(paths.tab) else { return };
    let Some(at) = found.stages.get(paths.cursor) else { return };
    let workspace = tab.engine.workspace();
    let function = found.function(paths.cursor);
    let mut segs = vec![];
    if at.anchor {
        segs.push(seg("◆ ", bold(AMBER)));
    }
    segs.push(seg(function.name.clone(), bold(FG)));
    segs.push(seg(format!("  {}:{}", project_relative(&function.path, workspace), function.line), fg(FAINT)));
    if let Some(parent) = at.parent {
        let lines: Vec<String> = at.sites.iter().map(u32::to_string).collect();
        let how = if at.observed { "seen called at run time by" } else { "called by" };
        segs.push(seg(format!("  · {how} {} at {}", found.function(parent).name, lines.join(", ")), fg(FAINT)));
    }
    let marks = paths.marks_of(paths.cursor);
    if !marks.is_empty() {
        segs.push(seg("  · in ", fg(FAINT)));
        for (i, mark) in marks.iter().enumerate() {
            if i > 0 {
                segs.push(seg(", ", fg(FAINT)));
            }
            segs.push(seg(paths.name(mark.set), fg(set_color(mark.set, true))));
        }
    }
    p.segs(area.x + 2, area.y, &segs, area.right());
    let Some((stops, set)) = paths.lines_here() else { return };
    let said = match (set, at.cut) {
        (_, Some(Cut::Recursion)) => "it calls a function above it again: followed once".to_owned(),
        (_, Some(Cut::Deep)) => {
            "what it calls was not asked for: it is as far below the paths as PATHS looks".to_owned()
        }
        (_, Some(Cut::Spent)) => "what it calls was not all followed: the requests or the rows ran out".to_owned(),
        (Some(set), None) => format!("its lines in {}, with the calls chosen below it", paths.name(set)),
        (None, None) if found.on_path(paths.cursor) => "its lines on the end-to-end path through it".to_owned(),
        (None, None) => "its lines, marked with the path through it".to_owned(),
    };
    p.text(area.x + 4, area.y + 1, &said, italic(FAINT), area.right());
    let (x_line, x_why, x_code) = (area.x + 4, area.x + 10, area.x + 34);
    for (i, stop) in stops.iter().enumerate() {
        let y = area.y + 2 + i as u16;
        if y >= area.bottom() {
            p.text(area.x + 2, area.bottom().saturating_sub(1), "…", fg(FAINT), area.right());
            break;
        }
        let canonical = std::fs::canonicalize(&stop.path).unwrap_or_else(|_| stop.path.clone());
        let set = tab.engine.breakpoints().at(&canonical, stop.line).is_some();
        let (dot, style) = if set { ("●", bold(RED_DIM)) } else { ("○", fg(FAINT)) };
        p.text(area.x + 2, y, dot, style, area.right());
        p.text(x_line, y, &format!("{:>4}", stop.line), fg(FAINT), area.right());
        let (why, style) = reason(&stop.why);
        p.text(x_why, y, &why, style, x_code.saturating_sub(1));
        code(p, app, &stop.path, stop.line, Rect::new(x_code, y, area.right().saturating_sub(x_code), 1));
    }
}

fn reason(why: &Why) -> (String, Style) {
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

/// What the limits cut short and what no server sees, wrapped to `width`.
fn notes(found: &Found, width: usize) -> Vec<(String, Style)> {
    let mut lines: Vec<(String, Style)> = Vec::new();
    for note in &found.notes {
        lines.extend(wrap(note, width).into_iter().map(|line| (line, italic(FAINT))));
    }
    let seen = found.stages.iter().any(|stage| stage.observed);
    let runs = if seen {
        "⇢ marks the calls FLOW's runs saw"
    } else {
        "FLOW's runs add the calls their stacks show (⇢): run the program and ask again"
    };
    let unseen = format!("not seen without a run: {}; {runs}", found.unseen);
    lines.extend(wrap(&unseen, width).into_iter().map(|line| (line, italic(AMBER_DIM))));
    lines
}

fn footer(p: &mut Painter<'_>, inner: Rect, actions: &[(&str, &str)]) {
    let y = inner.bottom().saturating_sub(1);
    let mut x = inner.x + 1;
    for (key, label) in actions {
        x = p.text(x, y, key, bold(KEY), inner.right());
        x = p.text(x, y, &format!(" {label}   "), fg(FAINT), inner.right());
    }
}
