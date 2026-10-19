use ratatui::layout::Rect;

use super::paint::{Painter, Panel, Seg, seg, truncate, truncate_left};
use super::syntax::{Lang, highlight};
use super::theme::*;
use crate::app::App;
use crate::overlay::{Choice, Finder, Palette};
use crate::picker::Picker;

fn highlighted(text: &str, matched: &[u32], base: ratatui::style::Style) -> Vec<Seg<'static>> {
    text.chars()
        .enumerate()
        .map(|(i, c)| {
            let hit = matched.binary_search(&(i as u32)).is_ok();
            seg(c.to_string(), if hit { bold(AMBER) } else { base })
        })
        .collect()
}

/// [`highlighted`], cut from the left to `room` columns so that the end of a path shows.
fn highlighted_tail(text: &str, matched: &[u32], base: ratatui::style::Style, room: usize) -> Vec<Seg<'static>> {
    let mut segs = highlighted(text, matched, base);
    if super::paint::width(&segs) as usize <= room {
        return segs;
    }
    while !segs.is_empty() && super::paint::width(&segs) as usize + 1 > room {
        segs.remove(0);
    }
    segs.insert(0, seg("…", base));
    segs
}

pub fn palette(p: &mut Painter<'_>, app: &App, palette: &Palette, screen: Rect) {
    let body = Rect::new(0, 0, screen.width, screen.height - 2);
    p.dim(body);
    let width = screen.width.min(132);
    let list_rows = palette.items.len().clamp(1, 20) as u16;
    let height = (list_rows + 3).min(screen.height - 4);
    let rect = Rect::new(0, screen.height - 2 - height, width, height);
    p.fill(rect, BG);
    let sub = vec![seg("every key runs one of these", fg(DIM))];
    p.panel(
        rect,
        &Panel {
            title: "COMMANDS",
            sub,
            right: vec![seg("tab completes · ⏎ runs · esc closes", fg(FAINT))],
            keys: vec![],
            focus: true,
        },
    );

    let inner = super::paint::inner(rect);
    let side = if inner.width >= 112 { 36 } else { 0 };
    let list_right = inner.right() - side;
    let (x_cmd, x_arg, x_desc) = (inner.x + 2, inner.x + 22, inner.x + 46);
    let x_key = list_right.saturating_sub(9);
    p.text(x_cmd, inner.y, "COMMAND", bold(FAINT), list_right);
    p.text(x_arg, inner.y, "ARGUMENT", bold(FAINT), list_right);
    p.text(x_desc, inner.y, "DESCRIPTION", bold(FAINT), list_right);
    p.text(x_key, inner.y, "KEY", bold(FAINT), list_right);

    let rows = usize::from(inner.height.saturating_sub(1));
    let offset = palette.selected.saturating_sub(rows.saturating_sub(1));
    for (i, item) in palette.items.iter().enumerate().skip(offset).take(rows) {
        let y = inner.y + 1 + (i - offset) as u16;
        let selected = i == palette.selected;
        if selected {
            p.band(inner.x, y, list_right - inner.x, SEL2);
            p.text(inner.x, y, "›", bold(AMBER), list_right);
        }
        let mut name = vec![seg("/", fg(FAINT))];
        name.extend(highlighted(item.spec.name, &item.matched, if selected { bold(WHITE) } else { fg(FG) }));
        p.segs(x_cmd, y, &name, x_arg - 1);
        p.text(x_arg, y, item.spec.arg_hint(), fg(FAINT), x_desc - 1);
        p.text(x_desc, y, item.spec.help, fg(if selected { FG } else { DIM }), x_key.saturating_sub(2));
        match &item.key {
            Some(key) => p.text(x_key, y, key, bold(KEY), list_right),
            None => p.text(x_key, y, "—", fg(GHOST), list_right),
        };
    }
    if palette.items.is_empty() {
        p.text(x_cmd, inner.y + 1, "no command matches", italic(FAINT), list_right);
    }

    if side > 0 {
        let sx = list_right + 1;
        for y in inner.y..inner.bottom() {
            p.text(sx - 1, y, "│", fg(GHOST), inner.right());
        }
        p.text(sx + 1, inner.y, "RECENT", bold(FAINT), inner.right());
        for (i, line) in palette.recent.iter().take(usize::from(inner.height.saturating_sub(4))).enumerate() {
            p.text(sx + 1, inner.y + 1 + i as u16, line, fg(DIM), inner.right());
        }
        let config = "~/.config/tuitor/keys.toml";
        p.text(sx + 1, inner.bottom() - 2, "keys are aliases for commands:", fg(GHOST), inner.right());
        p.text(sx + 1, inner.bottom() - 1, config, fg(FAINT), inner.right());
    }
    let _ = app;
}

