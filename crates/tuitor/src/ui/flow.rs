//! FLOW: the files of the run in the order execution reached them, each with a bar on
//! an axis of trace time or of stop numbers, where this stop is, and the breakpoints
//! on the way. Below it the selected file and the evidence for every row. In CODE the
//! same list is a compact sidebar.

use std::collections::BTreeSet;
use std::path::Path;

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use tuitor_core::breakpoints::Breakpoint;
use tuitor_core::config::home_relative;
use tuitor_core::flow::{Axis, Flow, Row, Status};

use super::breakpoints::glyph;
use super::paint::{Painter, Panel, Seg, seg, truncate_left};
use super::syntax::highlight;
use super::theme::*;
use crate::app::App;
use crate::command::View;
use crate::flow::{FlowView, Item, TraceNote, items, short_function};
use crate::outline::{self, Symbol};
use crate::tab::Tab;

/// Columns of the bars in the CODE sidebar.
const MINI_BAR: u16 = 10;

pub fn draw(p: &mut Painter<'_>, app: &App, area: Rect, focus: bool) {
    let tab = app.tab();
    let compact = tab.ui.view != View::Flow && !tab.ui.zoom;
    let view = tab.flow();
    let inner = super::paint::inner(area);
    let Some(view) = view else {
        p.panel(area, &Panel { title: "FLOW", focus, ..Panel::default() });
        let text = if compact { "files show from the first stop" } else { "files show here from the first stop on" };
        p.text(inner.x + 1, inner.y, text, fg(FAINT), inner.right());
        return;
    };
    let items = items(&view.flow, tab.ui.flow_vendor);
    let cursor = tab.flow_cursor(&items);
    if compact {
        sidebar(p, app, &view, &items, cursor, area, focus);
    } else {
        table(p, app, &view, &items, cursor, area, focus);
    }
}

/// How a row's status shows: its glyph, and the colour of its name.
fn status_style(status: Status) -> (&'static str, Style, Style) {
    match status {
        Status::Executing => ("▶", bold(AMBER), bold(AMBER)),
        Status::Stack => ("▷", fg(FG), fg(FG)),
        Status::Ran => ("✓", fg(FAINT), fg(DIM)),
        Status::Predicted => ("◇", fg(GHOST), fg(FAINT)),
    }
}

