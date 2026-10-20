//! Drawing. Every frame is rendered from scratch from `App`; nothing here mutates
//! state except the scroll offsets and the animation flag kept in `Cell`s.

mod breakpoints;
mod compare;
mod config;
mod doctor;
mod flow;
mod history;
mod install;
mod locals;
mod logo;
mod logs;
mod outline;
mod overlays;
pub mod paint;
mod source;
mod stack;
pub mod syntax;
pub mod theme;
mod watch;

use std::ops::Range;
use std::time::Duration;

use chrono::Local;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use tuitor_core::Level;
use tuitor_core::session::SessionState;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Pane};
use crate::command::View;
use crate::overlay::Overlay;
use crate::tab::Tab;
use logo::Mark;
use paint::{Painter, Seg, seg};
use theme::*;

pub const MIN_WIDTH: u16 = 100;
pub const MIN_HEIGHT: u16 = 28;
/// Rows above the panes: the logo beside the views, the session and the context.
pub const HEADER: u16 = logo::MARK_ROWS;

pub fn draw(frame: &mut Frame, app: &App) {
    app.ui.animating.set(None);
    // Soon again, to show what a compose file maps once it is read.
    if app.compose.is_some() {
        app.ui.animate(Duration::from_millis(100));
    }
    let area = frame.area();
    let mut p = Painter { buf: frame.buffer_mut() };
    p.fill(area, BG);
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let text = format!("tuitor needs {MIN_WIDTH}×{MIN_HEIGHT}, the terminal is {}×{}", area.width, area.height);
        p.text(1, area.height / 2, &text, fg(DIM), area.right());
        return;
    }

    header(&mut p, app, Rect::new(0, 0, area.width, HEADER));
    let body = Rect::new(0, HEADER, area.width, area.height - HEADER - 2);
    for (pane, rect) in layout(app, body) {
        draw_pane(&mut p, app, pane, rect);
    }
    let tab = app.tab();
    if tab.ui.view == View::History && !tab.ui.zoom {
        history::value(&mut p, tab, history_value_area(body));
    }
    if tab.ui.view == View::Compare && !tab.ui.zoom {
        compare::detail(&mut p, app, compare_detail_area(body));
    }
    if tab.ui.view == View::Flow && !tab.ui.zoom {
        let below = flow_detail_area(app, body);
        let half = pct(below.width, 55);
        flow::detail(&mut p, app, Rect::new(below.x, below.y, half, below.height));
        flow::evidence(&mut p, app, Rect::new(below.x + half, below.y, below.width - half, below.height));
    }
    status(&mut p, app, Rect::new(0, area.height - 2, area.width, 1));
    hints(&mut p, app, Rect::new(0, area.height - 1, area.width, 1));

    match &app.ui.overlay {
        Some(Overlay::Palette(palette)) => overlays::palette(&mut p, app, palette, area),
        Some(Overlay::Finder(finder)) => overlays::finder(&mut p, app, finder, area),
        Some(Overlay::Picker(picker)) => overlays::picker(&mut p, app, picker, area),
        Some(Overlay::Doctor(view)) => doctor::draw(&mut p, app, view, area),
        Some(Overlay::Install) => {
            if let Some(installer) = &app.installer {
                install::draw(&mut p, app, installer, area);
            }
        }
        // On the status line, over SOURCE and its highlights.
        Some(Overlay::Search(_)) => {}
        None => {}
    }
}

fn draw_pane(p: &mut Painter<'_>, app: &App, pane: Pane, area: Rect) {
    let tab = app.tab();
    let focus = tab.focus() == pane && app.ui.overlay.is_none();
    match pane {
        Pane::Source => source::draw(p, app, area, focus),
        Pane::Locals => locals::draw(p, tab, area, focus),
        Pane::Stack => stack::draw(p, tab, area, focus),
        Pane::Watch => watch::draw(p, tab, area, focus),
        Pane::Breakpoints => breakpoints::draw(p, app, area, focus, tab.ui.view == View::Project || tab.ui.zoom),
        Pane::Output => logs::output(p, tab, area, focus),
        Pane::Events => logs::events(p, tab, area, focus),
        Pane::History => history::table(p, tab, area, focus),
        Pane::Config => config::draw(p, app, area, focus),
        Pane::Flow => flow::draw(p, app, area, focus),
        Pane::Outline => outline::draw(p, app, area, focus),
        Pane::Compare => compare::draw(p, app, area, focus),
    }
}

