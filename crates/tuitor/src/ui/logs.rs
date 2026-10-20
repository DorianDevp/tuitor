use ratatui::layout::Rect;
use tuitor_core::log::OutputKind;

use super::paint::{Painter, Panel, seg, truncate};
use super::theme::*;
use crate::tab::Tab;

pub fn output(p: &mut Painter<'_>, tab: &Tab, area: Rect, focus: bool) {
    let lines = tab.engine.output();
    let right = if tab.ui.output_back > 0 {
        vec![seg(format!("{} lines up", tab.ui.output_back), fg(AMBER_DIM))]
    } else {
        vec![seg(format!("{} lines", lines.len()), fg(DIM))]
    };
    let keys = if focus { vec![("C", "clear")] } else { vec![] };
    p.panel(area, &Panel { title: "OUTPUT", sub: vec![seg("debuggee", fg(DIM))], right, keys, focus });

    let inner = super::paint::inner(area);
    let height = usize::from(inner.height);
    let end = lines.len().saturating_sub(tab.ui.output_back);
    let start = end.saturating_sub(height);
    for (row, line) in lines.iter().skip(start).take(end - start).enumerate() {
        let y = inner.y + row as u16;
        // The bottom line is the one `enter` and `copy` take.
        if focus && start + row + 1 == end {
            p.band(inner.x, y, inner.width, SEL);
        }
        let tag = tab.engine.session(line.session).map(|s| s.tag.clone()).unwrap_or_default();
        let mut x = p.text(inner.x + 1, y, &line.time.format("%H:%M:%S%.3f ").to_string(), fg(FAINT), inner.right());
        x = p.text(x, y, &format!("{tag:<5} "), fg(super::theme::tag(&tag)), inner.right());
        let style = match line.kind {
            OutputKind::Stdout => fg(FG),
            OutputKind::Stderr => fg(YELLOW_DIM),
            OutputKind::Console => fg(DIM),
            OutputKind::Adapter => fg(GHOST),
            OutputKind::Repl => bold(WHITE),
        };
        p.text(x, y, &line.text, style, inner.right());
    }
    if lines.is_empty() {
        p.text(inner.x + 2, inner.y, "program output and adapter messages appear here", fg(GHOST), inner.right());
    }
}

/// Columns for an event's name: `initialized`, `capabilities` and shorter fit, a longer one
/// (`progressUpdate`) is cut, and a space always keeps it apart from the detail.
const EVENT_NAME: usize = 12;

pub fn events(p: &mut Painter<'_>, tab: &Tab, area: Rect, focus: bool) {
    p.panel(
        area,
        &Panel {
            title: "EVENTS",
            sub: vec![seg("DAP", fg(DIM))],
            right: vec![seg("newest first", fg(FAINT))],
            keys: vec![],
            focus,
        },
    );
    let inner = super::paint::inner(area);
    let height = usize::from(inner.height);
    let events: Vec<_> = tab.engine.events().iter().rev().collect();
    let offset = tab.ui.events.window(height, events.len());
    for (row, event) in events.iter().enumerate().skip(offset).take(height) {
        let y = inner.y + (row - offset) as u16;
        if focus && row == tab.ui.events.pos {
            p.band(inner.x, y, inner.width, SEL);
        }
        let newest = row == 0;
        let tag = tab.engine.session(event.session).map(|s| s.tag.clone()).unwrap_or_default();
        let mut x = p.text(inner.x + 1, y, &event.time.format("%H:%M:%S%.3f ").to_string(), fg(FAINT), inner.right());
        x = p.text(x, y, &format!("{tag:<5} "), fg(super::theme::tag(&tag)), inner.right());
        let name_style = match (event.name.as_str(), newest) {
            // Where execution is: the stop, and its place.
            ("stopped" | "at", true) => bold(AMBER),
            ("failed", _) => bold(RED),
            (_, true) => bold(FG),
            _ => fg(DIM),
        };
        let name = truncate(&event.name, EVENT_NAME);
        x = p.text(x, y, &format!("{name:<EVENT_NAME$} "), name_style, inner.right());
        p.text(x, y, &event.detail, fg(if newest { FG } else { DIM }), inner.right());
    }
}