fn status_word(status: Status) -> &'static str {
    match status {
        Status::Executing => "executing",
        Status::Stack => "on the stack",
        Status::Ran => "ran",
        Status::Predicted => "predicted",
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

/// What a line of FLOW is called: the file, or the package.
fn item_name(flow: &Flow, item: &Item) -> String {
    match item {
        Item::File(row) => file_name(&flow.rows[*row].path),
        Item::Library { package, .. } => package.clone(),
    }
}

/// The directory of `path` under the workspace, for the dim column after its name.
fn directory(tab: &Tab, path: &Path) -> String {
    let parent = path.parent().unwrap_or(path);
    match parent.strip_prefix(tab.engine.workspace()) {
        Ok(rel) => rel.display().to_string(),
        Err(_) => home_relative(parent),
    }
}

/// Function names of an item, the most called first per file, with counts above one.
fn functions(flow: &Flow, item: &Item) -> String {
    let mut names: Vec<String> = Vec::new();
    for row in item.rows() {
        let row = &flow.rows[row];
        for (name, count) in &row.functions {
            let short = short_function(name, &row.path);
            let text = if *count > 1 { format!("{short} ×{count}") } else { short };
            if !names.contains(&text) {
                names.push(text);
            }
        }
    }
    names.join(", ")
}

/// The full FLOW pane of the FLOW view.
fn table(p: &mut Painter<'_>, app: &App, view: &FlowView, items: &[Item], cursor: usize, area: Rect, focus: bool) {
    let tab = app.tab();
    let flow = &view.flow;
    let sub = vec![seg(run_title(tab, view), fg(DIM))];
    let keys = if focus { vec![("⏎", "open in code"), ("b", "bp on entry"), ("v", "library")] } else { vec![] };
    p.panel(area, &Panel { title: "FLOW", sub, right: sources_summary(view), keys, focus });
    let inner = super::paint::inner(area);
    if inner.height < 5 || inner.width < 40 {
        return;
    }

    let file_w = (inner.width * 30 / 100).clamp(20, 46);
    let count_w = 6;
    let bars_w = (inner.width * 38 / 100).max(16);
    let fn_w = inner.width.saturating_sub(1 + 2 + file_w + count_w + 2 + bars_w + 1).max(8);
    let x_file = inner.x + 1;
    let x_fn = x_file + 2 + file_w + 1;
    let x_count = x_fn + fn_w;
    let x_bars = x_count + count_w + 2;
    let bars_w = inner.right().saturating_sub(x_bars + 1);

    let y = inner.y;
    p.text(x_file + 2, y, "FILE", bold(FAINT), x_fn);
    p.text(x_fn, y, "FUNCTIONS", bold(FAINT), x_count);
    let count_label = if matches!(flow.axis, Axis::Time(_)) { "CALLS" } else { "STOPS" };
    p.segs_right(x_count + count_w, y, &[seg(count_label, bold(FAINT))], x_count);
    axis(p, flow, view.base, x_bars, y, bars_w);

    let legend_h = 2;
    let height = usize::from(inner.height.saturating_sub(2 + legend_h));
    let offset = tab.ui.flow.window(height, items.len());
    for (i, item) in items.iter().enumerate().skip(offset).take(height) {
        let y = inner.y + 2 + (i - offset) as u16;
        if i == cursor {
            p.band(inner.x, y, inner.width, if focus { SEL2 } else { SEL });
        }
        let status = item.status(flow);
        let (glyph, glyph_style, name_style) = status_style(status);
        p.text(x_file, y, glyph, glyph_style, x_fn);
        let name_style = if matches!(item, Item::Library { .. }) { fg(GHOST) } else { name_style };
        let end = p.text(x_file + 2, y, &item_name(flow, item), name_style, x_fn - 1);
        let extra = match item {
            Item::File(row) => directory(tab, &flow.rows[*row].path),
            Item::Library { rows, .. } => plural(rows.len(), "file"),
        };
        let room = usize::from((x_fn - 1).saturating_sub(end + 2));
        p.text(end + 2, y, &truncate_left(&extra, room), fg(GHOST), x_fn - 1);
        let fn_style = if status == Status::Predicted { fg(FAINT) } else { fg(DIM) };
        p.text(x_fn, y, &functions(flow, item), fn_style, x_count - 1);
        let count: u32 = item.rows().iter().map(|&r| flow.rows[r].count).sum();
        p.segs_right(x_count + count_w, y, &[seg(count.to_string(), fg(FAINT))], x_count);
        bar(p, tab, flow, item, x_bars, y, bars_w);
    }

    let y = inner.bottom() - legend_h;
    legend(p, inner.x + 1, y, inner.right());
    p.segs(inner.x + 1, y + 1, &next_breakpoints(tab, view), inner.right());
}

/// "run #4 from orders.php:68", or how it was stopped when it has no number.
fn run_title(tab: &Tab, view: &FlowView) -> String {
    let first = tab.active().and_then(|s| s.history.get(view.base as usize));
    let from = first.and_then(|s| s.top()).map(|f| f.location()).unwrap_or_default();
    match view.run {
        Some(run) => format!("run #{run} from {from}"),
        None => format!("from {from}"),
    }
}

/// "observed 4 stops · trace 2026-10-20 01:12 · run #3": what the rows come from.
fn sources_summary(view: &FlowView) -> Vec<Seg<'static>> {
    let mut parts = vec![seg(format!("observed {}", plural(view.stops, "stop")), fg(DIM))];
    if let TraceNote::Found { modified, .. } = &view.trace {
        parts.push(seg(" · ", fg(GHOST)));
        parts.push(seg(format!("trace {}", local_time(*modified)), fg(DIM)));
    }
    if let Some(earlier) = &view.flow.earlier {
        parts.push(seg(" · ", fg(GHOST)));
        parts.push(seg(format!("run #{}", earlier.run), fg(DIM)));
    }
    parts
}

/// "1 stop", "3 stops".
fn plural(count: usize, noun: &str) -> String {
    if count == 1 { format!("1 {noun}") } else { format!("{count} {noun}s") }
}

fn local_time(time: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(time).format("%Y-%m-%d %H:%M").to_string()
}