fn pct(total: u16, percent: u16) -> u16 {
    total * percent / 100
}

/// Pane rectangles for the current view (one when zoomed).
fn layout(app: &App, body: Rect) -> Vec<(Pane, Rect)> {
    let tab = app.tab();
    if tab.ui.zoom {
        return vec![(tab.focus(), body)];
    }
    let (x, y, w, h) = (body.x, body.y, body.width, body.height);
    match tab.ui.view {
        View::Overview => {
            let bottom_h = pct(h, 19).clamp(6, 10);
            let top_h = h - bottom_h;
            let lw = pct(w, 61).max(60).min(w - 36);
            let rw = w - lw;
            let source_h = pct(top_h, 56);
            let stack_h = pct(top_h, 24).clamp(5, 12);
            let watch_h = pct(top_h, 32).clamp(5, 14);
            vec![
                (Pane::Source, Rect::new(x, y, lw, source_h)),
                (Pane::Locals, Rect::new(x, y + source_h, lw, top_h - source_h)),
                (Pane::Stack, Rect::new(x + lw, y, rw, stack_h)),
                (Pane::Watch, Rect::new(x + lw, y + stack_h, rw, watch_h)),
                (Pane::Breakpoints, Rect::new(x + lw, y + stack_h + watch_h, rw, top_h - stack_h - watch_h)),
                (Pane::Output, Rect::new(x, y + top_h, lw, bottom_h)),
                (Pane::Events, Rect::new(x + lw, y + top_h, rw, bottom_h)),
            ]
        }
        View::Code => {
            // FLOW and OUTLINE on the left, SOURCE, then STACK and BREAKPOINTS.
            let side = (w / 5).clamp(22, 34);
            let right = pct(w, 28);
            let lw = w - side - right;
            let stack_h = pct(h, 55);
            let flow_h = pct(h, 45);
            vec![
                (Pane::Source, Rect::new(x + side, y, lw, h)),
                (Pane::Stack, Rect::new(x + side + lw, y, right, stack_h)),
                (Pane::Breakpoints, Rect::new(x + side + lw, y + stack_h, right, h - stack_h)),
                (Pane::Flow, Rect::new(x, y, side, flow_h)),
                (Pane::Outline, Rect::new(x, y + flow_h, side, h - flow_h)),
            ]
        }
        View::Flow => vec![(Pane::Flow, Rect::new(x, y, w, flow_height(app, h)))],
        View::History => {
            let table_h = pct(h, 48).max(8);
            let half = w / 2;
            vec![
                (Pane::History, Rect::new(x, y, w, table_h)),
                (Pane::Locals, Rect::new(x + half, y + table_h, w - half, h - table_h)),
            ]
        }
        View::Project => {
            let lw = pct(w, 55);
            vec![(Pane::Breakpoints, Rect::new(x, y, lw, h)), (Pane::Config, Rect::new(x + lw, y, w - lw, h))]
        }
        View::Compare => vec![(Pane::Compare, Rect::new(x, y, w, compare_height(h)))],
    }
}

/// COMPARE's hits take a little over half of the body, their values the rest.
fn compare_height(height: u16) -> u16 {
    pct(height, 55).max(8)
}

fn compare_detail_area(body: Rect) -> Rect {
    let top = compare_height(body.height);
    Rect::new(body.x, body.y + top, body.width, body.height - top)
}

