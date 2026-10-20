//! HISTORY: one row per stop, one column per variable, so you can see in which
//! step a value changed; below it the TIMELINE of those changes over every stop.

use ratatui::layout::Rect;
use tuitor_core::snapshot::Snapshot;

use super::breakpoints::stop_marks;
use super::paint::{Painter, Panel, Seg, seg, truncate};
use super::theme::*;
use super::watch::number;
use crate::tab::Tab;
use crate::timeline::{Track, cell};

const MAX_CELL: u16 = 16;

/// Columns: watches first, then top-level variables of the viewed stop's top frame.
fn columns(tab: &Tab) -> Vec<String> {
    let mut cols: Vec<String> = tab.engine.watches().to_vec();
    if let Some(snapshot) = tab.viewed() {
        for scope in snapshot.scopes(0).unwrap_or_default().iter().filter(|s| s.default_open) {
            for var in scope.vars.iter().flatten() {
                if !cols.contains(&var.name) {
                    cols.push(var.name.clone());
                }
            }
        }
    }
    cols
}

pub fn table(p: &mut Painter<'_>, tab: &Tab, area: Rect, focus: bool) {
    let session = tab.active();
    let history: &[Snapshot] = session.map(|s| s.history.as_slice()).unwrap_or_default();
    let sub = vec![seg("value of every variable at every stop", fg(DIM))];
    let passed = history.iter().filter(|s| s.silent).count();
    let mut right = vec![seg(format!("{} stops", history.len()), fg(DIM))];
    if passed > 0 {
        right.push(seg(format!(" · ↷ {passed} passed"), fg(FAINT)));
    }
    let keys = if focus { vec![("j/k", "stop"), ("h/l", "column"), ("⏎", "open")] } else { vec![] };
    p.panel(area, &Panel { title: "HISTORY", sub, right, keys, focus });

    let inner = super::paint::inner(area);
    if history.is_empty() {
        p.text(
            inner.x + 2,
            inner.y + 1,
            "every stop is recorded here; step through the program to fill it",
            fg(FAINT),
            inner.right(),
        );
        return;
    }
    let cols = columns(tab);
    let viewed = tab.viewed().map_or(history.len() as u32, |s| s.number);
    let latest = history.len() as u32;

    // Fixed columns.
    let x_num = inner.x + 2;
    let x_time = x_num + 5;
    let x_reason = x_time + 13;
    let x_loc = x_reason + 12;
    let loc_w = (inner.width / 5).clamp(18, 34);
    let x_vars = x_loc + loc_w + 1;

    // Variable columns, scrolled so the selected one is visible.
    let widths: Vec<u16> = cols
        .iter()
        .map(|c| {
            let longest = history.iter().filter_map(|s| cell(s, c)).map(|v| v.chars().count()).max().unwrap_or(1);
            (longest.max(c.chars().count()) as u16 + 2).min(MAX_CELL)
        })
        .collect();
    let selected_col = tab.ui.history_col.min(cols.len().saturating_sub(1));
    let room = inner.right().saturating_sub(x_vars + 4);
    let mut first = 0;
    while first < selected_col && widths[first..=selected_col].iter().sum::<u16>() > room {
        first += 1;
    }

    let y0 = inner.y;
    p.text(x_num, y0, "#", bold(FAINT), inner.right());
    p.text(x_time, y0, "time", bold(FAINT), inner.right());
    p.text(x_reason, y0, "reason", bold(FAINT), inner.right());
    p.text(x_loc, y0, "location", bold(FAINT), inner.right());
    let mut x = x_vars;
    let mut shown = Vec::new();
    for (i, name) in cols.iter().enumerate().skip(first) {
        let w = widths[i];
        if x + w > inner.right() - 3 {
            break;
        }
        let style = if i == selected_col { bold(WHITE) } else { bold(DIM) };
        let label = truncate(name, usize::from(w - 1)).into_owned();
        p.text(x + w - 1 - label.chars().count() as u16, y0, &label, style, inner.right());
        shown.push((i, x, w));
        x += w;
    }
    p.text(inner.right() - 2, y0, "Δ", bold(FAINT), inner.right());

    let height = usize::from(inner.height.saturating_sub(1));
    // Keep the viewed row visible.
    let cursor = (viewed as usize).saturating_sub(1);
    let offset = cursor.saturating_sub(height.saturating_sub(1)).min(history.len().saturating_sub(height));
    for (i, snapshot) in history.iter().enumerate().skip(offset).take(height) {
        let y = y0 + 1 + (i - offset) as u16;
        let is_cursor = snapshot.number == viewed;
        let is_live = snapshot.number == latest;
        if is_cursor {
            p.band(inner.x, y, inner.width, SEL2);
        }
        let mark = if is_cursor {
            "›"
        } else if is_live {
            "●"
        } else {
            " "
        };
        p.text(inner.x, y, mark, bold(if is_cursor { WHITE } else { AMBER }), inner.right());
        p.text(x_num, y, &format!("#{}", snapshot.number), fg(if is_cursor { FG } else { DIM }), inner.right());
        p.text(x_time, y, &snapshot.time.format("%H:%M:%S%.3f").to_string(), fg(FAINT), inner.right());
        p.segs(x_reason, y, &reason(snapshot), x_loc - 1);
        let location = snapshot.top().map(|f| f.location()).unwrap_or_default();
        let loc_style = if is_live {
            fg(AMBER)
        } else if is_cursor {
            fg(FG)
        } else {
            fg(DIM)
        };
        p.text(x_loc, y, &location, loc_style, x_vars - 1);

        let previous = i.checked_sub(1).map(|k| &history[k]);
        let mut changed = 0;
        for (col, cx, w) in &shown {
            let name = &cols[*col];
            let value = cell(snapshot, name);
            let before = previous.and_then(|s| cell(s, name));
            let is_change = matches!((&value, &before), (Some(v), Some(b)) if v != b);
            changed += usize::from(is_change);
            if is_cursor && *col == selected_col {
                p.band(cx.saturating_sub(1), y, w + 1, CELL);
            }
            let text = value.clone().unwrap_or_else(|| "·".into());
            let text = truncate(&text, usize::from(w - 1)).into_owned();
            let style = match (&value, is_change) {
                (None, _) => fg(GHOST),
                (Some(_), true) if is_cursor || is_live => bold(GREEN),
                (Some(_), true) => fg(GREEN_DIM),
                (Some(_), false) if is_cursor => fg(FG),
                _ => fg(DIM),
            };
            p.text(cx + w - 1 - text.chars().count() as u16, y, &text, style, cx + w);
        }
        if changed > 0 {
            p.text(
                inner.right() - 2,
                y,
                &changed.to_string(),
                fg(if is_cursor || is_live { GREEN } else { GREEN_DIM }),
                inner.right(),
            );
        }
    }
}