/// The axis over the bars: labels, and a line with ticks and the stop marked on it. Stops
/// are numbered as in the session, `base` before the run's first.
fn axis(p: &mut Painter<'_>, flow: &Flow, base: u32, x: u16, y: u16, width: u16) {
    if width < 8 {
        return;
    }
    let ticks = (width / 14).clamp(1, 6);
    let line: String = "─".repeat(usize::from(width));
    p.text(x, y + 1, &line, fg(LINE), x + width);
    // Labels sit where the bars put what they name; one that would run into the last is left out.
    let mut free = x;
    for k in 0..=ticks {
        let fraction = f64::from(k) / f64::from(ticks);
        let (at, label) = match flow.axis {
            Axis::Time(end) => (end * fraction, format!("{}ms", ms(end * fraction))),
            Axis::Stops(end) => {
                let stop = 1.0 + (f64::from(end.max(1) - 1) * fraction).round();
                (stop, format!("#{}", stop + f64::from(base)))
            }
        };
        let col = column(flow.axis, at, width);
        let label_w = label.chars().count() as u16;
        let label_x = (x + col).saturating_sub(label_w / 2).clamp(x, (x + width).saturating_sub(label_w));
        if label_x >= free {
            p.text(label_x, y, &label, fg(FAINT), x + width);
            free = label_x + label_w + 1;
        }
        p.text(x + col, y + 1, "┬", fg(LINE), x + width);
    }
    let now = column(flow.axis, flow.now, width);
    p.text(x + now, y + 1, "▼", bold(AMBER), x + width);
}

fn ms(seconds: f64) -> String {
    let ms = seconds * 1000.0;
    match ms {
        ms if ms >= 10.0 => format!("{ms:.0}"),
        ms if ms >= 1.0 || ms == 0.0 => format!("{ms:.1}"),
        ms => format!("{ms:.2}"),
    }
}

/// The column of axis position `at` in `width` columns. Stop numbers take a column range each.
fn column(axis: Axis, at: f64, width: u16) -> u16 {
    let (lo, hi) = match axis {
        Axis::Time(end) => (0.0, end.max(1e-9)),
        Axis::Stops(end) => (0.5, f64::from(end.max(1)) + 0.5),
    };
    let fraction = ((at - lo) / (hi - lo)).clamp(0.0, 1.0);
    ((fraction * f64::from(width)).floor() as u16).min(width.saturating_sub(1))
}

/// An item's bar: solid where its activity is known to have happened, dotted where it is
/// predicted, with the breakpoints of its files where the evidence reaches their lines.
fn bar(p: &mut Painter<'_>, tab: &Tab, flow: &Flow, item: &Item, x: u16, y: u16, width: u16) {
    if width == 0 {
        return;
    }
    let hits = tab.viewed().map(|s| s.hit_breakpoints.as_slice()).unwrap_or_default();
    let active = tab.engine.breakpoints().active_set();
    let now = column(flow.axis, flow.now, width);
    let rows: Vec<&Row> = item.rows().iter().map(|&r| &flow.rows[r]).collect();
    let status = item.status(flow);
    let mut cells: Vec<Option<(&str, Color)>> = vec![None; usize::from(width)];
    for row in &rows {
        let Some((start, end)) = row.span else { continue };
        // This run's own stops are what happened; a trace is until the stop; a prediction never.
        let solid_until = match (flow.axis, row.sources.observed, row.status) {
            (Axis::Stops(_), true, _) => width,
            (_, _, Status::Predicted) => 0,
            _ => now + 1,
        };
        let color = match status {
            Status::Executing => AMBER_DIM,
            Status::Stack => FG,
            Status::Ran => DIM,
            Status::Predicted => GHOST,
        };
        for c in column(flow.axis, start, width)..=column(flow.axis, end, width) {
            let cell = &mut cells[usize::from(c)];
            if c < solid_until {
                *cell = Some(("─", color));
            } else if cell.is_none() {
                *cell = Some(("┄", GHOST));
            }
        }
        if let Some(ahead) = row.ahead {
            for c in column(flow.axis, end, width) + 1..=column(flow.axis, ahead, width) {
                cells[usize::from(c)].get_or_insert(("┄", GHOST));
            }
        }
    }
    for (c, cell) in cells.iter().enumerate() {
        let cx = x + c as u16;
        match cell {
            Some((glyph, color)) => {
                p.text(cx, y, glyph, fg(*color), x + width);
            }
            None if c as u16 == now => {
                p.text(cx, y, "│", fg(AMBER_DIM), x + width);
            }
            None => {}
        }
    }
    for row in &rows {
        for bp in tab.engine.breakpoints().in_file(&row.path).filter(|b| b.enabled) {
            for at in row.lines.get(&bp.line).into_iter().flatten() {
                let hit = hits.contains(&bp.id) && column(flow.axis, *at, width) == now;
                let (glyph, color) = glyph(bp, active, hit);
                p.text(x + column(flow.axis, *at, width), y, glyph, bold(color), x + width);
            }
        }
    }
}