/// FLOW gets a line per file, with its axis and legend, and leaves the rest of the
/// body to the selected file and the evidence.
fn flow_height(app: &App, height: u16) -> u16 {
    let tab = app.tab();
    let lines = tab.flow().map_or(0, |view| crate::flow::items(&view.flow, tab.ui.flow_vendor).len());
    let wanted = u16::try_from(lines).unwrap_or(u16::MAX).saturating_add(6);
    wanted.clamp(height.min(12), height * 3 / 5)
}

fn flow_detail_area(app: &App, body: Rect) -> Rect {
    let top = flow_height(app, body.height);
    Rect::new(body.x, body.y + top, body.width, body.height - top)
}

fn history_value_area(body: Rect) -> Rect {
    let table_h = pct(body.height, 48).max(8);
    Rect::new(body.x, body.y + table_h, body.width / 2, body.height - table_h)
}

/// Where the logo starts, after a margin column.
const LOGO_X: u16 = 1;
/// Columns between the logo and the rows beside it.
const LOGO_GAP: u16 = 2;
/// Columns the session keeps beside the tabs before they take the rest of the row.
const SESSION_ROOM: u16 = 24;
/// Shortest a project name gets before tabs are left out.
const MIN_NAME: usize = 3;

/// The logo on the left, centred on three rows. Beside it the views and the clock; the
/// active session and the project tabs, each with the state of its own active
/// session; and the context line.
fn header(p: &mut Painter<'_>, app: &App, area: Rect) {
    p.fill(area, BAR);
    let tabs = tab_labels(app);
    let sizes: Vec<(usize, bool)> = tabs.iter().map(|t| (t.name.width(), t.glyph.is_some())).collect();
    let session = session(app);
    let reserve = paint::width(&session).min(SESSION_ROOM);
    let mark = header_mark(area.width, tabs_width(&sizes, usize::MAX), reserve);
    logo::mark(p, LOGO_X, area.y, mark, logo_clock(app));

    let left = rows_x(mark);
    let row = |y| Rect::new(left, area.y + y, area.right().saturating_sub(left), 1);
    views(p, app, row(0));
    let fit = fit_tabs(&sizes, app.tabs.active_index(), tabs_room(area.width, mark, reserve));
    let tabs_x = project_tabs(p, app, &tabs, &sizes, &fit, row(1));
    p.segs(left + 1, area.y + 1, &session, tabs_x.saturating_sub(2));
    context(p, app, row(2));
}

/// Where the rows beside the logo start: a padding column before their text.
fn rows_x(mark: Mark) -> u16 {
    LOGO_X + mark.width() + LOGO_GAP
}

/// Columns for the tabs beside `mark` on a row `width` wide, after the padding,
/// `session` columns of the session, a gap and the margin.
fn tabs_room(width: u16, mark: Mark, session: u16) -> u16 {
    width.saturating_sub(rows_x(mark) + 1 + session + 2 + 1)
}

/// The logo gives way first: it keeps only its "t" before a tab name is shortened.
fn header_mark(width: u16, tabs: u16, session: u16) -> Mark {
    if tabs <= tabs_room(width, Mark::Word, session) { Mark::Word } else { Mark::T }
}

/// The views on the left of the row, the clock on its right.
fn views(p: &mut Painter<'_>, app: &App, area: Rect) {
    let (mut x, y) = (area.x, area.y);
    for view in View::ALL {
        let label = format!(" {} ", view.name().to_uppercase());
        if view == app.tab().ui.view {
            let start = x;
            x = p.text(x, y, &label, bold(WHITE), area.right());
            p.band(start, y, x - start, SEL2);
        } else {
            x = p.text(x, y, &label, fg(FAINT), area.right());
        }
    }
    let clock = Local::now().format("%H:%M:%S ").to_string();
    p.segs_right(area.right().saturating_sub(1), y, &[seg(clock, fg(DIM))], x + 2);
}

