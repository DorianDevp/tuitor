use ratatui::layout::Rect;

use super::paint::{Painter, Panel, seg};
use super::theme::*;
use crate::model::StackRow;
use crate::tab::Tab;

pub fn draw(p: &mut Painter<'_>, tab: &Tab, area: Rect, focus: bool) {
    let snapshot = tab.viewed();
    let rows = tab.stack_rows();
    let sub = snapshot.map(|s| vec![seg(format!("thread {}", s.thread_id), fg(DIM))]).unwrap_or_default();
    let right = snapshot.map(|s| vec![seg(format!("{} frames", s.frames.len()), fg(DIM))]).unwrap_or_default();
    let keys = if focus { vec![("v", "library"), ("⏎", "show")] } else { vec![] };
    p.panel(area, &Panel { title: "STACK", sub, right, keys, focus });

    let inner = super::paint::inner(area);
    let Some(snapshot) = snapshot else {
        p.text(inner.x + 2, inner.y, "no stop yet", fg(FAINT), inner.right());
        return;
    };
    let height = usize::from(inner.height);
    let offset = tab.ui.stack.window(height, rows.len());
    for (i, row) in rows.iter().enumerate().skip(offset).take(height) {
        let y = inner.y + (i - offset) as u16;
        let selected = i == tab.ui.stack.pos;
        if selected {
            p.band(inner.x, y, inner.width, if focus { SEL2 } else { SEL });
        }
        match row {
            StackRow::Frame(index) => {
                let frame = &snapshot.frames[*index];
                let marker = if *index == 0 {
                    "▶"
                } else if selected {
                    "›"
                } else {
                    " "
                };
                p.text(inner.x + 1, y, marker, bold(if *index == 0 { AMBER } else { WHITE }), inner.right());
                p.text(inner.x + 3, y, &format!("#{index:<3}"), fg(FAINT), inner.right());
                let location = frame.location();
                let loc_x = inner.right().saturating_sub(location.chars().count() as u16 + 1);
                let name_style = if frame.library {
                    fg(GHOST)
                } else if selected {
                    bold(WHITE)
                } else {
                    fg(FG)
                };
                p.text(inner.x + 8, y, &frame.name, name_style, loc_x.saturating_sub(1).max(inner.x + 12));
                p.text(loc_x, y, &location, fg(if frame.library { GHOST } else { DIM }), inner.right());
            }
            StackRow::Collapsed { first, count, label } => {
                p.text(inner.x + 3, y, &format!("#{first:<3}"), fg(FAINT), inner.right());
                let text = format!("┄ {count} library frames · {label} ┄");
                p.text(inner.x + 8, y, &text, italic(GHOST), inner.right());
            }
        }
    }
}
