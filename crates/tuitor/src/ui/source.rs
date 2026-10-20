use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use ratatui::layout::Rect;
use ratatui::style::Style;
use tuitor_core::breakpoints::Breakpoint;
use tuitor_core::config::home_relative;
use tuitor_core::session::{SessionState, SourceText};
use tuitor_core::snapshot::{Change, Frame};
use unicode_width::UnicodeWidthChar;

use super::logo::{self, Size};
use super::paint::{Painter, Panel, Seg, seg, truncate, truncate_left};
use super::syntax::{Lang, highlight};
use super::theme::*;
use crate::app::App;
use crate::model::Shown;
use crate::search::{self, Match};
use crate::tab::Tab;

const GUTTER: u16 = 8;
const INLINE_VALUE_WIDTH: usize = 28;

pub fn draw(p: &mut Painter<'_>, app: &App, area: Rect, focus: bool) {
    let tab = app.tab();
    let shown = tab.shown_source();
    let frame = tab.selected_frame();
    let top_frame = tab.viewed().and_then(|s| s.top());

    let mut right = Vec::new();
    if tab.ui.source.browse.is_some() {
        right.push(seg("browsing", fg(DIM)));
    } else if let Some(frame) = frame {
        right.push(seg(truncate(&frame.name, 40).into_owned(), fg(DIM)));
    }
    let sub_width = usize::from(area.width / 2);
    let sub = match &shown {
        Some(Shown::File(path)) => {
            let rel = path.strip_prefix(tab.engine.workspace()).unwrap_or(path).display().to_string();
            vec![seg(truncate_left(&rel, sub_width).into_owned(), fg(DIM))]
        }
        Some(Shown::Adapter { name, .. }) => {
            vec![seg(truncate_left(name, sub_width).into_owned(), fg(DIM)), seg(" · from adapter", fg(FAINT))]
        }
        None => Vec::new(),
    };
    // The adapter's own source is read-only, and takes no breakpoints.
    let keys = match (&shown, focus) {
        (_, false) => vec![],
        (Some(Shown::Adapter { .. }), true) => vec![(".", "follow")],
        (_, true) => vec![("b", "bp"), (".", "follow")],
    };
    p.panel(area, &Panel { title: "SOURCE", sub, right, keys, focus });

    let inner = super::paint::inner(area);
    let Some(shown) = shown else {
        match frame {
            Some(frame) => no_source(p, tab, frame, inner),
            None => empty(p, app, inner),
        }
        return;
    };
    let Some(file) = app.shown_lines() else {
        let lines = unavailable(app, &shown);
        for (row, line) in lines.iter().enumerate() {
            p.segs(inner.x + 2, inner.y + 1 + row as u16, line, inner.right());
        }
        return;
    };

    let lines = u32::try_from(file.lines.len()).unwrap_or(u32::MAX);
    let height = u32::from(inner.height);
    let in_history = tab.in_history();
    // The execution line, if the shown frame's source is the one shown.
    let exec = frame.filter(|f| shows(&shown, f)).map(|f| f.line);
    let exec_is_top = frame.zip(top_frame).is_some_and(|(a, b)| a.id == b.id);
    let cursor = tab.ui.source.cursor.clamp(1, lines.max(1));

    let recenter = tab.ui.source.recenter.replace(false);
    let top = window_top(tab.ui.source.top.get(), cursor, height, lines, recenter);
    tab.ui.source.top.set(top);
    tab.ui.source.rows.set(height);
    let mut found: HashMap<u32, Vec<Match>> = HashMap::new();
    for m in tab.ui.search.iter().flat_map(|search| search::find(&file.lines, &search.query)) {
        found.entry(m.line).or_default().push(m);
    }
    let current = tab.ui.search.as_ref().and_then(|search| search.at);

    // The adapter's own source has no path for breakpoints to be in.
    let file_path = match &shown {
        Shown::File(path) => Some(tuitor_canonical(path)),
        Shown::Adapter { .. } => None,
    };
    let breakpoints: HashMap<u32, &Breakpoint> =
        file_path.iter().flat_map(|path| tab.engine.breakpoints().in_file(path)).map(|b| (b.line, b)).collect();
    let hit: Vec<u32> = tab.viewed().map(|s| s.hit_breakpoints.clone()).unwrap_or_default();
    let inline =
        if tab.ui.source.browse.is_none() { inline_values(tab, &file.lines, top, exec) } else { HashMap::new() };
    let lang = match &shown {
        Shown::File(path) => Lang::from_path(path),
        Shown::Adapter { session, name, .. } => adapter_lang(tab, *session, name),
    };
    // Lines a stop or the trace reached, as FLOW knows them.
    let flow = tab.flow();
    let reached = flow.as_ref().zip(file_path.as_deref()).and_then(|(view, path)| view.flow.row(path));
    let reached = reached.map(|row| &row.lines);
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
        } else if reached.is_some_and(|lines| lines.contains_key(&n)) {
            p.text(x, y, "│", fg(GHOST), inner.right());
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
        if let Some(found) = found.get(&n) {
            mark_found(p, line, found, current, (x + GUTTER + 1, y), inner.right());
        }
        if let Some(values) = inline.get(&n)
            && let Some(segs) = fit_inline(values, inner.right().saturating_sub(cx + 3))
        {
            p.segs(cx + 3, y, &segs, inner.right());
        }
    }
}

