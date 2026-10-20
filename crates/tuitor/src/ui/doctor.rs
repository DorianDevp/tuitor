//! DOCTOR: what debugging needs on this machine, adapter by adapter and for the
//! configurations of the open projects, with what fixes each problem.

use std::time::Duration;

use ratatui::layout::Rect;
use tuitor_core::doctor::{Fix, Report, Status};

use super::paint::{Painter, Panel, Seg, seg, wrap};
use super::theme::*;
use crate::app::App;
use crate::doctor::{Doctor, fix_text, glyph};

/// Longest a subject column gets; longer subjects are cut.
const SUBJECT: usize = 24;

/// One line of the report, and the finding it belongs to.
struct Line {
    finding: Option<usize>,
    segs: Vec<Seg<'static>>,
}

pub fn draw(p: &mut Painter<'_>, app: &App, doctor: &Doctor, screen: Rect) {
    p.dim(screen);
    let width = screen.width.saturating_sub(8).min(124);
    let height = screen.height.saturating_sub(super::HEADER + 4).min(40);
    let rect = Rect::new((screen.width - width) / 2, super::HEADER + 1, width, height);
    p.fill(rect, BG);
    let right = match &doctor.report {
        Some(report) => summary(report),
        None => vec![seg("checking…", fg(FAINT))],
    };
    let sub = vec![seg("what debugging needs here", fg(DIM))];
    p.panel(rect, &Panel { title: "DOCTOR", sub, right, keys: vec![], focus: true });
    let inner = super::paint::inner(rect);
    let Some(report) = &doctor.report else {
        // The loop draws again soon, to show the report once it is there.
        app.ui.animate(Duration::from_millis(100));
        let text = "asking the adapters, gdb, Node.js and PHP…";
        p.text(inner.x + 2, inner.y + 1, text, italic(FAINT), inner.right());
        return;
    };

    let lines = lines(report, usize::from(inner.width.saturating_sub(2)));
    let height = usize::from(inner.height.saturating_sub(1));
    let last = lines.iter().rposition(|line| line.finding == Some(doctor.selected)).unwrap_or(0);
    let offset = (last + 2).saturating_sub(height);
    for (row, line) in lines.iter().enumerate().skip(offset).take(height) {
        let y = inner.y + (row - offset) as u16;
        if line.finding == Some(doctor.selected) {
            p.band(inner.x, y, inner.width, SEL2);
        }
        p.segs(inner.x + 1, y, &line.segs, inner.right().saturating_sub(1));
    }
    footer(p, doctor, inner);
}

/// How many problems stand in the way, or that none does.
fn summary(report: &Report) -> Vec<Seg<'static>> {
    match report.blocking() {
        0 if report.sections.iter().any(|s| s.needed) => vec![seg("nothing stands in the way", fg(DIM))],
        0 => vec![],
        1 => vec![seg("1 problem stands in the way", fg(RED))],
        n => vec![seg(format!("{n} problems stand in the way"), fg(RED))],
    }
}

/// The sections, each finding with its text wrapped beside the subject and its fix below.
fn lines(report: &Report, width: usize) -> Vec<Line> {
    let subject_width = report
        .sections
        .iter()
        .flat_map(|s| &s.findings)
        .map(|f| f.subject.chars().count())
        .max()
        .unwrap_or(0)
        .min(SUBJECT);
    let text_x = 2 + subject_width + 2;
    let room = width.saturating_sub(text_x).max(20);
    let indent = " ".repeat(text_x);
    let mut lines = Vec::new();
    let mut index = 0;
    for section in &report.sections {
        if !lines.is_empty() {
            lines.push(Line { finding: None, segs: Vec::new() });
        }
        lines.push(Line { finding: None, segs: vec![seg(section.title.clone(), bold(FAINT))] });
        let mut last: Option<&str> = None;
        for finding in &section.findings {
            let subject = if last == Some(finding.subject.as_str()) { "" } else { finding.subject.as_str() };
            last = Some(&finding.subject);
            let (mark, text) = match finding.status {
                Status::Ok => (fg(FAINT), fg(DIM)),
                Status::Note => (bold(YELLOW_DIM), fg(FG)),
                Status::Problem => (bold(RED), fg(FG)),
            };
            let subject = super::paint::truncate(subject, subject_width);
            for (i, part) in wrap_hard(&finding.text, room).into_iter().enumerate() {
                let segs = if i == 0 {
                    vec![
                        seg(format!("{} ", glyph(finding.status)), mark),
                        seg(format!("{subject:<subject_width$}  "), fg(FG)),
                        seg(part, text),
                    ]
                } else {
                    vec![seg(indent.clone(), fg(FAINT)), seg(part, text)]
                };
                lines.push(Line { finding: Some(index), segs });
            }
            if let Some(fix) = &finding.fix {
                for (i, part) in wrap_hard(&fix_text(fix, true), room.saturating_sub(2)).into_iter().enumerate() {
                    let arrow = if i == 0 { "→ " } else { "  " };
                    let segs = vec![seg(indent.clone(), fg(FAINT)), seg(arrow, fg(FAINT)), seg(part, bold(KEY))];
                    lines.push(Line { finding: Some(index), segs });
                }
            }
            index += 1;
        }
    }
    lines
}

/// [`wrap`], with words longer than a line cut into lines: a path is what a finding is about,
/// and it shows whole.
fn wrap_hard(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for line in wrap(text, width) {
        let chars: Vec<char> = line.chars().collect();
        lines.extend(chars.chunks(width.max(1)).map(|chunk| chunk.iter().collect::<String>()));
    }
    lines
}

/// What `enter` does with the selected finding's fix, and the other keys.
fn footer(p: &mut Painter<'_>, doctor: &Doctor, inner: Rect) {
    let enter = match doctor.selection().and_then(|(_, finding)| finding.fix.as_ref()) {
        Some(Fix::Command(_)) => Some("edit the command, ⏎ again runs it"),
        Some(Fix::Shell(_)) => Some("copy the command"),
        Some(Fix::Text(_)) | None => None,
    };
    let mut actions = Vec::new();
    if let Some(enter) = enter {
        actions.push(("⏎", enter));
    }
    actions.extend([("↑/↓", "select"), ("esc", "close")]);
    let y = inner.bottom().saturating_sub(1);
    let mut x = inner.x + 1;
    for (key, label) in actions {
        x = p.text(x, y, key, bold(KEY), inner.right());
        x = p.text(x, y, &format!(" {label}   "), fg(FAINT), inner.right());
    }
}