/// The selected column across all stops: a bar chart for numbers and the list of changes.
pub fn value(p: &mut Painter<'_>, tab: &Tab, area: Rect) {
    let cols = columns(tab);
    let Some(name) = cols.get(tab.ui.history_col.min(cols.len().saturating_sub(1))) else {
        p.panel(area, &Panel { title: "VALUE", ..Panel::default() });
        return;
    };
    let history: &[Snapshot] = tab.active().map(|s| s.history.as_slice()).unwrap_or_default();
    let viewed = tab.viewed().map_or(0, |s| s.number);
    let sub = vec![seg(name.clone(), bold(WHITE))];
    p.panel(
        area,
        &Panel { title: "VALUE", sub, right: vec![seg(format!("at #{viewed}"), fg(DIM))], keys: vec![], focus: false },
    );
    let inner = super::paint::inner(area);

    let values: Vec<(u32, Option<String>)> = history.iter().map(|s| (s.number, cell(s, name))).collect();
    if let Some((_, Some(current))) = values.iter().find(|(n, _)| *n == viewed) {
        let mut segs = vec![seg(format!("#{viewed} "), fg(DIM)), seg(truncate(current, 60).into_owned(), bold(WHITE))];
        if let Some((n, Some(before))) = values.iter().rev().find(|(n, v)| *n < viewed && v.as_deref() != Some(current))
        {
            segs.push(seg(format!("   was {} at #{n}", truncate(before, 24)), fg(DIM)));
        }
        p.segs(inner.x + 1, inner.y, &segs, inner.right());
    }

    // Numeric values get a bar chart over the last stops that fit.
    let numeric: Vec<(u32, f64)> = values.iter().filter_map(|(n, v)| Some((*n, number(v.as_deref()?)?))).collect();
    let chart_h = inner.height.saturating_sub(8).min(10);
    let mut y = inner.y + 2;
    if numeric.len() >= 2 && chart_h >= 3 {
        let bar_w = 7u16;
        let fit = usize::from(inner.width.saturating_sub(4) / bar_w);
        let shown = &numeric[numeric.len().saturating_sub(fit)..];
        let hi = shown.iter().map(|(_, v)| *v).fold(f64::MIN, f64::max);
        let lo = shown.iter().map(|(_, v)| *v).fold(f64::MAX, f64::min).min(0.0);
        let base = y + chart_h;
        for (k, (n, v)) in shown.iter().enumerate() {
            let x = inner.x + 2 + k as u16 * bar_w;
            let ratio = if hi > lo { (v - lo) / (hi - lo) } else { 0.5 };
            let full = (ratio * f64::from(chart_h - 1)).round() as u16;
            let color = if *n == viewed {
                WHITE
            } else if *n as usize == history.len() {
                AMBER_DIM
            } else {
                BAR_GREY
            };
            for r in 0..full.max(1) {
                p.text(x, base - 1 - r, "█████", fg(color), inner.right());
            }
            let label = truncate(&format_number(*v), 6).into_owned();
            p.text(x, base - 1 - full.max(1), &label, fg(if *n == viewed { WHITE } else { FAINT }), inner.right());
            p.text(x + 1, base, &format!("#{n}"), if *n == viewed { bold(WHITE) } else { fg(FAINT) }, inner.right());
        }
        y = base + 2;
    }

    p.text(inner.x + 1, y, "CHANGES", bold(FAINT), inner.right());
    y += 1;
    let mut last: Option<&str> = None;
    for (n, v) in &values {
        let Some(v) = v.as_deref() else { continue };
        if last.is_some_and(|l| l != v) {
            if y >= inner.bottom() {
                break;
            }
            let location = history.get(*n as usize - 1).and_then(|s| s.top()).map(|f| f.location()).unwrap_or_default();
            let style = if *n == viewed { fg(WHITE) } else { fg(DIM) };
            let segs = [
                seg(format!("#{n:<4}"), style),
                seg(format!("{:<10}", truncate(v, 10)), fg(GREEN_DIM)),
                seg(format!(" {location}"), fg(FAINT)),
            ];
            p.segs(inner.x + 1, y, &segs, inner.right());
            y += 1;
        }
        last = Some(v);
    }
}