/// The project tabs, right-aligned in `area` with a column to spare. Returns where they start.
fn project_tabs(
    p: &mut Painter<'_>,
    app: &App,
    tabs: &[TabLabel],
    sizes: &[(usize, bool)],
    fit: &TabFit,
    area: Rect,
) -> u16 {
    let y = area.y;
    let hidden = tabs.len() - fit.shown.len();
    let more = if hidden > 0 { format!(" +{hidden} ") } else { String::new() };
    let width = tabs_width(&sizes[fit.shown.clone()], fit.name) + more.width() as u16;
    let start = area.right().saturating_sub(1 + width).max(area.x);
    let mut x = start;
    for (index, tab) in tabs.iter().enumerate().take(fit.shown.end).skip(fit.shown.start) {
        let first = x;
        let active = index == app.tabs.active_index();
        let name = format!(" {}", paint::truncate_middle(&tab.name, fit.name));
        x = p.text(x, y, &name, if active { bold(WHITE) } else { fg(FAINT) }, area.right());
        if let Some((glyph, color)) = tab.glyph {
            x = p.text(x, y, &format!(" {glyph}"), bold(color), area.right());
        }
        x = p.text(x, y, " ", fg(FAINT), area.right());
        if active {
            p.band(first, y, x - first, SEL2);
        }
    }
    p.text(x, y, &more, fg(FAINT), area.right());
    start
}

/// The tab's active session and its state, and how many more are alive.
fn session(app: &App) -> Vec<Seg<'static>> {
    let tab = app.tab();
    let Some(session) = tab.active() else {
        return vec![seg(if tab.project.is_some() { "no session" } else { "no project" }, fg(DIM))];
    };
    let (glyph, color) = state_glyph(session.state, true);
    let state = match session.state {
        SessionState::Stopped => "stopped",
        SessionState::Running => "running",
        SessionState::Starting => "starting",
        SessionState::Exited => "exited",
    };
    // A child session is named after what it runs (`orders.js [4242]`, a page's title), its
    // parent after the configuration.
    let launch = tab.engine.session(tab.engine.root(session.id)).unwrap_or(session);
    let mut segs =
        vec![seg(session.tag.clone(), fg(tag(&session.tag))), seg(format!(" {}", launch.config.name), fg(DIM))];
    if launch.id != session.id {
        segs.push(seg(format!(" › {}", session.config.name), fg(FAINT)));
    }
    segs.push(seg(format!("  {glyph}"), bold(color)));
    segs.push(seg(format!(" {state}"), fg(DIM)));
    let live = tab.live_sessions();
    if live > 1 {
        segs.push(seg("  ·  ", fg(GHOST)));
        segs.push(seg(format!("{live} live sessions"), fg(FAINT)));
        if let Some(key) = app.keymap.key_for(tab.focus().name(), "session next") {
            segs.push(seg(format!("  {key} next"), fg(GHOST)));
        }
    }
    segs
}

