use std::collections::HashMap;
use std::time::Duration;

use ratatui::layout::Rect;
use ratatui::style::Style;
use tuitor_core::breakpoints::Breakpoint;
use tuitor_core::config::home_relative;
use tuitor_core::session::SessionState;
use tuitor_core::snapshot::Change;

use super::logo::{self, Size};
use super::paint::{Painter, Panel, seg, truncate, truncate_left};
use super::syntax::{Lang, highlight};
use super::theme::*;
use crate::app::App;
use crate::tab::Tab;

const GUTTER: u16 = 8;
const INLINE_VALUE_WIDTH: usize = 28;

pub fn draw(p: &mut Painter<'_>, app: &App, area: Rect, focus: bool) {
    let tab = app.tab();
    let path = tab.source_path();
    let frame = tab.selected_frame();
    let top_frame = tab.viewed().and_then(|s| s.top());
    let rel = path.as_deref().map(|p| p.strip_prefix(tab.engine.workspace()).unwrap_or(p).display().to_string());

    let mut right = Vec::new();
    if tab.ui.source.browse.is_some() {
        right.push(seg("browsing", fg(DIM)));
    } else if let Some(frame) = frame {
        right.push(seg(truncate(&frame.name, 40).into_owned(), fg(DIM)));
    }
    let sub_width = usize::from(area.width / 2);
    let sub = rel.map(|r| vec![seg(truncate_left(&r, sub_width).into_owned(), fg(DIM))]).unwrap_or_default();
    p.panel(
        area,
        &Panel {
            title: "SOURCE",
            sub,
            right,
            keys: if focus { vec![("b", "bp"), (".", "follow")] } else { vec![] },
            focus,
        },
    );

    let inner = super::paint::inner(area);
    let Some(path) = path else {
        empty(p, app, inner);
        return;
    };
    let Some(file) = app.sources.borrow_mut().get(&path) else {
        p.text(inner.x + 2, inner.y + 1, &format!("cannot read {}", path.display()), fg(DIM), inner.right());
        p.text(
            inner.x + 2,
            inner.y + 2,
            "(remote path? add pathMappings to the launch configuration)",
            fg(FAINT),
            inner.right(),
        );
        return;
    };

    let lines = u32::try_from(file.lines.len()).unwrap_or(u32::MAX);
    let height = u32::from(inner.height);
    let in_history = tab.in_history();
    // The execution line, if the shown frame is in this file.
    let exec = frame.filter(|f| f.path.as_deref() == Some(path.as_path())).map(|f| f.line);
    let exec_is_top = frame.zip(top_frame).is_some_and(|(a, b)| a.id == b.id);
    let cursor = tab.ui.source.cursor.clamp(1, lines.max(1));

    let recenter = tab.ui.source.recenter.replace(false);
    let top = window_top(tab.ui.source.top.get(), cursor, height, lines, recenter);
    tab.ui.source.top.set(top);

    let canonical = tuitor_canonical(&path);
    let breakpoints: HashMap<u32, &Breakpoint> =
        tab.engine.breakpoints().in_file(&canonical).map(|b| (b.line, b)).collect();
    let hit: Vec<u32> = tab.viewed().map(|s| s.hit_breakpoints.clone()).unwrap_or_default();
    let inline = if exec_is_top && tab.ui.source.browse.is_none() {
        inline_values(tab, &file.lines, top, exec)
    } else {
        HashMap::new()
    };
    let lang = Lang::from_path(&path);
    let browsing = tab.ui.source.browse.is_some() || focus;

    for row in 0..height {
        let n = top + row;
        if n > lines {
            break;
        }
        let y = inner.y + row as u16;
        let x = inner.x;
        let is_exec = exec == Some(n);
        let row_bg = if is_exec {
            Some(if in_history {
                SNAPSHOT_BG
            } else if exec_is_top {
                AMBER_BG
            } else {
                SEL2
            })
        } else if browsing && n == cursor {
            Some(SEL)
        } else {
            None
        };
        if let Some(bg) = row_bg {
            p.band(x, y, inner.width, bg);
        }
        if let Some(bp) = breakpoints.get(&n) {
            let (glyph, color) = marker(bp, hit.contains(&bp.id));
            p.text(x, y, glyph, bold(color), inner.right());
        }
        let number_style = if is_exec {
            bold(if exec_is_top && !in_history { AMBER } else { DIM })
        } else if n == cursor && browsing {
            fg(FG)
        } else {
            fg(FAINT)
        };
        p.text(x + 1, y, &format!("{n:>5} "), number_style, inner.right());
        if is_exec {
            p.text(
                x + 7,
                y,
                if exec_is_top { "▶" } else { "▷" },
                bold(if in_history || !exec_is_top { DIM } else { AMBER }),
                inner.right(),
            );
        }
        let line = file.lines.get((n - 1) as usize).map(String::as_str).unwrap_or_default();
        let mut cx = x + GUTTER + 1;
        for (text, style) in highlight(line, lang) {
            cx = p.text(cx, y, &text, style, inner.right());
            if cx >= inner.right() {
                break;
            }
        }
        if let Some(values) = inline.get(&n)
            && let Some(segs) = fit_inline(values, inner.right().saturating_sub(cx + 3))
        {
            p.segs(cx + 3, y, &segs, inner.right());
        }
    }
}