/// Bands under the matches of the search on a line: dim, and brighter for the one the cursor
/// went to. `at` is where the line's text starts.
fn mark_found(
    p: &mut Painter<'_>,
    line: &str,
    found: &[Match],
    current: Option<(u32, usize)>,
    at: (u16, u16),
    right: u16,
) {
    let chars: Vec<char> = line.chars().collect();
    let column = |end: usize| -> u16 {
        let width: usize = chars[..end.min(chars.len())].iter().map(|c| c.width().unwrap_or(0)).sum();
        at.0.saturating_add(u16::try_from(width).unwrap_or(u16::MAX))
    };
    for m in found {
        let (start, end) = (column(m.start), column(m.start + m.len).min(right));
        if start >= end {
            continue;
        }
        if current == Some((m.line, m.start)) {
            let text: String = chars[m.start..(m.start + m.len).min(chars.len())].iter().collect();
            p.text(start, at.1, &text, bold(WHITE).bg(FOUND_HERE), end);
        } else {
            p.band(start, at.1, end - start, FOUND);
        }
    }
}

/// Whether SOURCE shows `frame`'s source.
fn shows(shown: &Shown, frame: &Frame) -> bool {
    match shown {
        Shown::File(path) => frame.path.as_deref() == Some(path.as_path()),
        Shown::Adapter { reference, .. } => frame.source_reference == Some(*reference),
    }
}

/// The language of an adapter's own source: by its name, else the language the adapter
/// debugs (js-debug names Node's internals `<node_internals>/internal/…`).
fn adapter_lang(tab: &Tab, session: tuitor_core::session::SessionId, name: &str) -> Lang {
    match Lang::from_path(Path::new(name)) {
        Lang::Plain => {
            let extension = tab.engine.session(session).and_then(|s| s.adapter.extensions.first());
            extension.map_or(Lang::Plain, |e| Lang::from_path(&Path::new("source").with_extension(e)))
        }
        lang => lang,
    }
}

/// What SOURCE says for a frame that has no source at all.
fn no_source(p: &mut Painter<'_>, tab: &Tab, frame: &Frame, inner: Rect) {
    let index = tab.selected_frame_index();
    let text = format!("#{index} {} has no source", truncate(&frame.name, 60));
    p.text(inner.x + 2, inner.y + 1, &text, fg(DIM), inner.right());
    let why = "the adapter knows no file or text for it, as for code without debug information";
    p.text(inner.x + 2, inner.y + 2, why, fg(FAINT), inner.right());
}