struct TabLabel {
    name: String,
    glyph: Option<(&'static str, Color)>,
}

/// Each tab's project name, with its parent where two projects share a name, and the
/// state of its active session.
fn tab_labels(app: &App) -> Vec<TabLabel> {
    let names: Vec<String> = app.tabs.iter().map(|(_, tab)| tab.name()).collect();
    app.tabs
        .iter()
        .zip(&names)
        .map(|((id, tab), name)| {
            let shared = names.iter().filter(|n| *n == name).count() > 1;
            let name = match (&tab.project, shared) {
                (Some(project), true) => parent_and_name(&project.dir),
                _ => name.clone(),
            };
            let glyph = tab.active().map(|session| state_glyph(session.state, id == app.tabs.active_id()));
            TabLabel { name, glyph }
        })
        .collect()
}

/// A session's state as a glyph. Amber marks where a program is stopped: bright in
/// the tab on screen, dim in one that waits behind it.
fn state_glyph(state: SessionState, shown: bool) -> (&'static str, Color) {
    match state {
        SessionState::Stopped => ("■", if shown { AMBER } else { AMBER_DIM }),
        SessionState::Running => ("▶", FAINT),
        SessionState::Starting => ("⧗", FAINT),
        SessionState::Exited => ("✕", GHOST),
    }
}

fn parent_and_name(dir: &std::path::Path) -> String {
    let mut parts: Vec<String> = dir.iter().rev().take(2).map(|p| p.to_string_lossy().into_owned()).collect();
    parts.reverse();
    parts.join("/")
}

/// How the tabs fit in `room` columns: names shrink first, then tabs far from the
/// active one are left out (and counted).
#[derive(Debug, PartialEq, Eq)]
struct TabFit {
    /// Longest a name may be.
    name: usize,
    shown: Range<usize>,
}

/// `(name width, has a glyph)` per tab.
fn fit_tabs(tabs: &[(usize, bool)], active: usize, room: u16) -> TabFit {
    let longest = tabs.iter().map(|(name, _)| *name).max().unwrap_or(0);
    for name in (MIN_NAME.min(longest)..=longest).rev() {
        if tabs_width(tabs, name) <= room {
            return TabFit { name, shown: 0..tabs.len() };
        }
    }
    // Room for " +N " after the tabs.
    let room = room.saturating_sub(3 + tabs.len().to_string().len() as u16);
    let (mut start, mut end) = (active, active + 1);
    loop {
        let grow_right = end < tabs.len() && (start == 0 || end - active <= active - start);
        let (s, e) = if grow_right {
            (start, end + 1)
        } else if start > 0 {
            (start - 1, end)
        } else {
            break;
        };
        if tabs_width(&tabs[s..e], MIN_NAME) > room {
            break;
        }
        (start, end) = (s, e);
    }
    TabFit { name: MIN_NAME, shown: start..end }
}

/// " name ■ " per tab, names cut to `name` columns.
fn tabs_width(tabs: &[(usize, bool)], name: usize) -> u16 {
    tabs.iter().map(|(width, glyph)| (width.min(&name) + 2 + if *glyph { 2 } else { 0 }) as u16).sum()
}

/// Time for the header's logo, or `None` to rest it. Its colours roll while nothing waits
/// to be read: with no session, or while the active one starts, runs or has ended,
/// so it also quietly shows a running program. It rests while that is stopped or
/// something is open over it.
fn logo_clock(app: &App) -> Option<Duration> {
    let stopped = app.tab().active().is_some_and(|s| s.state == SessionState::Stopped);
    let clock = app.ui.clock.filter(|_| !stopped && app.ui.overlay.is_none());
    if clock.is_some() {
        app.ui.animate(logo::FRAME);
    }
    clock
}

/// What the session is at: where it stopped, the stop on view in history, the
/// file being browsed. `area` starts with a padding column.
fn context(p: &mut Painter<'_>, app: &App, area: Rect) {
    let tab = app.tab();
    let (x, y) = (area.x + 1, area.y);
    let sep = || seg("  ·  ", fg(GHOST));
    if tab.in_history()
        && let (Some(session), Some(snapshot)) = (tab.active(), tab.viewed())
    {
        p.fill(area, AMBER);
        let ago = (Local::now() - snapshot.time).num_milliseconds() as f64 / 1000.0;
        let mut segs = vec![
            seg(" ◂ HISTORY ", bold(theme::BG)),
            seg(format!("  stop #{} of {}", snapshot.number, session.history.len()), fg(theme::BG)),
            seg(format!("   ·   {}  (−{ago:.1}s)", snapshot.time.format("%H:%M:%S%.3f")), fg(theme::BG)),
            seg(
                if snapshot.silent {
                    "   ·   passed without stopping, its set not active; recorded"
                } else {
                    "   ·   recorded values, nothing is re-read"
                },
                fg(theme::BG),
            ),
            seg("   ·   ] newer   } live ▸", bold(theme::BG)),
        ];
        // Where it is narrow the note goes first, then the time; the keys stay.
        for part in [3, 2] {
            if paint::width(&segs) > area.width {
                segs.remove(part);
            }
        }
        p.segs(area.x, y, &segs, area.right());
        return;
    }

    let right = area.right().saturating_sub(1);
    let Some(session) = tab.active() else {
        p.segs(x, y, &idle(tab), right);
        return;
    };

    let mut segs = Vec::new();
    match session.state {
        SessionState::Starting => segs.push(seg("starting the adapter…", fg(DIM))),
        SessionState::Running if session.threads.is_empty() => segs.push(seg("waiting for the debuggee", fg(DIM))),
        SessionState::Running => {
            let threads = session.threads.len();
            segs.push(seg(format!("{threads} thread{}", if threads == 1 { "" } else { "s" }), fg(DIM)));
        }
        SessionState::Exited => {
            if let Some(code) = session.exit_code {
                segs.push(seg(format!("exit code {code}"), fg(DIM)));
            }
        }
        SessionState::Stopped => {
            if let Some(snapshot) = tab.viewed() {
                let reason = match snapshot.hit_breakpoints.first() {
                    Some(id) => format!("breakpoint #{id}"),
                    None => snapshot.reason.clone(),
                };
                // The breakpoint's set, by its colour, beside the amber of execution.
                if let Some(set) = snapshot.set {
                    segs.push(seg("◉ ", bold(set_color(set, true))));
                }
                segs.push(seg(reason, bold(AMBER)));
                if let Some(top) = snapshot.top() {
                    segs.push(seg(" at ", fg(DIM)));
                    segs.push(seg(top.location(), fg(FG)));
                }
                segs.push(seg(format!(" · stop #{}", snapshot.number), fg(FAINT)));
                if let Some(description) = snapshot.description.as_deref().filter(|d| *d != snapshot.reason) {
                    segs.push(sep());
                    segs.push(seg(description.to_owned(), fg(DIM)));
                }
            } else {
                segs.push(seg("reading state…", fg(DIM)));
            }
        }
    }
    if let Some(path) = &tab.ui.source.browse {
        let rel = path.strip_prefix(tab.engine.workspace()).unwrap_or(path);
        if !segs.is_empty() {
            segs.push(sep());
        }
        segs.push(seg("browsing ", fg(FAINT)));
        segs.push(seg(rel.display().to_string(), fg(FG)));
        segs.push(seg("  . follows execution", fg(GHOST)));
    }
    p.segs(x, y, &segs, right);
}

/// The context line of a tab with no session: how to get one.
fn idle(tab: &Tab) -> Vec<Seg<'static>> {
    let sep = || seg("  ·  ", fg(GHOST));
    let Some(project) = &tab.project else {
        return vec![
            seg("O", bold(KEY)),
            seg(" opens one   ", fg(FAINT)),
            seg("/", bold(KEY)),
            seg(" command line", fg(FAINT)),
        ];
    };
    let configs = project.configurations.len();
    let start = match tab.selected_configuration() {
        Some(config) => vec![seg("S", bold(KEY)), seg(format!(" starts {}   ", config.name()), fg(FAINT))],
        None => vec![seg("g p", bold(KEY)), seg(" adds one   ", fg(FAINT))],
    };
    let mut segs =
        vec![seg(format!("{configs} launch configuration{}", if configs == 1 { "" } else { "s" }), fg(FAINT)), sep()];
    segs.extend(start);
    segs.extend([
        seg("f", bold(KEY)),
        seg(" find a file   ", fg(FAINT)),
        seg("b", bold(KEY)),
        seg(" breakpoints work before starting", fg(FAINT)),
    ]);
    segs
}

