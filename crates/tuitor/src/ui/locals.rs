use ratatui::layout::Rect;
use tuitor_core::snapshot::Change;

use super::paint::{Painter, Panel, seg, truncate};
use super::theme::*;
use crate::app::App;
use crate::command::View;

pub fn draw(p: &mut Painter<'_>, app: &App, area: Rect, focus: bool) {
    let rows = app.local_rows();
    let snapshot = app.viewed();
    let changed = rows.iter().filter(|r| changed_here(r.change.as_ref())).count();
    let vars = rows.iter().filter(|r| !r.scope && !r.note && r.depth == 1).count();

    let title = if app.ui.view == View::History { "SNAPSHOT" } else { "LOCALS" };
    let sub = match snapshot {
        Some(s) if app.in_history() => vec![seg(format!("stop #{} · recorded", s.number), fg(DIM))],
        Some(s) => vec![seg(format!("stop #{}", s.number), fg(DIM))],
        None => vec![],
    };
    let right = if snapshot.is_some() {
        let mut r = vec![seg(format!("{vars} vars"), fg(DIM))];
        if changed > 0 {
            r.push(seg(format!(" · {changed} changed"), fg(DIM)));
        }
        r
    } else {
        vec![]
    };
    let keys = if focus { vec![("⏎", "expand"), ("w", "watch")] } else { vec![] };
    p.panel(area, &Panel { title, sub, right, keys, focus });

    let inner = super::paint::inner(area);
    if rows.is_empty() {
        let text = if app.active().is_some_and(|s| s.is_stopped()) {
            "reading…"
        } else {
            "variables appear when the program stops"
        };
        p.text(inner.x + 2, inner.y + 1, text, fg(FAINT), inner.right());
        return;
    }

    let name_w = (inner.width * 28 / 100).clamp(12, 34);
    let type_w = (inner.width * 22 / 100).clamp(8, 30);
    let x_name = inner.x + 2;
    let x_type = x_name + name_w;
    let x_value = x_type + type_w;
    let y0 = inner.y;
    p.text(x_name, y0, "NAME", bold(FAINT), inner.right());
    p.text(x_type, y0, "TYPE", bold(FAINT), inner.right());
    p.text(x_value, y0, "VALUE", bold(FAINT), inner.right());
    if let Some(prev) = app.previous() {
        p.segs_right(inner.right(), y0, &[seg(format!("Δ since #{}", prev.number), bold(FAINT))], x_value + 6);
    }

    let height = usize::from(inner.height.saturating_sub(1));
    let offset = app.ui.locals.window(height, rows.len());
    for (i, row) in rows.iter().enumerate().skip(offset).take(height) {
        let y = y0 + 1 + (i - offset) as u16;
        let selected = focus && i == app.ui.locals.pos;
        if selected {
            p.band(inner.x, y, inner.width, SEL);
        }
        let indent = row.depth.saturating_sub(1) as u16 * 2;
        if row.scope {
            let arrow = if row.expanded { "▾ " } else { "▸ " };
            let x = p.text(inner.x + 1, y, arrow, fg(FAINT), inner.right());
            p.text(x, y, &row.name, bold(if selected { FG } else { DIM }), inner.right());
            continue;
        }
        if row.note {
            p.text(x_name + indent + 2, y, &row.value, italic(GHOST), inner.right());
            continue;
        }
        match row.change {
            Some(Change::Changed { .. }) => {
                p.text(inner.x, y, "▲", bold(GREEN), inner.right());
            }
            Some(Change::New) => {
                p.text(inner.x, y, "+", bold(GREEN), inner.right());
            }
            // Only a hint: what changed is further down, bright once expanded.
            Some(Change::Inside) => {
                p.text(inner.x, y, "△", fg(GREEN_DIM), inner.right());
            }
            None => {}
        }
        let fresh = changed_here(row.change.as_ref());
        let nx = x_name + indent;
        if row.expandable {
            p.text(nx, y, if row.expanded { "▾" } else { "▸" }, fg(FAINT), inner.right());
        }
        let name_style = if fresh {
            fg(WHITE)
        } else if row.depth == 1 {
            fg(SYN_VAR)
        } else {
            fg(FG)
        };
        p.text(nx + 2, y, &row.name, name_style, x_type.saturating_sub(1));
        p.text(x_type, y, &row.type_name, fg(TYPE), x_value.saturating_sub(1));

        let (delta, delta_w) = match &row.change {
            Some(Change::Changed { was }) => {
                let was = truncate(&one_line(was), 24).into_owned();
                let segs = vec![seg("was ", fg(FAINT)), seg(was, fg(DIM))];
                let w = super::paint::width(&segs);
                (segs, w)
            }
            Some(Change::New) => (vec![seg("new", italic(FAINT))], 3),
            Some(Change::Inside) | None => (vec![], 0),
        };
        let value_end = if delta_w > 0 { inner.right().saturating_sub(delta_w + 2) } else { inner.right() };
        let value = one_line(&row.value);
        let value_style = if fresh { bold(GREEN) } else { value_style(&value) };
        p.text(x_value, y, &value, value_style, value_end.max(x_value + 4));
        if delta_w > 0 {
            p.segs_right(inner.right(), y, &delta, value_end);
        }
    }
}

/// The row's own value changed or appeared in this step (bright), as opposed to
/// something inside it (dim).
fn changed_here(change: Option<&Change>) -> bool {
    matches!(change, Some(Change::Changed { .. } | Change::New))
}

fn value_style(value: &str) -> ratatui::style::Style {
    match value {
        "null" | "None" | "undefined" | "nil" | "uninitialized" | "<optimized out>" => italic(FAINT),
        v if v.starts_with('"') || v.starts_with('\'') => fg(SYN_STRING),
        _ => fg(FG),
    }
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