fn legend(p: &mut Painter<'_>, x: u16, y: u16, right: u16) {
    let key = |glyph: &'static str, style: Style, text: &'static str| {
        [seg(glyph, style), seg(format!(" {text}   "), fg(FAINT))]
    };
    let segs: Vec<Seg<'_>> = [
        key("✓", fg(FAINT), "ran"),
        key("▶", bold(AMBER), "executing"),
        key("▷", fg(FG), "on the stack"),
        key("◇", fg(GHOST), "predicted"),
        key("─", fg(DIM), "ran"),
        key("┄", fg(GHOST), "predicted"),
        key("▼", bold(AMBER), "this stop"),
        key("●", bold(DIM), "breakpoint, in its set's colour"),
        key("◆", bold(DIM), "logpoint"),
    ]
    .into_iter()
    .flatten()
    .collect();
    p.segs(x, y, &segs, right);
}

/// The breakpoints the evidence says execution reaches after this stop, nearest first.
fn next_breakpoints(tab: &Tab, view: &FlowView) -> Vec<Seg<'static>> {
    let flow = &view.flow;
    let base = f64::from(view.base);
    let mut ahead: Vec<(f64, &Row, &Breakpoint)> = Vec::new();
    for row in &flow.rows {
        for bp in tab.engine.breakpoints().in_file(&row.path).filter(|b| b.enabled) {
            if let Some(at) = row.lines.get(&bp.line).into_iter().flatten().find(|at| **at > flow.now) {
                ahead.push((*at, row, bp));
            }
        }
    }
    ahead.sort_by(|a, b| a.0.total_cmp(&b.0));
    if ahead.is_empty() {
        return vec![seg("no breakpoint ahead on this path that the evidence reaches", fg(GHOST))];
    }
    let mut segs = vec![seg("next breakpoints on this path: ", fg(FAINT))];
    for (i, (at, row, bp)) in ahead.iter().take(3).enumerate() {
        if i > 0 {
            segs.push(seg(" · ", fg(GHOST)));
        }
        let (glyph, color) = glyph(bp, tab.engine.breakpoints().active_set(), false);
        segs.push(seg(glyph, bold(color)));
        segs.push(seg(format!(" {}:{}", file_name(&row.path), bp.line), fg(FG)));
        let when = match flow.axis {
            Axis::Time(_) => format!(" in {} ms of trace", ms(at - flow.now)),
            Axis::Stops(_) => match row.sources.run {
                Some(run) => format!(" at stop #{} as run #{run} went", at + base),
                None => format!(" at stop #{}", at + base),
            },
        };
        segs.push(seg(when, fg(DIM)));
    }
    segs
}

