//! REBASE: the rebases in git's reflog, and under one opened, each pick that stopped with
//! what was decided by hand in it, then what the branch lost; below, the row under the cursor:
//! a conflict's two sides and its resolution, or the lines lost.

use chrono::{DateTime, Local};
use ratatui::layout::Rect;
use tuitor_core::rebase::{Decision, Kept, Lost, Rebase};

use super::paint::{Painter, Panel, Seg, seg, truncate};
use super::theme::*;
use crate::app::App;
use crate::rebase::{RebaseView, Row};

pub fn draw(p: &mut Painter<'_>, app: &App, area: Rect, focus: bool) {
    let inner = super::paint::inner(area);
    let Some(view) = app.tab().rebase.as_ref() else {
        p.panel(area, &Panel { title: "REBASE", focus, ..Panel::default() });
        p.text(inner.x + 2, inner.y + 1, "/rebase reads the rebases from git's reflog", fg(FAINT), inner.right());
        return;
    };
    let keys = if focus { vec![("⏎", "open"), ("h/l", "fold"), ("b", "set of decisions")] } else { vec![] };
    let repo = view.repo.display().to_string();
    let sub = vec![seg(repo, fg(FAINT))];
    let right = vec![seg(format!("{} rebases", view.rebases.len()), fg(FAINT))];
    p.panel(area, &Panel { title: "REBASE", sub, right, keys, focus });

    let rows = view.rows();
    let height = usize::from(inner.height);
    let cursor = view.cursor.pos.min(rows.len().saturating_sub(1));
    let offset = view.cursor.window(height, rows.len());
    for (i, row) in rows.iter().enumerate().skip(offset).take(height) {
        let y = inner.y + (i - offset) as u16;
        if i == cursor {
            p.band(inner.x, y, inner.width, if focus { SEL2 } else { SEL });
        }
        let segs = row_segs(view, *row);
        p.segs(inner.x + 1, y, &segs, inner.right());
    }
}

fn row_segs(view: &RebaseView, row: Row) -> Vec<Seg<'static>> {
    match row {
        Row::Rebase(i) => {
            let rebase = &view.rebases[i];
            let fold = if view.open.contains(&i) { "▾ " } else { "▸ " };
            let mut segs = vec![
                seg(fold, fg(DIM)),
                seg(format!("#{:<3}", i + 1), fg(FAINT)),
                seg(format!("{}  ", when(rebase.ended)), fg(DIM)),
                seg(rebase.title(), if rebase.aborted { fg(FAINT) } else { bold(WHITE) }),
                seg(
                    format!("   {} picks · {} stopped · {}", rebase.picks.len(), rebase.stops().count(), took(rebase)),
                    fg(FAINT),
                ),
            ];
            if rebase.aborted {
                segs.push(seg("  aborted", italic(FAINT)));
            }
            if let Some(analysis) = view.analyses.get(&i) {
                let lost = code_lost(&analysis.lost);
                if !analysis.upstream.is_empty() {
                    segs.push(seg(format!(" · {} picked again", analysis.upstream.len()), fg(YELLOW_DIM)));
                }
                segs.push(seg(format!(" · {} by hand", analysis.decisions()), fg(AMBER_DIM)));
                if lost > 0 {
                    segs.push(seg(format!(" · {lost} lines of code lost"), fg(YELLOW_DIM)));
                }
            }
            segs
        }
        Row::Reading(i) => {
            let progress = view.progress().filter(|(at, ..)| *at == i);
            let text = match progress {
                Some((_, done, total)) => format!("reading the picks that stopped: {done} of {total}…"),
                None => "reading…".into(),
            };
            vec![seg("    ", fg(FAINT)), seg(text, italic(FAINT))]
        }
        Row::Stop(i, s) => {
            let stop = &view.analyses[&i].stops[s];
            let count = match (&stop.original, stop.decisions.len()) {
                (None, _) => seg("  its original is not in the branch before", italic(FAINT)),
                (Some(_), 0) => seg("  nothing by hand", italic(FAINT)),
                (Some(_), n) => seg(format!("  {n} by hand"), fg(AMBER_DIM)),
            };
            let analysis = &view.analyses[&i];
            if let Some(there) = analysis.upstream.get(&stop.pick.commit) {
                // Its conflicts are with itself: the target has it already, rewritten.
                let onto = &view.rebases[i].onto_name;
                return vec![
                    seg("    □ ", fg(FAINT)),
                    seg(truncate(&stop.pick.subject, 90).into_owned(), fg(DIM)),
                    seg(format!("  in {onto} already as {}; picked again", short(there)), italic(FAINT)),
                ];
            }
            let mut segs =
                vec![seg("    ■ ", fg(AMBER_DIM)), seg(truncate(&stop.pick.subject, 90).into_owned(), fg(FG)), count];
            // The pick's side dropped is what a rebase most often loses.
            let dropped =
                stop.decisions.iter().filter(|d| d.conflict && d.kept() == Kept::Onto && !d.theirs.is_empty()).count();
            if dropped > 0 {
                segs.push(seg(format!(", {dropped} dropping the pick's side"), fg(YELLOW_DIM)));
            }
            segs
        }
        Row::Decision(i, s, d) => {
            let decision = &view.analyses[&i].stops[s].decisions[d];
            let kind = if decision.conflict { seg("conflict ", fg(AMBER)) } else { seg("edit     ", fg(DIM)) };
            let (said, style) = kept(decision, &view.rebases[i].onto_name);
            vec![
                seg("        ", fg(FAINT)),
                kind,
                seg(format!("{}:{}  ", decision.file.display(), decision.line()), fg(FG)),
                seg(said, style),
            ]
        }
        Row::LostHead(i) => {
            let lost = &view.analyses[&i].lost;
            let lines = lost.iter().flat_map(|l| &l.lines);
            let gone = lines.clone().filter(|l| l.now.is_none() && !l.comment).count();
            let changed = lines.clone().filter(|l| l.now.is_some() && !l.comment).count();
            let comments = lines.filter(|l| l.comment).count();
            vec![
                seg("    ■ ", fg(YELLOW_DIM)),
                seg("lost on the way", bold(FG)),
                seg(format!("  {gone} lines of code gone · {changed} changed · {comments} comments"), fg(FAINT)),
            ]
        }
        Row::Lost(i, l) => {
            let lost = &view.analyses[&i].lost[l];
            let comments = lost.lines.iter().all(|l| l.comment);
            let gone = lost.lines.iter().filter(|l| l.now.is_none()).count();
            let what = if comments {
                seg("comment  ", fg(FAINT))
            } else if gone > 0 {
                seg("gone     ", fg(YELLOW_DIM))
            } else {
                seg("changed  ", fg(DIM))
            };
            let first = lost.lines.first().map(|l| l.text.trim().to_owned()).unwrap_or_default();
            vec![
                seg("        ", fg(FAINT)),
                what,
                seg(format!("{}:{}", lost.file.display(), lost.line), if comments { fg(FAINT) } else { fg(FG) }),
                seg(format!("  {}", truncate(&first, 70)), fg(FAINT)),
            ]
        }
    }
}