/// First visible line of `height` rows over a file of `lines` lines that shows `cursor`:
/// with `recenter` the cursor line is in the middle, otherwise `top` scrolls only as far as
/// needed. The window never starts before line 1 nor ends after the last line of a file
/// longer than it.
fn window_top(top: u32, cursor: u32, height: u32, lines: u32, recenter: bool) -> u32 {
    let mut top = if recenter { cursor.saturating_sub(height.saturating_sub(1) / 2) } else { top };
    if cursor < top {
        top = cursor;
    } else if cursor >= top + height {
        top = cursor + 1 - height;
    }
    top.min(lines.saturating_sub(height) + 1).max(1)
}

fn marker(bp: &Breakpoint, hit: bool) -> (&'static str, ratatui::style::Color) {
    if !bp.enabled {
        ("○", GHOST)
    } else if bp.verified() == Some(false) {
        ("◌", RED_DIM)
    } else if bp.is_logpoint() {
        ("◆", YELLOW_DIM)
    } else if bp.condition.is_some() {
        ("●", YELLOW_DIM)
    } else if hit {
        ("●", RED)
    } else {
        ("●", RED_DIM)
    }
}

struct InlineValue {
    name: String,
    value: String,
    changed: bool,
}

impl InlineValue {
    fn segs(&self, max_value: usize) -> Vec<super::paint::Seg<'static>> {
        let value = truncate(&self.value, max_value).into_owned();
        let (name_style, value_style): (Style, Style) =
            if self.changed { (italic(DIM), bold(GREEN)) } else { (italic(FAINT), italic(DIM)) };
        vec![
            seg("◂ ", fg(FAINT)),
            seg(format!("{} ", self.name), name_style),
            seg(value, value_style),
            seg("  ", fg(FAINT)),
        ]
    }
}

/// Shortens values, then drops unchanged ones, until the annotations fit in `room`.
fn fit_inline(values: &[InlineValue], room: u16) -> Option<Vec<super::paint::Seg<'static>>> {
    let changed: Vec<&InlineValue> = values.iter().filter(|v| v.changed).collect();
    let all: Vec<&InlineValue> = values.iter().collect();
    let subsets = [all.as_slice(), changed.as_slice(), &all[..all.len().min(1)]];
    for max_value in [INLINE_VALUE_WIDTH, 14, 8] {
        for subset in subsets.iter().filter(|s| !s.is_empty()) {
            let segs: Vec<_> = subset.iter().flat_map(|v| v.segs(max_value)).collect();
            if super::paint::width(&segs) <= room {
                return Some(segs);
            }
        }
    }
    None
}

/// Values next to the last line (up to the execution line) where each variable appears.
fn inline_values(tab: &Tab, lines: &[String], top: u32, exec: Option<u32>) -> HashMap<u32, Vec<InlineValue>> {
    let mut out: HashMap<u32, Vec<InlineValue>> = HashMap::new();
    let (Some(snapshot), Some(exec)) = (tab.viewed(), exec) else { return out };
    let changes = tab.changes();
    let first = top.max(exec.saturating_sub(40)).max(1);
    for scope in snapshot.scopes.iter().filter(|s| s.default_open) {
        for var in scope.vars.iter().flatten() {
            let last =
                (first..=exec).rev().find(|n| lines.get((*n - 1) as usize).is_some_and(|l| mentions(l, &var.name)));
            let Some(line) = last else { continue };
            let entry = out.entry(line).or_default();
            if entry.len() >= 2 {
                continue;
            }
            let changed = matches!(
                changes.get(&vec![scope.name.clone(), var.name.clone()]),
                Some(Change::Changed { .. } | Change::New)
            );
            entry.push(InlineValue { name: var.name.clone(), value: one_line(&var.value), changed });
        }
    }
    out
}