/// The compact list of the CODE sidebar: glyph, name and a small bar per line.
fn sidebar(p: &mut Painter<'_>, app: &App, view: &FlowView, items: &[Item], cursor: usize, area: Rect, focus: bool) {
    let tab = app.tab();
    let flow = &view.flow;
    let sub = view.run.map(|run| vec![seg(format!("run #{run}"), fg(DIM))]).unwrap_or_default();
    let keys = if focus { vec![("⏎", "open")] } else { vec![] };
    p.panel(area, &Panel { title: "FLOW", sub, right: vec![], keys, focus });
    let inner = super::paint::inner(area);
    let bar_w = MINI_BAR.min(inner.width / 3);
    let x_bar = inner.right().saturating_sub(bar_w + 1);
    let legend = inner.height >= 8;
    let height = usize::from(inner.height.saturating_sub(if legend { 2 } else { 0 }));
    let offset = tab.ui.flow.window(height, items.len());
    let shown = tab.source_path();
    for (i, item) in items.iter().enumerate().skip(offset).take(height) {
        let y = inner.y + (i - offset) as u16;
        let here = matches!(item, Item::File(row) if Some(&flow.rows[*row].path) == shown.as_ref());
        if focus && i == cursor {
            p.band(inner.x, y, inner.width, SEL2);
        } else if here {
            p.band(inner.x, y, inner.width, SEL);
        }
        let status = item.status(flow);
        let (glyph, glyph_style, name_style) = status_style(status);
        p.text(inner.x + 1, y, glyph, glyph_style, x_bar);
        // SOURCE names the file in full; here its stem leaves room for the bar.
        let name = match item {
            Item::File(row) => {
                let path = &flow.rows[*row].path;
                path.file_stem().map_or_else(|| file_name(path), |s| s.to_string_lossy().into_owned())
            }
            Item::Library { package, .. } => format!("{package} ⋯"),
        };
        let style = if matches!(item, Item::Library { .. }) { fg(GHOST) } else { name_style };
        p.text(inner.x + 3, y, &name, style, x_bar.saturating_sub(1));
        bar(p, tab, flow, item, x_bar, y, bar_w);
    }
    if legend {
        let y = inner.bottom() - 1;
        let segs = [
            seg("✓", fg(FAINT)),
            seg(" ran  ", fg(FAINT)),
            seg("▶", bold(AMBER)),
            seg(" now  ", fg(FAINT)),
            seg("◇", fg(GHOST)),
            seg(" predicted", fg(FAINT)),
        ];
        p.segs(inner.x + 1, y, &segs, inner.right());
    }
}