fn status(p: &mut Painter<'_>, app: &App, area: Rect) {
    p.fill(area, BAR);
    let y = area.y;
    if let Some(Overlay::Palette(palette)) = &app.ui.overlay {
        let x = p.text(1, y, &format!("/{}", palette.input.trim_start_matches('/')), bold(WHITE), area.right());
        p.text(x, y, "█", fg(AMBER), area.right());
        return;
    }
    if let Some(Overlay::Search(prompt)) = &app.ui.overlay {
        // As vim shows a search being typed.
        let x = p.text(1, y, "/", bold(AMBER), area.right());
        let x = p.text(x, y, &prompt.input, bold(WHITE), area.right());
        let x = p.text(x, y, "█", fg(AMBER), area.right());
        let said = app.ui.status.as_ref().filter(|_| !prompt.input.is_empty());
        if let Some(status) = said {
            let color = if status.level == Level::Info { DIM } else { YELLOW_DIM };
            p.text(x + 2, y, &status.message, fg(color), area.right());
        } else if let Some(previous) = prompt.previous.as_ref().filter(|_| prompt.input.is_empty()) {
            p.text(x + 2, y, &format!("⏎ searches `{}` again", previous.query), fg(GHOST), area.right());
        }
        return;
    }
    let Some(status) = &app.ui.status else {
        p.text(1, y, "/ opens the command line · ? lists every command and key", fg(GHOST), area.right());
        return;
    };
    let mut segs = Vec::new();
    if let Some(key) = &status.key {
        segs.push(seg(format!("‹{key}› "), fg(FAINT)));
    }
    if let Some(command) = &status.command {
        segs.push(seg(command.clone(), bold(FG)));
        if !status.message.is_empty() {
            segs.push(seg(" → ", fg(FAINT)));
        }
    }
    let color = match status.level {
        Level::Info => DIM,
        Level::Warn => YELLOW_DIM,
        Level::Error => RED,
    };
    segs.push(seg(status.message.clone(), fg(color)));
    p.segs(1, y, &segs, area.right());
}

