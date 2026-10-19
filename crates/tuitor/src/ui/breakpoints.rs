use ratatui::layout::Rect;

use super::paint::{Painter, Panel, Seg, seg};
use super::theme::*;
use crate::app::App;
use crate::command::BpScope;
use crate::model::BpRow;

pub fn draw(p: &mut Painter<'_>, app: &App, area: Rect, focus: bool, wide: bool) {
    let rows = app.bp_rows();
    let scope = scope_toggle(app.ui.bp_scope);
    let keys =
        if focus { vec![("a", "scope"), ("e", "cond"), ("l", "log"), ("d", "del")] } else { vec![("a", "scope")] };
    p.panel(area, &Panel { title: "BREAKPOINTS", sub: scope, right: vec![], keys, focus });

    let inner = super::paint::inner(area);
    if rows.is_empty() {
        p.text(inner.x + 2, inner.y, "b on a SOURCE line adds one · f finds files", fg(FAINT), inner.right());
        return;
    }
    let hits = app.viewed().map(|s| s.hit_breakpoints.clone()).unwrap_or_default();
    let workspace = app.engine.workspace();
    let hits_x = inner.right().saturating_sub(5);
    let cond_x = if wide { inner.x + inner.width * 45 / 100 } else { inner.x + inner.width * 55 / 100 };

    let mut y = inner.y;
    if wide {
        p.text(inner.x + 4, y, "LOCATION", bold(FAINT), inner.right());
        p.text(cond_x, y, "CONDITION / LOG", bold(FAINT), inner.right());
        p.text(hits_x, y, "HITS", bold(FAINT), inner.right());
        y += 1;
    }
    // Unfocused and narrow: exception filters collapse into one summary line.
    let compact = !wide && !focus;
    let rows: Vec<BpRow> =
        if compact { rows.into_iter().filter(|r| matches!(r, BpRow::Breakpoint(_))).collect() } else { rows };
    let summary = if compact { exception_summary(app) } else { None };
    let footer = app.hidden_bp_count();
    let reserved = usize::from(footer > 0) + usize::from(summary.is_some());
    let height = usize::from(inner.bottom().saturating_sub(y)).saturating_sub(reserved);
    let offset = app.ui.bps.window(height, rows.len());
    for (i, row) in rows.iter().enumerate().skip(offset).take(height) {
        let selected = focus && i == app.ui.bps.pos;
        match row {
            BpRow::Breakpoint(id) => {
                let Some(bp) = app.engine.breakpoints().get(*id) else { continue };
                let here = hits.contains(id);
                if here {
                    p.band(inner.x, y, inner.width, AMBER_BG);
                } else if selected {
                    p.band(inner.x, y, inner.width, SEL2);
                }
                let (glyph, color) = if !bp.enabled {
                    ("○", GHOST)
                } else if bp.verified() == Some(false) {
                    ("◌", RED_DIM)
                } else if bp.is_logpoint() {
                    ("◆", YELLOW_DIM)
                } else if bp.condition.is_some() {
                    ("●", YELLOW_DIM)
                } else {
                    ("●", if here { RED } else { RED_DIM })
                };
                p.text(inner.x + 1, y, glyph, bold(color), inner.right());
                let path = if wide {
                    bp.path.strip_prefix(workspace).unwrap_or(&bp.path).display().to_string()
                } else {
                    file_name(&bp.path)
                };
                let location = format!("{path}:{}", bp.line);
                let loc_style = if !bp.enabled {
                    fg(GHOST)
                } else if here {
                    bold(WHITE)
                } else {
                    fg(FG)
                };
                p.text(inner.x + 3, y, &location, loc_style, cond_x.saturating_sub(1));
                let detail: Vec<Seg<'_>> = if here {
                    vec![seg("stopped here", italic(AMBER))]
                } else if let Some(log) = &bp.log_message {
                    vec![seg(format!("log \"{log}\""), italic(DIM))]
                } else if let Some(cond) = &bp.condition {
                    vec![seg(cond.clone(), italic(DIM))]
                } else if !bp.enabled {
                    vec![seg("disabled", italic(GHOST))]
                } else if bp.verified() == Some(false) {
                    vec![seg(bp.adapter_message().unwrap_or("not verified").to_owned(), italic(RED_DIM))]
                } else {
                    vec![]
                };
                p.segs(cond_x, y, &detail, hits_x.saturating_sub(1));
                let hits_text = bp.hits.to_string();
                p.segs_right(inner.right() - 1, y, &[seg(hits_text, fg(if bp.hits > 0 { FG } else { GHOST }))], hits_x);
            }
            BpRow::Exception { session, filter, label } => {
                if selected {
                    p.band(inner.x, y, inner.width, SEL2);
                }
                let enabled = app
                    .engine
                    .session(*session)
                    .and_then(|s| s.exception_filters.iter().find(|f| &f.filter == filter))
                    .is_some_and(|f| f.enabled);
                p.text(
                    inner.x + 1,
                    y,
                    if enabled { "⚑" } else { "⚐" },
                    bold(if enabled { RED_DIM } else { GHOST }),
                    inner.right(),
                );
                p.text(inner.x + 3, y, label, fg(if enabled { FG } else { GHOST }), cond_x.saturating_sub(1));
                let tag = app.engine.session(*session).map(|s| s.tag.clone()).unwrap_or_default();
                p.text(cond_x, y, &format!("exceptions · {tag}"), italic(FAINT), hits_x);
            }
        }
        y += 1;
    }
    if let Some(summary) = summary {
        p.segs(inner.x + 1, y, &summary, inner.right());
    }
    if footer > 0 {
        let text = format!("+ {footer} for other adapters   a → all");
        p.text(inner.x + 1, inner.bottom() - 1, &text, fg(FAINT), inner.right());
    }
}

fn exception_summary(app: &App) -> Option<Vec<Seg<'static>>> {
    let session = app.active()?;
    if session.exception_filters.is_empty() {
        return None;
    }
    let on: Vec<&str> = session.exception_filters.iter().filter(|f| f.enabled).map(|f| f.label.as_str()).collect();
    Some(if on.is_empty() {
        vec![
            seg("⚐ ", fg(GHOST)),
            seg(format!("exceptions off · {} filters", session.exception_filters.len()), italic(FAINT)),
        ]
    } else {
        vec![seg("⚑ ", bold(RED_DIM)), seg(format!("stops on {}", on.join(", ")), italic(DIM))]
    })
}

fn scope_toggle(scope: BpScope) -> Vec<Seg<'static>> {
    let (session, all) = match scope {
        BpScope::Session => (bold(WHITE), fg(FAINT)),
        BpScope::All => (fg(FAINT), bold(WHITE)),
    };
    vec![seg("session", session), seg(" · ", fg(GHOST)), seg("all", all)]
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}
