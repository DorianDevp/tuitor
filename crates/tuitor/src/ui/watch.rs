use ratatui::layout::Rect;
use tuitor_core::snapshot::Snapshot;

use super::paint::{Painter, Panel, seg, truncate};
use super::theme::*;
use crate::app::App;

const COL: u16 = 10;

pub fn draw(p: &mut Painter<'_>, app: &App, area: Rect, focus: bool) {
    let watches = app.engine.watches();
    let inner = super::paint::inner(area);
    let expr_w = (inner.width * 34 / 100).clamp(10, 28);
    let trend_w = 8;
    let max_cols = usize::from(inner.width.saturating_sub(expr_w + trend_w + 2) / COL).max(1);

    // The last stops up to the one being viewed.
    let history: Vec<&Snapshot> = match (app.active(), app.viewed()) {
        (Some(session), Some(viewed)) => {
            let end = viewed.number as usize;
            session.history[end.saturating_sub(max_cols)..end].iter().collect()
        }
        _ => Vec::new(),
    };
    let sub = vec![seg(format!("last {} stops", history.len()), fg(DIM))];
    let keys = if focus { vec![("w", "add"), ("d", "delete")] } else { vec![] };
    p.panel(area, &Panel { title: "WATCH", sub, right: vec![], keys, focus });

    if watches.is_empty() {
        p.text(inner.x + 2, inner.y, "w adds an expression; it is evaluated at every stop", fg(FAINT), inner.right());
        return;
    }
    let y0 = inner.y;
    p.text(inner.x + 1, y0, "EXPRESSION", bold(FAINT), inner.right());
    let latest = app.active().and_then(|s| s.latest()).map(|s| s.number);
    for (j, snap) in history.iter().enumerate() {
        let x = inner.x + 1 + expr_w + j as u16 * COL;
        let is_view = j + 1 == history.len();
        let live = Some(snap.number) == latest;
        let label = format!("#{}{}", snap.number, if live { "●" } else { "" });
        let color = if is_view { WHITE } else { FAINT };
        p.text(x + COL - 1 - label.chars().count() as u16, y0, &label, bold(color), inner.right());
    }
    let trend_x = inner.x + 1 + expr_w + history.len() as u16 * COL + 1;
    p.text(trend_x, y0, "TREND", bold(FAINT), inner.right());

    let height = usize::from(inner.height.saturating_sub(1));
    let offset = app.ui.watch.window(height, watches.len());
    for (i, expr) in watches.iter().enumerate().skip(offset).take(height) {
        let y = y0 + 1 + (i - offset) as u16;
        if focus && i == app.ui.watch.pos {
            p.band(inner.x, y, inner.width, SEL);
        }
        p.text(inner.x + 1, y, expr, fg(FG), inner.x + expr_w);
        let values: Vec<Option<String>> =
            history.iter().map(|s| s.watch(expr).map(|v| v.clone().unwrap_or_else(|_| "err".into()))).collect();
        for (j, value) in values.iter().enumerate() {
            let x = inner.x + 1 + expr_w + j as u16 * COL;
            let is_view = j + 1 == values.len();
            if is_view {
                p.band(x, y, COL, SEL);
            }
            let Some(value) = value else {
                p.text(x + COL - 3, y, "·", fg(GHOST), inner.right());
                continue;
            };
            let prev = j.checked_sub(1).and_then(|k| values[k].as_ref());
            let (arrow, color) = change(prev.map(String::as_str), value, is_view);
            let color = if value == "err" { RED_DIM } else { color };
            let text = truncate(&one_line(value), usize::from(COL - 2)).into_owned();
            let text_x = x + COL - 2 - text.chars().count() as u16;
            p.text(text_x, y, &text, if is_view && arrow != " " { bold(color) } else { fg(color) }, x + COL - 1);
            p.text(x + COL - 2, y, arrow, bold(color), x + COL);
        }
        let numbers: Option<Vec<f64>> = values.iter().map(|v| v.as_deref().and_then(number)).collect();
        match numbers {
            Some(n) if n.len() > 1 && n.iter().any(|v| *v != n[0]) => {
                p.text(trend_x, y, &spark(&n), fg(DIM), inner.right());
            }
            _ => {
                p.text(trend_x, y, &"·".repeat(values.len().min(6)), fg(GHOST), inner.right());
            }
        }
    }
}

fn change(prev: Option<&str>, value: &str, current: bool) -> (&'static str, ratatui::style::Color) {
    let Some(prev) = prev.filter(|p| *p != value) else {
        return (" ", if current { FG } else { DIM });
    };
    match (number(prev), number(value)) {
        (Some(a), Some(b)) if b > a => ("▲", if current { GREEN } else { GREEN_DIM }),
        (Some(_), Some(_)) => ("▼", if current { RED } else { RED_DIM }),
        _ => ("◆", if current { GREEN } else { YELLOW_DIM }),
    }
}

pub fn number(value: &str) -> Option<f64> {
    let cleaned: String = value.trim().trim_matches('"').chars().filter(|c| *c != '_').collect();
    cleaned.parse().ok()
}

pub fn spark(values: &[f64]) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let (lo, hi) = values.iter().fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    values
        .iter()
        .map(|v| if hi > lo { BARS[(((v - lo) / (hi - lo)) * 7.0).round() as usize] } else { BARS[3] })
        .collect()
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