pub fn finder(p: &mut Painter<'_>, app: &App, finder: &Finder, screen: Rect) {
    p.dim(screen);
    let width = screen.width.saturating_sub(8).min(150);
    let height = screen.height.saturating_sub(6).min(30);
    let rect = Rect::new((screen.width - width) / 2, 3, width, height);
    p.fill(rect, BG);
    p.panel(
        rect,
        &Panel {
            title: "FIND",
            sub: vec![seg("files · breakpoints", fg(DIM))],
            right: vec![],
            keys: vec![],
            focus: true,
        },
    );
    let inner = super::paint::inner(rect);
    let list_w = inner.width * 46 / 100;
    let list_right = inner.x + list_w;

    let x = p.text(inner.x + 1, inner.y, "› ", bold(AMBER), list_right);
    let x = p.text(x, inner.y, &finder.input, bold(WHITE), list_right);
    p.text(x, inner.y, "█", fg(AMBER), list_right);
    let count = format!("{} matches · {} files", finder.len(), finder.total_files);
    p.segs_right(list_right - 1, inner.y, &[seg(count, fg(FAINT))], x + 2);
    p.text(inner.x, inner.y + 1, &"─".repeat(usize::from(list_w)), fg(GHOST), list_right);

    let mut y = inner.y + 2;
    let bottom = inner.bottom() - 2;
    let mut index = 0;
    let visible_files = usize::from(bottom.saturating_sub(y)).saturating_sub(if finder.bps.is_empty() {
        1
    } else {
        3 + finder.bps.len().min(6)
    });
    let file_offset = finder.selected.min(finder.files.len()).saturating_sub(visible_files.saturating_sub(1));
    if !finder.files.is_empty() {
        p.text(inner.x + 1, y, "FILES", bold(FAINT), list_right);
        y += 1;
        for hit in finder.files.iter().skip(file_offset).take(visible_files) {
            let i = index + file_offset;
            index += 1;
            let selected = i == finder.selected;
            if selected {
                p.band(inner.x, y, list_w, SEL2);
                p.text(inner.x + 1, y, "›", bold(AMBER), list_right);
            }
            p.segs(
                inner.x + 3,
                y,
                &highlighted(&hit.rel, &hit.matched, if selected { bold(WHITE) } else { fg(FG) }),
                list_right - 1,
            );
            y += 1;
        }
    }
    let first_bp = finder.files.len();
    if !finder.bps.is_empty() && y + 2 < bottom {
        y += 1;
        p.text(inner.x + 1, y, "BREAKPOINTS", bold(FAINT), list_right);
        y += 1;
        for (k, hit) in finder.bps.iter().enumerate() {
            if y >= bottom {
                break;
            }
            let selected = first_bp + k == finder.selected;
            if selected {
                p.band(inner.x, y, list_w, SEL2);
                p.text(inner.x + 1, y, "›", bold(AMBER), list_right);
            }
            p.text(inner.x + 3, y, "●", fg(RED_DIM), list_right);
            p.segs(
                inner.x + 5,
                y,
                &highlighted(&hit.label, &hit.matched, if selected { bold(WHITE) } else { fg(FG) }),
                list_right - 1,
            );
            y += 1;
        }
    }
    if finder.len() == 0 {
        p.text(inner.x + 3, inner.y + 2, "nothing matches · name:line jumps to a line", italic(FAINT), list_right);
    }

    // Preview of the selected file.
    for row in inner.y..inner.bottom() - 1 {
        p.text(list_right, row, "│", fg(GHOST), inner.right());
    }
    if let Some(choice) = finder.choice() {
        let (path, line) = match choice {
            Choice::File { path, line } => (path, line.unwrap_or(1)),
            Choice::Breakpoint(id) => match app.tab().engine.breakpoints().get(id) {
                Some(bp) => (bp.path.clone(), bp.line),
                None => return,
            },
        };
        preview(
            p,
            app,
            &path,
            line,
            Rect::new(list_right + 2, inner.y, inner.right() - list_right - 2, inner.height - 1),
        );
    }

    let actions = [("⏎", "open in CODE"), ("^b", "toggle breakpoint at :line"), ("↑/↓", "select"), ("esc", "close")];
    let mut x = inner.x + 1;
    for (key, label) in actions {
        x = p.text(x, inner.bottom() - 1, key, bold(KEY), inner.right());
        x = p.text(x, inner.bottom() - 1, &format!(" {label}   "), fg(FAINT), inner.right());
    }
}