/// The row under the cursor, at length.
pub fn detail(p: &mut Painter<'_>, app: &App, area: Rect) {
    let Some(view) = app.tab().rebase.as_ref() else { return };
    let inner = super::paint::inner(area);
    let Some(row) = view.selected() else {
        p.panel(area, &Panel { title: "DECIDED BY HAND", ..Panel::default() });
        return;
    };
    match row {
        Row::Decision(i, s, d) => {
            let stop = &view.analyses[&i].stops[s];
            let decision = &stop.decisions[d];
            let title =
                if decision.conflict { "CONFLICT, RESOLVED BY HAND" } else { "EDITED WHILE THE REBASE STOPPED" };
            let sub = vec![seg(format!("{}:{}", decision.file.display(), decision.line()), fg(FG))];
            let right = vec![seg(short(&stop.pick.commit), fg(FAINT))];
            p.panel(area, &Panel { title, sub, right, ..Panel::default() });
            sides(p, inner, &view.rebases[i], decision);
        }
        Row::Lost(i, l) => {
            let lost = &view.analyses[&i].lost[l];
            let sub = vec![seg(format!("{}:{}", lost.file.display(), lost.line), fg(FG))];
            p.panel(area, &Panel { title: "LOST ON THE WAY", sub, ..Panel::default() });
            lost_lines(p, inner, &view.rebases[i], lost);
        }
        Row::Stop(i, s) => {
            let stop = &view.analyses[&i].stops[s];
            p.panel(
                area,
                &Panel { title: "STOPPED PICK", sub: vec![seg(stop.pick.subject.clone(), fg(FG))], ..Panel::default() },
            );
            let lines = [
                format!("made as {}", short(&stop.pick.commit)),
                match &stop.original {
                    Some(original) => format!("picked from {}", short(original)),
                    None => "no commit of the branch before the rebase has its author, date and subject".into(),
                },
                format!(
                    "{} decisions by hand: ⏎ on one opens its line as the file is now; b makes a set of them",
                    stop.decisions.len()
                ),
            ];
            for (y, line) in (inner.y..inner.bottom()).zip(lines) {
                p.text(inner.x + 1, y, &line, fg(DIM), inner.right());
            }
        }
        Row::Rebase(i) | Row::Reading(i) | Row::LostHead(i) => {
            let rebase = &view.rebases[i];
            p.panel(area, &Panel { title: "REBASE", sub: vec![seg(rebase.title(), fg(FG))], ..Panel::default() });
            let tip = |sha: &Option<String>| sha.as_deref().map(short).unwrap_or_else(|| "—".into());
            let again = view
                .analyses
                .get(&i)
                .and_then(|a| Some((a.upstream.len(), a.decisions_again(), a.replayed_from.as_deref()?)));
            let again = match again {
                Some((n, decided, from)) if n > 0 => format!(
                    "{n} picks were in {onto} already, rewritten by another rebase, and conflicted with themselves \
                     ({decided} of the decisions): git rebase --onto {onto} {} would have picked only the commits after them",
                    short(from),
                    onto = rebase.onto_name
                ),
                _ => String::new(),
            };
            let lines = [
                format!("{} → {}, {}", when(rebase.started), when(rebase.ended), took(rebase)),
                format!(
                    "the branch was {} before, {} after; onto {} at {}",
                    tip(&rebase.before),
                    tip(&rebase.after),
                    rebase.onto_name,
                    short(&rebase.onto)
                ),
                format!(
                    "{} picks, {} of them stopped for a conflict or an edit",
                    rebase.picks.len(),
                    rebase.stops().count()
                ),
                "⏎ reads what was decided by hand in each pick that stopped, and what the branch lost".into(),
                again,
            ];
            for (y, line) in (inner.y..inner.bottom()).zip(lines) {
                p.text(inner.x + 1, y, &line, fg(DIM), inner.right());
            }
        }
    }
}