/// The rows TIMELINE would take: the stops, one for each value that changed, the change
/// under the cursor, and its frame.
pub fn timeline_height(tab: &Tab) -> u16 {
    let values = tab.timeline_tracks().len().max(1);
    u16::try_from(values).unwrap_or(u16::MAX).saturating_add(4)
}

/// TIMELINE: a row for each value that changed, with a mark at every stop it changed at in
/// the colour of that stop's breakpoint set, and the stops along the top: `●` where the
/// program waited, `↷` where it went on, `·` where no breakpoint stopped it. The stop on view
/// is the lighter column, and below is what changed there.
pub fn timeline(p: &mut Painter<'_>, tab: &Tab, area: Rect, focus: bool) {
    let history: &[Snapshot] = tab.active().map(|s| s.history.as_slice()).unwrap_or_default();
    let tracks = tab.timeline_tracks();
    let count: usize = tracks.iter().map(|t| t.changes.len()).sum();
    let sub = vec![seg("what changed, where and when", fg(DIM))];
    let right = vec![seg(format!("{count} changes of {} values", tracks.len()), fg(DIM))];
    let keys = if focus { vec![("j/k", "value"), ("h/l", "change"), ("⏎", "open")] } else { vec![] };
    p.panel(area, &Panel { title: "TIMELINE", sub, right, keys, focus });
    let inner = super::paint::inner(area);
    if tracks.is_empty() || inner.height < 3 {
        let text = "no value has changed from one stop to the next yet";
        p.text(inner.x + 2, inner.y, text, fg(FAINT), inner.right());
        return;
    }
    let x0 = inner.x + 2 + 16;
    let width = usize::from(inner.right().saturating_sub(x0 + 1)).max(1);
    // Up to four columns a stop while they fit, else several stops a column.
    let (per, bucket) = match history.len() {
        n if n <= width => ((width / n.max(1)).min(4), 1),
        n => (1, n.div_ceil(width)),
    };
    let column = |number: u32| x0 + u16::try_from((number as usize).saturating_sub(1) / bucket * per).unwrap_or(0);
    let viewed = tab.viewed().map(|s| s.number);
    let cursor = tab.ui.timeline.pos.min(tracks.len() - 1);
    let height = usize::from(inner.height - 2);
    let offset = tab.ui.timeline.window(height, tracks.len());

    if let Some(number) = viewed {
        for y in inner.y..inner.bottom() - 1 {
            p.band(column(number), y, 1, SEL2);
        }
    }
    p.text(inner.x + 2, inner.y, "stops", fg(FAINT), x0);
    for snapshot in history {
        let (glyph, color) = match (snapshot.sets.first(), snapshot.silent) {
            (Some(set), false) => ("●", set_color(*set, true)),
            (Some(set), true) => ("↷", set_color(*set, true)),
            (None, _) => ("·", FAINT),
        };
        p.text(column(snapshot.number), inner.y, glyph, fg(color), inner.right());
    }
    for (i, track) in tracks.iter().enumerate().skip(offset).take(height) {
        let y = inner.y + 1 + (i - offset) as u16;
        let selected = i == cursor;
        if selected && focus {
            p.band(inner.x, y, x0 - inner.x - 1, SEL2);
        }
        p.text(inner.x + 2, y, &track.name, fg(if selected { WHITE } else { DIM }), x0 - 1);
        for change in &track.changes {
            let set = history.get(change.number as usize - 1).and_then(|s| s.sets.first());
            let color = set.map_or(DIM, |set| set_color(*set, true));
            let style = if Some(change.number) == viewed { bold(color) } else { fg(color) };
            p.text(column(change.number), y, "◆", style, inner.right());
        }
    }
    let at = viewed.and_then(|n| history.get(n as usize - 1));
    if let Some(snapshot) = at {
        p.segs(inner.x + 2, inner.bottom() - 1, &change_line(snapshot, &tracks[cursor]), inner.right());
    }
}