pub fn picker(p: &mut Painter<'_>, app: &App, picker: &Picker, screen: Rect) {
    p.dim(screen);
    let width = screen.width.saturating_sub(8).min(110);
    let rows = picker.rows.len().max(1) as u16;
    let height = (rows + 6).clamp(10, screen.height.saturating_sub(6).min(26));
    let rect = Rect::new((screen.width - width) / 2, 3, width, height);
    p.fill(rect, BG);
    let what =
        if picker.typing_path() { "directories" } else { "opened recently · or type a path: ~/…  /…  ../…" };
    let target = if app.tab().project.is_none() { "opens in this tab" } else { "opens in a new tab" };
    p.panel(
        rect,
        &Panel {
            title: "OPEN PROJECT",
            sub: vec![seg(what, fg(DIM))],
            right: vec![seg(target, fg(FAINT))],
            keys: vec![],
            focus: true,
        },
    );
    let inner = super::paint::inner(rect);
    let x = p.text(inner.x + 1, inner.y, "› ", bold(AMBER), inner.right());
    // A long path shows its end, where the typing is.
    let input = truncate_left(&picker.input, usize::from(inner.right().saturating_sub(x + 2)));
    let x = p.text(x, inner.y, &input, bold(WHITE), inner.right());
    p.text(x, inner.y, "█", fg(AMBER), inner.right());
    p.text(inner.x, inner.y + 1, &"─".repeat(usize::from(inner.width)), fg(GHOST), inner.right());

    let top = inner.y + 2;
    let height = usize::from(inner.height.saturating_sub(4));
    let offset = picker.selected.saturating_sub(height.saturating_sub(1));
    let kinds_x = inner.right().saturating_sub(24);
    for (i, row) in picker.rows.iter().enumerate().skip(offset).take(height) {
        let y = top + (i - offset) as u16;
        let selected = i == picker.selected;
        if selected {
            p.band(inner.x, y, inner.width, SEL2);
            p.text(inner.x + 1, y, "›", bold(AMBER), inner.right());
        }
        let style = if selected { bold(WHITE) } else { fg(FG) };
        let room = usize::from(kinds_x.saturating_sub(inner.x + 4));
        let label = highlighted_tail(&row.candidate.label, &row.candidate.matched, style, room);
        p.segs(inner.x + 3, y, &label, kinds_x - 1);
        let kinds: Vec<&str> = row.kinds.iter().map(|k| k.name()).collect();
        let mut info = vec![seg(kinds.join(" · "), fg(DIM))];
        if row.open {
            info.push(seg(if kinds.is_empty() { "open" } else { "  open" }, italic(FAINT)));
        }
        p.segs_right(inner.right() - 1, y, &info, kinds_x);
    }
    if picker.rows.is_empty() {
        let hint = if picker.typing_path() {
            "no directory matches"
        } else if picker.input.is_empty() {
            "no project opened yet · type a path: ~/…  /…  ../…"
        } else {
            "no recent project matches · a path starts with ~ / or ."
        };
        p.text(inner.x + 3, top, hint, italic(FAINT), inner.right());
    }

    let actions = [("⏎", "open"), ("tab", "complete the path"), ("↑/↓", "select"), ("esc", "close")];
    let mut x = inner.x + 1;
    for (key, label) in actions {
        x = p.text(x, inner.bottom() - 1, key, bold(KEY), inner.right());
        x = p.text(x, inner.bottom() - 1, &format!(" {label}   "), fg(FAINT), inner.right());
    }
}

fn preview(p: &mut Painter<'_>, app: &App, path: &std::path::Path, line: u32, area: Rect) {
    let engine = &app.tab().engine;
    let rel = path.strip_prefix(engine.workspace()).unwrap_or(path);
    p.text(
        area.x,
        area.y,
        &truncate(&format!("{}:{line}", rel.display()), usize::from(area.width)),
        fg(DIM),
        area.right(),
    );
    let Some(file) = app.sources.borrow_mut().get(path) else { return };
    let lang = Lang::from_path(path);
    let height = u32::from(area.height.saturating_sub(1));
    let first = line.saturating_sub(height / 3).max(1);
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    for row in 0..height {
        let n = first + row;
        let Some(text) = file.lines.get((n - 1) as usize) else { break };
        let y = area.y + 1 + row as u16;
        if n == line {
            p.band(area.x, y, area.width, SEL2);
        }
        if engine.breakpoints().at(&canonical, n).is_some() {
            p.text(area.x, y, "●", bold(RED_DIM), area.right());
        }
        p.text(area.x + 1, y, &format!("{n:>4} "), fg(if n == line { FG } else { FAINT }), area.right());
        let mut x = area.x + 6;
        for (piece, style) in highlight(text, lang) {
            x = p.text(x, y, &piece, style, area.right());
        }
    }
}