/// A conflict's two sides and what was kept, in three columns.
fn sides(p: &mut Painter<'_>, inner: Rect, rebase: &Rebase, decision: &Decision) {
    let third = inner.width / 3;
    let columns = [
        (format!("{} (onto)", rebase.onto_name), &decision.ours),
        ("the pick".to_owned(), &decision.theirs),
        ("kept".to_owned(), &decision.resolved),
    ];
    for (c, (head, lines)) in columns.iter().enumerate() {
        if !decision.conflict && c < 2 {
            continue;
        }
        let x = if decision.conflict { inner.x + 1 + third * c as u16 } else { inner.x + 1 };
        let right = if decision.conflict { x + third - 2 } else { inner.right() };
        p.text(x, inner.y, head, bold(if c == 2 { AMBER } else { FAINT }), right);
        if lines.is_empty() {
            p.text(x, inner.y + 1, "nothing", italic(FAINT), right);
        }
        for (y, line) in (inner.y + 1..inner.bottom()).zip(lines.iter()) {
            p.text(x, y, line, fg(if c == 2 { FG } else { DIM }), right);
        }
    }
}

fn lost_lines(p: &mut Painter<'_>, inner: Rect, rebase: &Rebase, lost: &Lost) {
    let before = rebase.before.as_deref().map(short).unwrap_or_default();
    p.text(
        inner.x + 1,
        inner.y,
        &format!("in the branch before the rebase ({before}), not after it"),
        bold(FAINT),
        inner.right(),
    );
    let mut y = inner.y + 1;
    for line in &lost.lines {
        if y >= inner.bottom() {
            break;
        }
        let style = if line.comment { fg(FAINT) } else { fg(FG) };
        p.text(inner.x + 1, y, &line.text, style, inner.right());
        y += 1;
        if let Some(now) = &line.now.as_ref().filter(|_| y < inner.bottom()) {
            p.segs(inner.x + 3, y, &[seg("now: ", italic(FAINT)), seg((*now).clone(), fg(DIM))], inner.right());
            y += 1;
        }
    }
}

/// What a decision kept, said for its row: `kept main's, dropped the pick's 7 lines`.
fn kept(decision: &Decision, onto: &str) -> (String, ratatui::style::Style) {
    let (ours, theirs, kept) = (decision.ours.len(), decision.theirs.len(), decision.resolved.len());
    if !decision.conflict {
        return (format!("{kept} lines"), fg(FAINT));
    }
    match decision.kept() {
        Kept::Onto if theirs > 0 => (format!("kept {onto}'s, dropped the pick's {theirs} lines"), fg(YELLOW_DIM)),
        Kept::Pick if ours > 0 => (format!("kept the pick's, dropped {onto}'s {ours} lines"), fg(YELLOW_DIM)),
        Kept::Onto => (format!("kept {onto}'s"), fg(FAINT)),
        Kept::Pick => ("kept the pick's".into(), fg(FAINT)),
        Kept::Both => (format!("merged by hand: {ours} and {theirs} → {kept} lines"), fg(DIM)),
        Kept::Neither => ("dropped both sides".into(), fg(YELLOW_DIM)),
    }
}

fn code_lost(lost: &[Lost]) -> usize {
    lost.iter().flat_map(|l| &l.lines).filter(|l| l.now.is_none() && !l.comment).count()
}

fn when(secs: i64) -> String {
    DateTime::from_timestamp(secs, 0)
        .map(|t| t.with_timezone(&Local).format("%d.%m %H:%M").to_string())
        .unwrap_or_default()
}

/// `25.5 h`, `6 min`
fn took(rebase: &Rebase) -> String {
    let secs = (rebase.ended - rebase.started).max(0);
    if secs >= 3600 { format!("{:.1} h", secs as f64 / 3600.0) } else { format!("{} min", secs / 60) }
}

fn short(sha: &str) -> String {
    sha.chars().take(9).collect()
}