/// The most useful bindings for the focused pane, read from the keymap so they
/// always show what the keys really do.
fn hints(p: &mut Painter<'_>, app: &App, area: Rect) {
    if let Some(Overlay::Search(_)) = &app.ui.overlay {
        let keys = [("⏎", "keep"), ("esc", "back"), ("^n ↓", "next"), ("^p ↑", "previous")];
        let mut x = 1;
        for (key, label) in keys {
            x = p.text(x, area.y, key, bold(KEY), area.right());
            x = p.text(x, area.y, &format!(" {label}   "), fg(FAINT), area.right());
        }
        return;
    }
    let pane = app.focus();
    let specific: &[(&str, &str)] = match pane {
        _ if app.tab().project.is_none() => &[("tab open", "open a project")],
        Pane::Source if app.tab().ui.search.is_some() => &[
            ("search next", "next match"),
            ("search prev", "previous"),
            ("close", "clear"),
            ("bp toggle", "breakpoint"),
            ("search", "search"),
        ],
        Pane::Source => &[
            ("bp toggle", "breakpoint"),
            ("find", "find"),
            ("search", "search"),
            ("follow", "follow"),
            ("edit", "editor"),
        ],
        Pane::Locals => &[("open", "expand"), ("watch add", "watch"), ("copy", "copy")],
        Pane::Stack => &[("open", "show frame"), ("vendor", "library frames")],
        Pane::Watch => &[("watch add", "add"), ("watch delete", "delete")],
        Pane::Breakpoints => &[
            ("bp scope", "scope"),
            ("bp condition", "condition"),
            ("bp log", "logpoint"),
            ("bp hits", "hits"),
            ("set move", "set"),
            ("bp enable", "on/off"),
            ("bp delete", "delete"),
            ("set delete", "delete set"),
            ("edit", "editor"),
        ],
        Pane::Output => &[("open", "go to its file:line"), ("copy", "copy"), ("clear output", "clear")],
        Pane::Config => &[
            ("config start", "start"),
            ("open", "edit"),
            ("config add", "add"),
            ("config map", "map"),
            ("config mappings-from-compose", "compose"),
            ("config unset", "unset"),
            ("config delete", "delete"),
            ("install", "install"),
            ("doctor", "doctor"),
        ],
        Pane::Events => &[("open", "go to its file:line")],
        Pane::History => &[("open", "open stop"), ("right", "column")],
        Pane::Flow => &[("open", "open in code"), ("bp toggle", "bp on entry"), ("vendor", "library files")],
        Pane::Compare => &[
            ("open", "code or history"),
            ("right", "column"),
            ("compare older", "older run"),
            ("compare newer", "newer run"),
        ],
        Pane::Outline => &[("open", "jump"), ("bp toggle", "bp on entry")],
    };
    let common: &[(&str, &str)] = &[
        ("set next", "active set"),
        ("continue", "continue"),
        ("next", "over"),
        ("step in", "into"),
        ("step out", "out"),
        ("history prev", "back"),
        ("history next", "fwd"),
        ("pane next", "pane"),
        ("tab next", "next tab"),
        ("zoom", "zoom"),
        ("palette", "command"),
        ("help", "keys"),
    ];
    let mut x = 1;
    let one_tab = app.tabs.len() == 1;
    let one_set = app.tab().engine.breakpoints().sets().len() == 1;
    for (command, label) in specific.iter().chain(common) {
        if (one_tab && *command == "tab next") || (one_set && *command == "set next") {
            continue;
        }
        let Some(key) = app.keymap.key_for(pane.name(), command) else { continue };
        let width = paint::width(&[seg(key.clone(), fg(KEY)), seg(format!(" {label}   "), fg(FAINT))]);
        if x + width > area.right() {
            break;
        }
        x = p.text(x, area.y, &key, bold(KEY), area.right());
        x = p.text(x, area.y, &format!(" {label}   "), fg(FAINT), area.right());
    }
    if !app.ui.pending.is_empty() {
        let pending = crate::keymap::label(&app.ui.pending);
        p.segs_right(area.right() - 1, area.y, &[seg(format!("{pending} …"), bold(AMBER))], x);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_keep_full_names_while_they_fit() {
        let tabs = [(3, true), (3, false)];
        // " api ■ " and " web ".
        assert_eq!(tabs_width(&tabs, usize::MAX), 12);
        assert_eq!(fit_tabs(&tabs, 0, 12), TabFit { name: 3, shown: 0..2 });
    }

    #[test]
    fn names_shrink_before_tabs_are_left_out() {
        let tabs = [(12, true), (9, true), (4, false)];
        let full = tabs_width(&tabs, usize::MAX);
        let fit = fit_tabs(&tabs, 1, full - 5);
        assert_eq!(fit.shown, 0..3);
        assert!(fit.name < 12 && tabs_width(&tabs, fit.name) <= full - 5);

        let shortest = tabs_width(&tabs, MIN_NAME);
        assert_eq!(fit_tabs(&tabs, 1, shortest), TabFit { name: MIN_NAME, shown: 0..3 });
        assert!(fit_tabs(&tabs, 1, shortest - 1).shown.len() < 3);
    }

    #[test]
    fn tabs_far_from_the_active_one_give_way_last() {
        let tabs = [(8, true); 9];
        let fit = fit_tabs(&tabs, 6, 40);
        assert_eq!(fit.name, MIN_NAME);
        assert!(fit.shown.contains(&6) && fit.shown.len() < 9);
        // With " +N " after them.
        assert!(tabs_width(&tabs[fit.shown.clone()], MIN_NAME) + 4 <= 40);
    }

    #[test]
    fn header_logo_keeps_its_t_before_the_tabs_shrink() {
        // Two tabs and a session at the narrowest screen: the whole wordmark fits.
        assert_eq!(header_mark(MIN_WIDTH, 20, SESSION_ROOM), Mark::Word);
        // More tabs than fit beside it: the logo shrinks to its "t", and they fit beside that.
        let tabs = tabs_room(MIN_WIDTH, Mark::Word, SESSION_ROOM) + 1;
        assert_eq!(header_mark(MIN_WIDTH, tabs, SESSION_ROOM), Mark::T);
        assert!(tabs <= tabs_room(MIN_WIDTH, Mark::T, SESSION_ROOM));
        assert_eq!(header_mark(160, tabs, SESSION_ROOM), Mark::Word);
        // The views and the clock always fit beside the whole wordmark, with a margin.
        let views: u16 = View::ALL.iter().map(|view| view.name().len() as u16 + 2).sum();
        let clock = "10:42:07 ".len() as u16;
        assert!(rows_x(Mark::Word) + views + 2 + clock < MIN_WIDTH);
    }
}