/// SELECTED: the file under the FLOW cursor, what the evidence says about it, and its code.
pub fn detail(p: &mut Painter<'_>, app: &App, area: Rect) {
    let tab = app.tab();
    let Some(view) = tab.flow() else {
        p.panel(area, &Panel { title: "SELECTED", ..Panel::default() });
        return;
    };
    let flow = &view.flow;
    let items = items(flow, tab.ui.flow_vendor);
    let Some(item) = items.get(tab.flow_cursor(&items)) else {
        p.panel(area, &Panel { title: "SELECTED", ..Panel::default() });
        return;
    };
    let status = item.status(flow);
    let sub = vec![seg(item_name(flow, item), bold(WHITE)), seg(format!(" · {}", status_word(status)), fg(DIM))];
    p.panel(area, &Panel { title: "SELECTED", sub, ..Panel::default() });
    let inner = super::paint::inner(area);
    let row = match item {
        Item::File(row) => &flow.rows[*row],
        Item::Library { rows, .. } => {
            for (y, &row) in (inner.y..inner.bottom()).zip(rows) {
                let row = &flow.rows[row];
                let (glyph, style, name) = status_style(row.status);
                let x = p.text(inner.x + 1, y, glyph, style, inner.right());
                let x = p.text(x + 1, y, &file_name(&row.path), name, inner.right());
                p.text(x + 2, y, &directory(tab, &row.path), fg(GHOST), inner.right());
            }
            return;
        }
    };
    let source = app.sources.borrow_mut().get(&row.path);
    let symbols: &[Symbol] = source.as_deref().map(|s| s.outline()).unwrap_or_default();
    let label_w = 13;
    let mut lines: Vec<(&str, Vec<Seg<'static>>)> = evidence_of(&view, row)
        .into_iter()
        .enumerate()
        .map(|(i, text)| (if i == 0 { "evidence" } else { "" }, vec![seg(text, fg(FG))]))
        .collect();
    lines.extend([
        ("called from", called_from(tab, flow, row)),
        ("functions", functions_with_lines(row, symbols)),
        ("calls out", calls_out(flow, row)),
        ("lines", vec![seg(ranges(&row.lines.keys().copied().collect::<Vec<_>>()), fg(FG))]),
    ]);
    let bps = breakpoints_of(tab, &view, row);
    if !bps.is_empty() {
        lines.push(("breakpoints", bps));
    }
    let mut y = inner.y;
    for (label, value) in &lines {
        if y >= inner.bottom() {
            return;
        }
        p.text(inner.x + 1, y, label, fg(FAINT), inner.x + 1 + label_w);
        p.segs(inner.x + 1 + label_w, y, value, inner.right().saturating_sub(1));
        y += 1;
    }
    // The code where the file is entered, as far as there is room.
    let Some(source) = source else { return };
    if inner.bottom() <= y + 2 {
        return;
    }
    y += 1;
    let separator = "─".repeat(usize::from(inner.width));
    p.text(inner.x, y, &separator, fg(LINE), inner.right());
    y += 1;
    let focus_line = row.stack_line.unwrap_or_else(|| app.flow_entry(row));
    let height = u32::from(inner.bottom() - y);
    let first = focus_line.saturating_sub(height / 3).max(1);
    let lang = source.lang();
    let breakpoints = tab.engine.breakpoints();
    for n in first..first + height {
        let Some(text) = source.lines.get(usize::try_from(n - 1).unwrap_or(usize::MAX)) else { break };
        let x = inner.x;
        if let Some(bp) = breakpoints.at(&row.path, n) {
            let (glyph, color) = glyph(bp, breakpoints.active_set(), false);
            p.text(x, y, glyph, bold(color), inner.right());
        } else if row.lines.contains_key(&n) {
            p.text(x, y, "│", fg(GHOST), inner.right());
        }
        let number_style = if n == focus_line { fg(FG) } else { fg(FAINT) };
        p.text(x + 1, y, &format!("{n:>5} "), number_style, inner.right());
        let mut cx = x + 8;
        for (piece, style) in highlight(text, lang) {
            cx = p.text(cx, y, &piece, style, inner.right());
        }
        y += 1;
    }
}

/// Why the row is there: what each source that names it says, a line each.
fn evidence_of(view: &FlowView, row: &Row) -> Vec<String> {
    let flow = &view.flow;
    let base = f64::from(view.base);
    let mut parts: Vec<String> = Vec::new();
    if row.sources.observed {
        parts.push(match (flow.axis, row.span) {
            (Axis::Stops(_), Some((first, last))) if first == last => format!("on the stack at stop #{}", first + base),
            (Axis::Stops(_), Some((first, last))) => {
                format!("on the stack at stops #{}–#{}", first + base, last + base)
            }
            _ => format!("on the stack at {} of this run", plural(row.stops as usize, "stop")),
        });
    }
    if row.sources.trace
        && let Some((first, last)) = row.span
    {
        let when = if first >= flow.now { "after this stop" } else { "before it" };
        let calls = plural(row.count as usize, "call");
        parts.push(format!("trace: {calls} from {} to {} ms, first {when}", ms(first), ms(last)));
    }
    if let (Some(run), Some(ahead), true) = (row.sources.run, row.ahead, row.sources.observed) {
        parts.push(format!("run #{run} went on in it, as far as stop #{} here", ahead + base));
    } else if let (Some(run), Some(earlier)) = (row.sources.run, &flow.earlier) {
        let after = match &earlier.matched {
            Some((location, stop)) => {
                format!("after its stop #{stop} at {}:{}, where this run is", file_name(&location.path), location.line)
            }
            None => "after where it first differs".to_owned(),
        };
        parts.push(format!("run #{run} reached it {after}"));
    }
    parts
}

fn called_from(tab: &Tab, flow: &Flow, row: &Row) -> Vec<Seg<'static>> {
    let mut seen = BTreeSet::new();
    let mut segs = Vec::new();
    for edge in flow.edges.iter().filter(|e| e.callee_path == row.path && e.site.path != row.path) {
        let text = format!(
            "{} {}:{}",
            short_function(&edge.caller, &edge.site.path),
            file_name(&edge.site.path),
            edge.site.line
        );
        if !seen.insert(text.clone()) {
            continue;
        }
        if !segs.is_empty() {
            segs.push(seg(" · ", fg(GHOST)));
        }
        let status = flow.row(&edge.site.path).map_or(Status::Ran, |r| r.status);
        let (glyph, style, _) = status_style(status);
        segs.push(seg(format!("{glyph} "), style));
        segs.push(seg(text, fg(FG)));
    }
    if segs.is_empty() {
        let note = if row.stack_line.is_some()
            && tab.viewed().and_then(|s| s.frames.last()).and_then(|f| f.path.as_ref()) == Some(&row.path)
        {
            "it is where the stack starts"
        } else {
            "no call into it from another file is recorded"
        };
        segs.push(seg(note, fg(GHOST)));
    }
    segs
}

