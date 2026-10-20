//! COMPARE: the hits of two breakpoint sets' paths lined up, breakpoint by breakpoint and
//! hit by hit, with how the values each stop read differ; below it, the values of the hit
//! under the cursor.

use ratatui::layout::Rect;
use tuitor_core::compare::{Delta, Entry, differences, state};
use tuitor_core::session::Session;
use tuitor_core::snapshot::Snapshot;

use super::breakpoints::glyph;
use super::paint::{Painter, Panel, Seg, seg, truncate};
use super::theme::*;
use crate::app::App;
use crate::compare::{Compared, Path};

pub fn draw(p: &mut Painter<'_>, app: &App, area: Rect, focus: bool) {
    let tab = app.tab();
    let inner = super::paint::inner(area);
    let Some(compared) = tab.compared() else {
        p.panel(area, &Panel { title: "COMPARE", focus, ..Panel::default() });
        let lines = [
            "Nothing to compare yet. A set's path is the stops at its breakpoints in one run.",
            "N makes a second set; run the program, then g d compares the two, or /compare rose teal.",
            "With one set, run twice: its latest run is compared with the run before.",
        ];
        for (y, line) in (inner.y + 1..).zip(lines) {
            p.text(inner.x + 2, y, line, fg(FAINT), inner.right());
        }
        return;
    };
    let sessions = tab.engine.sessions();
    let side = |path: &Path<'_>| {
        vec![
            seg("◉ ", bold(set_color(path.set, true))),
            seg(format!("{} {}", set_name(path.set), path.run.label(sessions)), fg(FG)),
        ]
    };
    let mut sub = side(&compared.a);
    sub.push(seg("  against  ", fg(FAINT)));
    sub.extend(side(&compared.b));
    let keys = if focus { vec![("h/l", "column"), ("[ ]", "run"), ("⏎", "open")] } else { vec![] };
    let names = names(&compared, sessions);
    p.panel(area, &Panel { title: "COMPARE", sub, right: counts(&compared, &names), keys, focus });

    let x_loc = inner.x + 3;
    let loc_w = (inner.width / 4).clamp(22, 40);
    let x_a = x_loc + loc_w;
    let side_w = 16;
    let x_b = x_a + side_w;
    let x_state = x_b + side_w + 1;
    let col = tab.ui.compare_col;
    let header = |c: usize| if focus && col == c { bold(WHITE) } else { bold(FAINT) };
    p.text(x_loc, inner.y, "BREAKPOINT, HIT", header(0), x_a);
    p.segs(x_a, inner.y, &label(&compared.a, sessions, header(1)), x_b);
    p.segs(x_b, inner.y, &label(&compared.b, sessions, header(2)), x_state);
    p.text(x_state, inner.y, "VALUES", bold(FAINT), inner.right());

    if compared.entries.is_empty() {
        p.text(x_loc, inner.y + 1, "neither path hit a breakpoint", fg(FAINT), inner.right());
        return;
    }
    let height = usize::from(inner.height.saturating_sub(1));
    let cursor = tab.ui.compare.pos.min(compared.entries.len() - 1);
    let offset = tab.ui.compare.window(height, compared.entries.len());
    for (i, entry) in compared.entries.iter().enumerate().skip(offset).take(height) {
        let y = inner.y + 1 + (i - offset) as u16;
        let selected = i == cursor;
        if selected {
            p.band(inner.x, y, inner.width, if focus { SEL2 } else { SEL });
            if focus {
                let (x, w) = [(x_loc - 1, loc_w), (x_a - 1, side_w), (x_b - 1, side_w)][col.min(2)];
                p.band(x, y, w, CELL);
            }
        }
        let bp = tab.engine.breakpoints().get(entry.breakpoint);
        let (mark, color) = bp.map_or(("·", GHOST), |bp| glyph(bp, tab.engine.breakpoints().active_set(), false));
        p.text(inner.x + 1, y, mark, bold(color), x_loc);
        let location = bp.map_or_else(
            || format!("#{} (deleted)", entry.breakpoint),
            |bp| format!("{}:{}", file_name(&bp.path), bp.line),
        );
        let end = p.text(x_loc, y, &location, fg(if selected { WHITE } else { FG }), x_a - 1);
        p.text(end + 1, y, &format!("hit {}", entry.occurrence), fg(DIM), x_a - 1);
        p.segs(x_a, y, &stop_cell(entry.a, &compared.a), x_b - 1);
        p.segs(x_b, y, &stop_cell(entry.b, &compared.b), x_state - 1);
        p.segs(x_state, y, &summary(entry, &names), inner.right());
    }
}

/// What the sides are called: their sets, or their runs when they are of one set.
fn names(compared: &Compared<'_>, sessions: &[Session]) -> [String; 2] {
    if compared.a.set == compared.b.set {
        [compared.a.run.label(sessions), compared.b.run.label(sessions)]
    } else {
        [set_name(compared.a.set).to_owned(), set_name(compared.b.set).to_owned()]
    }
}