/// Why SOURCE has no lines to show, and what to do about it, line by line.
fn unavailable(app: &App, shown: &Shown) -> Vec<Vec<Seg<'static>>> {
    let tab = app.tab();
    let line = |text: String, color| vec![seg(text, fg(color))];
    match shown {
        Shown::Adapter { session, reference, name } => {
            let session = tab.engine.session(*session);
            match session.and_then(|s| s.source(*reference)) {
                Some(SourceText::Failed(err)) => {
                    vec![line(format!("the adapter did not give {name}"), DIM), line(err.clone(), FAINT)]
                }
                None if !session.is_some_and(|s| s.is_alive()) => {
                    vec![line(format!("{name} was not read from the adapter before its session ended"), DIM)]
                }
                _ => vec![line(format!("reading {name} from the adapter…"), FAINT)],
            }
        }
        Shown::File(path) if !path.exists() => {
            let project = tab.engine.workspace();
            let root = remote_root(path, |rest| project.join(rest).is_file());
            let map = match root {
                Some(root) => format!("/config map {}=.", root.display()),
                None => "/config map ‹directory there›=‹directory here›".to_owned(),
            };
            let config =
                app.keymap.key_for(tab.focus().name(), "view config").map(|k| format!(" ({k})")).unwrap_or_default();
            vec![
                line(format!("not on this machine: {}", path.display()), DIM),
                line("The program runs elsewhere, in a container or on another host, where its".into(), FAINT),
                line(format!("files have other paths. Map them to this project in CONFIG{config}:"), FAINT),
                vec![seg("    ", fg(FAINT)), seg(map, bold(FG))],
            ]
        }
        Shown::File(path) => {
            let err = std::fs::read(path).err().map(|e| format!(": {e}")).unwrap_or_default();
            vec![line(format!("cannot read {}{err}", path.display()), DIM)]
        }
    }
}

/// Where the program's files are, for a path that is not here: the directory it starts in
/// when the rest of it is a file of the project (`is_file`, relative to the project). For
/// `/var/www/app/src/Order.php` and a project with `src/Order.php`, `/var/www/app`. The
/// longest rest that matches wins, as a file name alone may match by chance.
fn remote_root(path: &Path, is_file: impl Fn(&Path) -> bool) -> Option<&Path> {
    let roots: Vec<&Path> = path.ancestors().skip(1).collect();
    roots.into_iter().rev().find(|root| path.strip_prefix(root).is_ok_and(&is_file))
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
    } else if bp.condition.is_some() || bp.hit_condition.is_some() {
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

/// Values of the shown frame's variables next to the last line (up to its execution line)
/// where each appears.
fn inline_values(tab: &Tab, lines: &[String], top: u32, exec: Option<u32>) -> HashMap<u32, Vec<InlineValue>> {
    let mut out: HashMap<u32, Vec<InlineValue>> = HashMap::new();
    let (Some(snapshot), Some(exec)) = (tab.viewed(), exec) else { return out };
    let changes = tab.changes();
    let first = top.max(exec.saturating_sub(40)).max(1);
    let scopes = snapshot.scopes(tab.selected_frame_index()).unwrap_or_default();
    for scope in scopes.iter().filter(|s| s.default_open) {
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
    // The message, a gap, the list with a line above and a gap below, then two hints,
    // under a gap; a short pane lists fewer.
    let listed = configs.len().min(6).min(usize::from(inner.height.saturating_sub(7)));
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
            // A recent project's path shows its end, which tells it apart.
            let item = if tab.project.is_some() {
                item.into()
            } else {
                truncate_left(item, usize::from(inner.right().saturating_sub(x + 5)))
            };
            p.text(x + 4, y, &item, fg(FAINT), inner.right());
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
    use std::path::Path;

    use super::{mentions, remote_root, window_top};

    #[test]
    fn a_path_elsewhere_maps_to_the_project_by_the_longest_rest_it_has() {
        let project = |files: &'static [&'static str]| move |rest: &Path| files.iter().any(|f| Path::new(f) == rest);
        let path = Path::new("/var/www/app/src/Order.php");
        assert_eq!(remote_root(path, project(&["src/Order.php"])), Some(Path::new("/var/www/app")));
        // A file of the same name at the top of the project is a weaker match.
        assert_eq!(remote_root(path, project(&["Order.php", "src/Order.php"])), Some(Path::new("/var/www/app")));
        assert_eq!(remote_root(Path::new("/app/orders.php"), project(&["orders.php"])), Some(Path::new("/app")));
        assert_eq!(remote_root(path, project(&["lib/Order.php"])), None);
    }

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