fn calls_out(flow: &Flow, row: &Row) -> Vec<Seg<'static>> {
    let mut seen = BTreeSet::new();
    let mut segs = Vec::new();
    for edge in flow.edges.iter().filter(|e| e.site.path == row.path && e.callee_path != row.path) {
        let text = short_function(&edge.callee, &edge.callee_path);
        let text = if text.contains("->") || text.contains("::") {
            text
        } else {
            format!("{}::{text}", edge.callee_path.file_stem().and_then(|s| s.to_str()).unwrap_or_default())
        };
        if !seen.insert(text.clone()) {
            continue;
        }
        if !segs.is_empty() {
            segs.push(seg(" · ", fg(GHOST)));
        }
        segs.push(seg(text, fg(FG)));
    }
    if segs.is_empty() {
        segs.push(seg("none recorded", fg(GHOST)));
    }
    segs
}

fn functions_with_lines(row: &Row, symbols: &[Symbol]) -> Vec<Seg<'static>> {
    let mut segs = Vec::new();
    for (name, count) in &row.functions {
        if !segs.is_empty() {
            segs.push(seg(" · ", fg(GHOST)));
        }
        segs.push(seg(short_function(name, &row.path), fg(FG)));
        if let Some(symbol) = outline::find(symbols, name).filter(|_| crate::flow::is_named_function(name)) {
            segs.push(seg(format!(" :{}", symbol.line), fg(DIM)));
        }
        if *count > 1 {
            segs.push(seg(format!(" ×{count}"), fg(FAINT)));
        }
    }
    segs
}

fn breakpoints_of(tab: &Tab, view: &FlowView, row: &Row) -> Vec<Seg<'static>> {
    let flow = &view.flow;
    let mut segs = Vec::new();
    for bp in tab.engine.breakpoints().in_file(&row.path) {
        if !segs.is_empty() {
            segs.push(seg(" · ", fg(GHOST)));
        }
        let (glyph, color) = glyph(bp, tab.engine.breakpoints().active_set(), false);
        segs.push(seg(glyph, bold(color)));
        segs.push(seg(format!(" :{}", bp.line), fg(FG)));
        let reached: Vec<f64> = row.lines.get(&bp.line).cloned().unwrap_or_default();
        let note = match (reached.iter().find(|at| **at > flow.now), reached.is_empty(), flow.axis) {
            (Some(at), _, Axis::Time(_)) => format!(" ahead, in {} ms", ms(at - flow.now)),
            (Some(at), _, Axis::Stops(_)) => format!(" ahead, stop #{}", at + f64::from(view.base)),
            (None, false, _) => " reached".to_owned(),
            (None, true, _) => " not reached on this evidence".to_owned(),
        };
        segs.push(seg(note, fg(DIM)));
    }
    segs
}

/// `[3, 4, 5, 9]` → `3–5, 9`.
fn ranges(lines: &[u32]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let start = lines[i];
        let mut end = start;
        while i + 1 < lines.len() && lines[i + 1] == end + 1 {
            i += 1;
            end = lines[i];
        }
        out.push(if end > start { format!("{start}–{end}") } else { start.to_string() });
        i += 1;
    }
    if out.is_empty() { "none recorded".to_owned() } else { out.join(", ") }
}

/// EVIDENCE: what each source of the flow is, and what it says.
pub fn evidence(p: &mut Painter<'_>, app: &App, area: Rect) {
    let tab = app.tab();
    let sub = vec![seg("why these files", fg(DIM))];
    p.panel(area, &Panel { title: "EVIDENCE", sub, ..Panel::default() });
    let inner = super::paint::inner(area);
    let Some(view) = tab.flow() else { return };
    let flow = &view.flow;
    let label_w = 13;
    let width = usize::from(inner.width.saturating_sub(label_w + 2));

    let run = view.run.map(|r| format!(" (run #{r})")).unwrap_or_default();
    let files = plural(flow.rows.iter().filter(|r| r.sources.observed).count(), "file");
    let earlier = flow.earlier.as_ref().map_or_else(|| "earlier run".to_owned(), |e| format!("run #{}", e.run));
    let sections = [
        (
            "observed".to_owned(),
            vec![vec![seg(
                format!("{} of this run{run} · {files} on their stacks", plural(view.stops, "stop")),
                fg(FG),
            )]],
        ),
        ("trace".to_owned(), trace_lines(tab, &view.trace, width)),
        (earlier, earlier_lines(tab, &view)),
    ];
    let mut y = inner.y;
    for (label, lines) in sections {
        for (i, segs) in lines.iter().enumerate() {
            if y + 1 >= inner.bottom() {
                break;
            }
            if i == 0 {
                p.text(inner.x + 1, y, &label, fg(FAINT), inner.x + 1 + label_w);
            }
            p.segs(inner.x + 1 + label_w, y, segs, inner.right().saturating_sub(1));
            y += 1;
        }
        y += 1;
    }
    let note = "predicted: reached later by the trace or an earlier run";
    p.text(inner.x + 1, inner.bottom() - 1, note, fg(GHOST), inner.right());
}