/// "#5 · 10:42:07.412 · ● orders.php:68 · $total 8900 → 13800": the value under the cursor at
/// the stop on view.
fn change_line(snapshot: &Snapshot, track: &Track) -> Vec<Seg<'static>> {
    let mut segs = vec![
        seg(format!("#{} ", snapshot.number), bold(WHITE)),
        seg(snapshot.time.format("%H:%M:%S%.3f  ").to_string(), fg(FAINT)),
    ];
    segs.extend(stop_marks(&snapshot.sets, if snapshot.silent { "↷" } else { "●" }));
    segs.push(seg(" ", fg(FAINT)));
    segs.push(seg(snapshot.top().map(|f| f.location()).unwrap_or_default(), fg(FG)));
    segs.push(seg("   ", fg(FAINT)));
    match track.at(snapshot.number) {
        Some(change) => segs.extend([
            seg(format!("{} ", track.name), fg(FG)),
            seg(truncate(&change.was, 30).into_owned(), fg(DIM)),
            seg(" → ", fg(FAINT)),
            seg(truncate(&change.now, 30).into_owned(), bold(GREEN)),
        ]),
        None => segs.push(seg(format!("{} did not change here; h and l go to its changes", track.name), fg(FAINT))),
    }
    segs
}

/// Why the program stopped: a breakpoint by its set's colour, `●` where it waited and `↷`
/// where it went on by itself, or the adapter's reason.
fn reason(snapshot: &Snapshot) -> Vec<Seg<'static>> {
    let Some(id) = snapshot.hit_breakpoints.first() else { return vec![seg(snapshot.reason.clone(), fg(FAINT))] };
    let (glyph, text) = if snapshot.silent { ("↷", fg(FAINT)) } else { ("●", fg(DIM)) };
    let mut segs = stop_marks(&snapshot.sets, glyph);
    segs.push(seg(format!(" bp #{id}"), text));
    segs
}

fn format_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e12 { format!("{v:.0}") } else { format!("{v:.2}") }
}