/// Whole-word occurrence (`$i` must not match `$item`).
fn mentions(line: &str, name: &str) -> bool {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    line.match_indices(name).any(|(at, _)| {
        let before = line[..at].chars().next_back();
        let after = line[at + name.len()..].chars().next();
        !before.is_some_and(|c| is_word(c) || (c == '$' && !name.starts_with('$'))) && !after.is_some_and(is_word)
    })
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn empty(p: &mut Painter<'_>, app: &App, inner: Rect) {
    let tab = app.tab();
    let configs: Vec<String> = match &tab.project {
        Some(project) => project.configurations.iter().map(|c| format!("{}  ({})", c.name(), c.adapter())).collect(),
        None => app.recent_projects.iter().map(|dir| home_relative(dir)).collect(),
    };
    let listed = configs.len().min(6);
    // The message, a gap, the list with a line above and a gap below, then two hints.
    let hints_height = if listed == 0 { 4 } else { 6 + listed as u16 };
    let logo = [Size::Full, Size::Small].into_iter().find(|size| {
        let (w, h) = size.cells();
        w + 2 * logo::PAD <= inner.width && 1 + h + 1 + hints_height <= inner.height
    });

    let mut x = inner.x + 3;
    let mut y = inner.y + 1;
    if let Some(size) = logo {
        let (w, h) = size.cells();
        let logo_x = match size {
            Size::Full => inner.x + (inner.width - w) / 2,
            Size::Small => x,
        };
        y = inner.y + ((inner.height - h - 1 - hints_height) / 2).max(1);
        logo::draw(p, logo_x, y, size, waves_clock(app));
        x = logo_x + size.indent();
        y += h + 1;
    }

    let line = |p: &mut Painter<'_>, y: u16, key: &str, text: &str| {
        let x2 = p.text(x, y, key, bold(KEY), inner.right());
        p.text(x2.max(x + 4), y, text, fg(DIM), inner.right());
    };
    let message = match (tab.active(), &tab.project) {
        (Some(s), _) if s.is_alive() => "Nothing is stopped yet.",
        (Some(_), _) => "The session ended.",
        (None, Some(_)) => "No debug session.",
        (None, None) => "No project in this tab.",
    };
    p.text(x, y, message, bold(FG), inner.right());
    y += 2;
    if listed > 0 {
        match tab.selected_configuration() {
            Some(config) => line(p, y, "S", &format!("starts {}; g p picks another:", config.name())),
            None => {
                p.text(x, y, "Opened recently:", fg(DIM), inner.right());
            }
        }
        y += 1;
        for item in configs.iter().take(listed) {
            p.text(x + 4, y, item, fg(FAINT), inner.right());
            y += 1;
        }
        y += 1;
    }
    match tab.project {
        Some(_) => line(p, y, "f", "find a file, then b toggles a breakpoint on the cursor line"),
        None => line(p, y, "O", "open a project in this tab"),
    }
    line(p, y + 1, "/", "command line · ? lists every command and its key");
}

/// Time for the waves, or `None` to hold them still: they never move while a
/// program is stopped or something is open over them.
fn waves_clock(app: &App) -> Option<Duration> {
    let stopped = app.tab().engine.sessions().iter().any(|s| s.state == SessionState::Stopped);
    let clock = app.ui.clock.filter(|_| !stopped && app.ui.overlay.is_none());
    if clock.is_some() {
        app.ui.animate(logo::FRAME);
    }
    clock
}

fn tuitor_canonical(path: &std::path::Path) -> std::path::PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::{mentions, window_top};

    #[test]
    fn recentring_puts_the_cursor_in_the_middle_within_the_file() {
        // 17 rows over 79 lines: 8 above and 8 below line 50.
        assert_eq!(window_top(1, 50, 17, 79, true), 42);
        // An even height leaves the extra row below.
        assert_eq!(window_top(1, 50, 16, 79, true), 43);
        // Near the top the window starts at line 1 rather than above it.
        assert_eq!(window_top(30, 5, 17, 79, true), 1);
        assert_eq!(window_top(30, 9, 17, 79, true), 1);
        // Near the end it stops at the last line.
        assert_eq!(window_top(1, 77, 17, 79, true), 63);
        // A file shorter than the window starts at line 1.
        assert_eq!(window_top(4, 8, 17, 10, true), 1);
    }

    #[test]
    fn moving_the_cursor_scrolls_only_as_far_as_needed() {
        assert_eq!(window_top(42, 55, 17, 79, false), 42);
        assert_eq!(window_top(42, 59, 17, 79, false), 43);
        assert_eq!(window_top(42, 40, 17, 79, false), 40);
        assert_eq!(window_top(70, 75, 17, 79, false), 63);
    }

    #[test]
    fn whole_word_mentions() {
        assert!(mentions("    $total += $item['price'];", "$total"));
        assert!(!mentions("foreach ($items as $item)", "$i"));
        assert!(mentions("for price in prices {", "price"));
        assert!(!mentions("let total_price = 1;", "price"));
        assert!(!mentions("echo $sum;", "sum"));
    }
}