fn trace_lines(tab: &Tab, note: &TraceNote, width: usize) -> Vec<Vec<Seg<'static>>> {
    let text = |t: String, c: Color| vec![seg(t, fg(c))];
    let dir = |dir: &Path| {
        let rel = dir.strip_prefix(tab.engine.workspace()).map(|r| r.display().to_string());
        rel.unwrap_or_else(|_| home_relative(dir))
    };
    match note {
        TraceNote::Found { file, started, calls, end, now, candidates, pass, modified, pauses, paused } => {
            let name = truncate_left(&file_name(file), width).into_owned();
            let mut lines = vec![text(name, FG)];
            lines.push(text(
                format!("written {} · {} · {} ms", local_time(*modified), plural(*calls, "call"), ms(*end)),
                DIM,
            ));
            let times = if *candidates > 1 { format!(", pass {} of {candidates}", pass + 1) } else { String::new() };
            lines.push(text(format!("has this stop's stack{times}, at {} ms", ms(*now)), DIM));
            if *pauses > 0 {
                let what = format!("{} left out of its times", plural(*pauses, "debugger stop"));
                lines.push(text(format!("{what}, {:.1} s in all", paused), FAINT));
            }
            if let Some(started) = started {
                lines.push(text(format!("TRACE START {started}, as PHP wrote it"), FAINT));
            }
            lines
        }
        TraceNote::Off => vec![
            text("none: this configuration names no xdebugTraceDir".into(), DIM),
            text("set it in CONFIG to where Xdebug writes traces (README: FLOW)".into(), FAINT),
        ],
        TraceNote::NotPhp => vec![text("none: function traces come from Xdebug, for PHP".into(), DIM)],
        TraceNote::Unreadable { dir: d, error } => {
            vec![text(format!("cannot read {}: {error}", dir(d)), RED_DIM)]
        }
        TraceNote::NoMatch { dir: d, checked, running } => {
            let first = match checked {
                0 => format!("no finished trace in {} yet", dir(d)),
                1 => format!("the finished trace in {} took another path", dir(d)),
                n => format!("none of the {n} finished traces in {} took this path", dir(d)),
            };
            let mut lines = vec![text(first, DIM)];
            if *running > 0 {
                lines.push(text(format!("{running} still being written (a request still running)"), FAINT));
            }
            lines
        }
    }
}

/// The earlier run predictions come from, and how it lines up with this one.
fn earlier_lines(tab: &Tab, view: &FlowView) -> Vec<Vec<Seg<'static>>> {
    let Some(earlier) = &view.flow.earlier else {
        let first = tab.active().and_then(|s| s.history.get(view.base as usize));
        let first = first.and_then(|s| s.top()).map(|f| f.location());
        let from = first.map(|l| format!(" from {l}")).unwrap_or_default();
        return vec![vec![seg(format!("none recorded{from} before this run"), fg(DIM))]];
    };
    let mut lines = vec![vec![seg(format!("{} · started where this run did", earlier.started), fg(FG))]];
    lines.push(vec![seg(
        match &earlier.matched {
            Some((_, stop)) => format!("its stop #{stop} was where this one is"),
            None => "none of its stops was where this one is".to_owned(),
        },
        fg(DIM),
    )]);
    let predicted = match earlier.predicted {
        0 => "it reached no file after that which this run has not".to_owned(),
        1 => "it reached 1 more file after that".to_owned(),
        n => format!("it reached {n} more files after that"),
    };
    lines.push(vec![seg(predicted, fg(DIM))]);
    lines
}
