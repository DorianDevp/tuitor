use ratatui::layout::Rect;
use tuitor_core::config::home_relative;

use super::paint::{Painter, Panel, Seg, seg, truncate_left};
use super::theme::*;
use crate::app::App;
use crate::command::BpScope;
use crate::model::{BpRow, Hidden};
use crate::tab::Tab;

pub fn draw(p: &mut Painter<'_>, app: &App, area: Rect, focus: bool, wide: bool) {
    let tab = app.tab();
    let rows = app.bp_rows();
    let scope = scope_toggle(tab.ui.bp_scope);
    let keys =
        if focus { vec![("a", "scope"), ("e", "cond"), ("l", "log"), ("d", "del")] } else { vec![("a", "scope")] };
    p.panel(area, &Panel { title: "BREAKPOINTS", sub: scope, right: vec![], keys, focus });

    let inner = super::paint::inner(area);
    if rows.is_empty() {
        p.text(inner.x + 2, inner.y, "b on a SOURCE line adds one · f finds files", fg(FAINT), inner.right());
        return;
    }
    // Breakpoint ids are per tab: only this tab's viewed stop marks one as hit.
    let current = app.tabs.active_id();
    let hits = tab.viewed().map(|s| s.hit_breakpoints.clone()).unwrap_or_default();
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
        if compact { rows.into_iter().filter(|r| !matches!(r, BpRow::Exception { .. })).collect() } else { rows };
    let summary = if compact { exception_summary(tab) } else { None };
    let footer = hidden_note(&app.hidden_bps());
    let reserved = usize::from(footer.is_some()) + usize::from(summary.is_some());
    let height = usize::from(inner.bottom().saturating_sub(y)).saturating_sub(reserved);
    let offset = tab.ui.bps.window(height, rows.len());
    for (i, row) in rows.iter().enumerate().skip(offset).take(height) {
        let selected = focus && i == tab.ui.bps.pos;
        match row {
            BpRow::Tab(id) => {
                let Some(owner) = app.tabs.get(*id) else { continue };
                if selected {
                    p.band(inner.x, y, inner.width, SEL2);
                }
                let name = owner.name();
                let x = p.text(inner.x + 1, y, &name, bold(if *id == current { WHITE } else { DIM }), inner.right());
                let dir = owner.project.as_ref().map(|p| home_relative(&p.dir)).unwrap_or_default();
                let room = usize::from(inner.right().saturating_sub(x + 3));
                p.text(x + 2, y, &truncate_left(&dir, room), fg(GHOST), inner.right());
            }
            BpRow::Breakpoint { tab: owner, id } => {
                let Some(owner_tab) = app.tabs.get(*owner) else { continue };
                let Some(bp) = owner_tab.engine.breakpoints().get(*id) else { continue };
                let here = *owner == current && hits.contains(id);
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
                    bp.path.strip_prefix(owner_tab.engine.workspace()).unwrap_or(&bp.path).display().to_string()
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
            BpRow::Exception { tab: owner, session, filter, label } => {
                if selected {
                    p.band(inner.x, y, inner.width, SEL2);
                }
                let session = app.tabs.get(*owner).and_then(|t| t.engine.session(*session));
                let enabled = session
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
                let tag = session.map(|s| s.tag.clone()).unwrap_or_default();
                p.text(cond_x, y, &format!("exceptions · {tag}"), italic(FAINT), hits_x);
            }
        }
        y += 1;
    }
    if let Some(summary) = summary {
        p.segs(inner.x + 1, y, &summary, inner.right());
    }
    if let Some(footer) = footer {
        p.text(inner.x + 1, inner.bottom() - 1, &footer, fg(FAINT), inner.right());
    }
}

/// What the session scope leaves out, and the key that shows it.
fn hidden_note(hidden: &Hidden) -> Option<String> {
    let mut parts = Vec::new();
    if hidden.here > 0 {
        parts.push(format!("{} for other adapters", hidden.here));
    }
    if hidden.other_tabs > 0 {
        parts.push(format!("{} in other tabs", hidden.other_tabs));
    }
    (!parts.is_empty()).then(|| format!("+ {}   a → all", parts.join(" · ")))
}

fn exception_summary(tab: &Tab) -> Option<Vec<Seg<'static>>> {
    let session = tab.active()?;
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