/// "4 on both · 2 rose alone · 3 teal alone".
fn counts(compared: &Compared<'_>, names: &[String; 2]) -> Vec<Seg<'static>> {
    let both = compared.entries.iter().filter(|e| e.both()).count();
    let alone = |a: bool| compared.entries.iter().filter(|e| !e.both() && e.a.is_some() == a).count();
    let text = format!("{both} on both · {} {} alone · {} {} alone", alone(true), names[0], alone(false), names[1]);
    vec![seg(text, fg(DIM))]
}

/// A side's column head: its set, by colour, and its run.
fn label(path: &Path<'_>, sessions: &[Session], style: ratatui::style::Style) -> Vec<Seg<'static>> {
    let name = format!("{} {}", set_name(path.set), path.run.label(sessions)).to_uppercase();
    vec![seg("● ", bold(set_color(path.set, true))), seg(name, style)]
}

/// The stop, and when after its run began.
fn stop_cell(stop: Option<&Snapshot>, path: &Path<'_>) -> Vec<Seg<'static>> {
    let Some(stop) = stop else { return vec![seg("—", fg(GHOST))] };
    let after = (stop.time - path.recording.start).num_milliseconds() as f64 / 1000.0;
    let glyph = if stop.silent { "↷" } else { "●" };
    vec![
        seg(format!("{glyph} "), fg(set_color(path.set, true))),
        seg(format!("#{:<4}", stop.number), fg(FG)),
        seg(format!("+{after:.2}s"), fg(FAINT)),
    ]
}

/// How the values of a hit both paths have differ, or what the one path that has it read.
fn summary(entry: &Entry<'_>, names: &[String; 2]) -> Vec<Seg<'static>> {
    match (entry.a, entry.b) {
        (Some(a), Some(b)) => {
            let deltas = differences(a, b);
            let count = |f: fn(&Delta) -> bool| deltas.iter().filter(|(_, d)| f(d)).count();
            let parts = [
                (count(|d| matches!(d, Delta::Differs(..))), "differ"),
                (count(|d| matches!(d, Delta::Appears(_))), "appear"),
                (count(|d| matches!(d, Delta::Disappears(_))), "disappear"),
            ];
            let said: Vec<String> =
                parts.iter().filter(|(n, _)| *n > 0).map(|(n, what)| format!("{n} {what}")).collect();
            if said.is_empty() {
                return vec![seg("same values", fg(FAINT))];
            }
            let first =
                deltas.iter().find(|(_, d)| d.differs()).map(|(name, _)| format!("  {name}…")).unwrap_or_default();
            vec![seg(said.join(" · "), fg(FG)), seg(first, fg(FAINT))]
        }
        (Some(stop), None) | (None, Some(stop)) => {
            let name = if entry.a.is_some() { &names[0] } else { &names[1] };
            let values: Vec<String> = state(stop)
                .into_iter()
                .filter(|(name, _)| !name.contains('.'))
                .take(3)
                .map(|(name, value)| format!("{name} {}", one_line(&value)))
                .collect();
            vec![seg(format!("{name} alone  "), fg(DIM)), seg(values.join(" · "), fg(FAINT))]
        }
        (None, None) => Vec::new(),
    }
}

/// VALUES: what the hit under the cursor read, on each path that has it.
pub fn detail(p: &mut Painter<'_>, app: &App, area: Rect) {
    let tab = app.tab();
    let Some(compared) = tab.compared() else {
        p.panel(area, &Panel { title: "VALUES", ..Panel::default() });
        return;
    };
    let Some(entry) = compared.entries.get(tab.ui.compare.pos) else {
        p.panel(area, &Panel { title: "VALUES", ..Panel::default() });
        return;
    };
    let function = entry.a.or(entry.b).and_then(|s| s.top()).map(|f| f.name.clone()).unwrap_or_default();
    let sub = vec![
        seg(format!("#{} hit {}", entry.breakpoint, entry.occurrence), bold(WHITE)),
        seg(format!(" · {function}"), fg(DIM)),
    ];
    p.panel(area, &Panel { title: "VALUES", sub, ..Panel::default() });
    let inner = super::paint::inner(area);
    let name_w = (inner.width * 30 / 100).clamp(16, 40);
    let value_w = (inner.width.saturating_sub(name_w + 4)) / 2;
    let (x_name, x_a) = (inner.x + 3, inner.x + 3 + name_w);
    let x_b = x_a + value_w;
    p.text(x_name, inner.y, "NAME", bold(FAINT), x_a);
    let sessions = tab.engine.sessions();
    p.segs(x_a, inner.y, &label(&compared.a, sessions, bold(FAINT)), x_b);
    p.segs(x_b, inner.y, &label(&compared.b, sessions, bold(FAINT)), inner.right());

    let rows: Vec<(String, Delta)> = match (entry.a, entry.b) {
        (Some(a), Some(b)) => {
            let (mut differ, same): (Vec<_>, Vec<_>) = differences(a, b).into_iter().partition(|(_, d)| d.differs());
            differ.extend(same);
            differ
        }
        (Some(a), None) => state(a).into_iter().map(|(name, value)| (name, Delta::Disappears(value))).collect(),
        (None, Some(b)) => state(b).into_iter().map(|(name, value)| (name, Delta::Appears(value))).collect(),
        (None, None) => Vec::new(),
    };
    let both = entry.both();
    for ((name, delta), y) in rows.iter().zip(inner.y + 1..inner.bottom()) {
        let (mark, a, b) = match delta {
            Delta::Same(value) => (" ", Some(value), Some(value)),
            Delta::Differs(a, b) => ("≠", Some(a), Some(b)),
            Delta::Appears(b) => ("+", None, Some(b)),
            Delta::Disappears(a) => ("−", Some(a), None),
        };
        let differs = both && delta.differs();
        let value = |v: Option<&String>| match v {
            Some(v) => seg(
                truncate(&one_line(v), usize::from(value_w.saturating_sub(2))).into_owned(),
                if differs { bold(WHITE) } else { fg(DIM) },
            ),
            None => seg(if both { "absent" } else { "" }, italic(GHOST)),
        };
        if both {
            p.text(inner.x + 1, y, mark, bold(if differs { WHITE } else { GHOST }), x_name);
        }
        p.text(x_name, y, name, fg(if differs { FG } else { FAINT }), x_a - 1);
        p.segs(x_a, y, &[value(a)], x_b - 1);
        p.segs(x_b, y, &[value(b)], inner.right());
    }
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}
