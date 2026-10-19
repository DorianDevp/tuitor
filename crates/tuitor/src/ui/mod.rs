//! Drawing. Every frame is rendered from scratch from `App`; nothing here mutates
//! state except the scroll offsets and the animation flag kept in `Cell`s.

mod breakpoints;
mod config;
mod history;
mod locals;
mod logo;
mod logs;
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
use paint::{Painter, Seg, seg};
use theme::*;

pub const MIN_WIDTH: u16 = 100;
pub const MIN_HEIGHT: u16 = 28;

pub fn draw(frame: &mut Frame, app: &App) {
    app.ui.animating.set(None);
    let area = frame.area();
    let mut p = Painter { buf: frame.buffer_mut() };
    p.fill(area, BG);
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let text = format!("tuitor needs {MIN_WIDTH}×{MIN_HEIGHT}, the terminal is {}×{}", area.width, area.height);
        p.text(1, area.height / 2, &text, fg(DIM), area.right());
        return;
    }

    topbar(&mut p, app, Rect::new(0, 0, area.width, 1));
    context(&mut p, app, Rect::new(0, 1, area.width, 1));
    let body = Rect::new(0, 2, area.width, area.height - 4);
    for (pane, rect) in layout(app, body) {
        draw_pane(&mut p, app, pane, rect);
    }
    let tab = app.tab();
    if tab.ui.view == View::History && !tab.ui.zoom {
        history::value(&mut p, tab, history_value_area(body));
    }
    status(&mut p, app, Rect::new(0, area.height - 2, area.width, 1));
    hints(&mut p, app, Rect::new(0, area.height - 1, area.width, 1));

    match &app.ui.overlay {
        Some(Overlay::Palette(palette)) => overlays::palette(&mut p, app, palette, area),
        Some(Overlay::Finder(finder)) => overlays::finder(&mut p, app, finder, area),
        Some(Overlay::Picker(picker)) => overlays::picker(&mut p, app, picker, area),
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
            let lw = pct(w, 68);
            let stack_h = pct(h, 55);
            vec![
                (Pane::Source, Rect::new(x, y, lw, h)),
                (Pane::Stack, Rect::new(x + lw, y, w - lw, stack_h)),
                (Pane::Breakpoints, Rect::new(x + lw, y + stack_h, w - lw, h - stack_h)),
            ]
        }
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
    }
}

fn history_value_area(body: Rect) -> Rect {
    let table_h = pct(body.height, 48).max(8);
    Rect::new(body.x, body.y + table_h, body.width / 2, body.height - table_h)
}

/// The clock and the bar before it: "│ 10:42:07 ".
const CLOCK: u16 = 11;
/// Shortest a project name gets before tabs are left out.
const MIN_NAME: usize = 3;

/// Logo and views on the left; on the right the project tabs, each with the state of
/// its active session, and the clock.
fn topbar(p: &mut Painter<'_>, app: &App, area: Rect) {
    p.fill(area, BAR);
    let y = area.y;
    let views: u16 = View::ALL.iter().map(|view| view.name().len() as u16 + 2).sum();
    // The margin, the waves, " │ " before the views and the gap after them.
    let room = area.width.saturating_sub(1 + logo::WAVES + 3 + views + 2);
    let tabs = tab_labels(app);
    let sizes: Vec<(usize, bool)> = tabs.iter().map(|t| (t.name.width(), t.glyph.is_some())).collect();
    let fit = fit_tabs(&sizes, app.tabs.active_index(), room);
    // The logo gives way first: it drops its name before a tab name is shortened.
    let logo_room = logo::WAVES + room.saturating_sub(tabs_width(&sizes, usize::MAX) + CLOCK);

    let mut x = logo::bar(p, 1, y, logo_room, bar_clock(app));
    x = p.text(x + 1, y, "│", fg(GHOST), area.right()) + 1;
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

    let hidden = tabs.len() - fit.shown.len();
    let more = if hidden > 0 { format!(" +{hidden} ") } else { String::new() };
    let width =
        tabs_width(&sizes[fit.shown.clone()], fit.name) + more.width() as u16 + if fit.clock { CLOCK } else { 1 };
    let mut x = area.right().saturating_sub(width).max(x + 2);
    for (index, tab) in tabs.iter().enumerate().take(fit.shown.end).skip(fit.shown.start) {
        let start = x;
        let active = index == app.tabs.active_index();
        let name = format!(" {}", paint::truncate_middle(&tab.name, fit.name));
        x = p.text(x, y, &name, if active { bold(WHITE) } else { fg(FAINT) }, area.right());
        if let Some((glyph, color)) = tab.glyph {
            x = p.text(x, y, &format!(" {glyph}"), bold(color), area.right());
        }
        x = p.text(x, y, " ", fg(FAINT), area.right());
        if active {
            p.band(start, y, x - start, SEL2);
        }
    }
    x = p.text(x, y, &more, fg(FAINT), area.right());
    if fit.clock {
        x = p.text(x, y, "│ ", fg(GHOST), area.right());
        p.text(x, y, &Local::now().format("%H:%M:%S ").to_string(), fg(DIM), area.right());
    }
}

struct TabLabel {
    name: String,
    glyph: Option<(&'static str, Color)>,
}

/// Each tab's project name, with its parent where two projects share a name, and the
/// state of its active session. Amber marks where a program is stopped: bright in the
/// tab on screen, dim in one that waits behind it.
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
            let active = id == app.tabs.active_id();
            let glyph = tab.active().map(|session| match session.state {
                SessionState::Stopped => ("■", if active { AMBER } else { AMBER_DIM }),
                SessionState::Running => ("▶", FAINT),
                SessionState::Starting => ("⧗", FAINT),
                SessionState::Exited => ("✕", GHOST),
            });
            TabLabel { name, glyph }
        })
        .collect()
}

fn parent_and_name(dir: &std::path::Path) -> String {
    let mut parts: Vec<String> = dir.iter().rev().take(2).map(|p| p.to_string_lossy().into_owned()).collect();
    parts.reverse();
    parts.join("/")
}

/// How the tabs fit in `room` columns: names shrink first, then the clock goes, then
/// tabs far from the active one are left out (and counted).
#[derive(Debug, PartialEq, Eq)]
struct TabFit {
    /// Longest a name may be.
    name: usize,
    clock: bool,
    shown: Range<usize>,
}

/// `(name width, has a glyph)` per tab.
fn fit_tabs(tabs: &[(usize, bool)], active: usize, room: u16) -> TabFit {
    let longest = tabs.iter().map(|(name, _)| *name).max().unwrap_or(0);
    for clock in [true, false] {
        let room = room.saturating_sub(if clock { CLOCK } else { 1 });
        for name in (MIN_NAME.min(longest)..=longest).rev() {
            if tabs_width(tabs, name) <= room {
                return TabFit { name, clock, shown: 0..tabs.len() };
            }
        }
    }
    // Room for " +N " after the tabs and the margin.
    let room = room.saturating_sub(1 + 3 + tabs.len().to_string().len() as u16);
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
    TabFit { name: MIN_NAME, clock: false, shown: start..end }
}

/// " name ■ " per tab, names cut to `name` columns.
fn tabs_width(tabs: &[(usize, bool)], name: usize) -> u16 {
    tabs.iter().map(|(width, glyph)| (width.min(&name) + 2 + if *glyph { 2 } else { 0 }) as u16).sum()
}

/// Time for the top bar's waves, or `None` to rest them. They move while nothing
/// waits to be read: with no session, or while the active one starts, runs or has
/// ended, so they also quietly show a running program. They rest while it is
/// stopped or something is open over them.
fn bar_clock(app: &App) -> Option<Duration> {
    let stopped = app.tab().active().is_some_and(|s| s.state == SessionState::Stopped);
    let clock = app.ui.clock.filter(|_| !stopped && app.ui.overlay.is_none());
    if let Some(t) = clock {
        app.ui.animate(logo::next_wave(t));
    }
    clock
}

fn context(p: &mut Painter<'_>, app: &App, area: Rect) {
    let tab = app.tab();
    let y = area.y;
    let sep = || seg("  ·  ", fg(GHOST));
    if tab.in_history()
        && let (Some(session), Some(snapshot)) = (tab.active(), tab.viewed())
    {
        p.fill(area, AMBER);
        let ago = (Local::now() - snapshot.time).num_milliseconds() as f64 / 1000.0;
        let segs = [
            seg(" ◂ HISTORY ", bold(theme::BG)),
            seg(format!("  stop #{} of {}", snapshot.number, session.history.len()), fg(theme::BG)),
            seg(format!("   ·   {}  (−{ago:.1}s)", snapshot.time.format("%H:%M:%S%.3f")), fg(theme::BG)),
            seg("   ·   recorded values, nothing is re-read", fg(theme::BG)),
            seg("   ·   ] newer   } live ▸", bold(theme::BG)),
        ];
        p.segs(1, y, &segs, area.right());
        return;
    }

    let Some(session) = tab.active() else {
        p.segs(1, y, &idle(tab), area.right());
        return;
    };

    let mut segs =
        vec![seg(session.tag.clone(), fg(tag(&session.tag))), seg(format!(" {}", session.config.name), fg(DIM)), sep()];
    match session.state {
        SessionState::Starting => segs.push(seg("starting adapter…", fg(DIM))),
        SessionState::Running if session.threads.is_empty() => {
            segs.push(seg("running · waiting for the debuggee", fg(DIM)))
        }
        SessionState::Running => segs.push(seg(format!("running · {} threads", session.threads.len()), fg(DIM))),
        SessionState::Exited => {
            let code = session.exit_code.map(|c| format!(" with code {c}")).unwrap_or_default();
            segs.push(seg(format!("exited{code}"), fg(DIM)));
        }
        SessionState::Stopped => {
            if let Some(snapshot) = tab.viewed() {
                let reason = match snapshot.hit_breakpoints.first() {
                    Some(id) => format!("breakpoint #{id}"),
                    None => snapshot.reason.clone(),
                };
                segs.push(seg("stopped ", fg(DIM)));
                segs.push(seg(reason, bold(AMBER)));
                if let Some(top) = snapshot.top() {
                    segs.push(seg(format!(" {}", top.location()), fg(FG)));
                }
                segs.push(seg(format!(" · stop #{}", snapshot.number), fg(FAINT)));
                if let Some(description) = snapshot.description.as_deref().filter(|d| *d != snapshot.reason) {
                    segs.push(sep());
                    segs.push(seg(description.to_owned(), fg(DIM)));
                }
            } else {
                segs.push(seg("stopped · reading state…", fg(DIM)));
            }
        }
    }
    let live = tab.engine.sessions().iter().filter(|s| s.is_alive()).count();
    if live > 1 {
        segs.push(sep());
        segs.push(seg(format!("{live} live sessions"), fg(FAINT)));
        if let Some(key) = app.keymap.key_for(tab.focus().name(), "session next") {
            segs.push(seg(format!("  {key} next"), fg(GHOST)));
        }
    }
    if let Some(path) = &tab.ui.source.browse {
        let rel = path.strip_prefix(tab.engine.workspace()).unwrap_or(path);
        segs.push(sep());
        segs.push(seg("browsing ", fg(FAINT)));
        segs.push(seg(rel.display().to_string(), fg(FG)));
        segs.push(seg("  . follows execution", fg(GHOST)));
    }
    p.segs(1, y, &segs, area.right());
}

/// The context line of a tab with no session.
fn idle(tab: &Tab) -> Vec<Seg<'static>> {
    let sep = || seg("  ·  ", fg(GHOST));
    let Some(project) = &tab.project else {
        return vec![
            seg("no project", fg(DIM)),
            sep(),
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
    let mut segs = vec![
        seg("no session", fg(DIM)),
        sep(),
        seg(format!("{configs} launch configuration{}", if configs == 1 { "" } else { "s" }), fg(FAINT)),
        sep(),
    ];
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
    let pane = app.focus();
    let specific: &[(&str, &str)] = match pane {
        _ if app.tab().project.is_none() => &[("tab open", "open a project")],
        Pane::Source => &[("bp toggle", "breakpoint"), ("find", "find"), ("follow", "follow")],
        Pane::Locals => &[("open", "expand"), ("watch add", "watch"), ("copy", "copy")],
        Pane::Stack => &[("open", "show frame"), ("vendor", "library frames")],
        Pane::Watch => &[("watch add", "add"), ("watch delete", "delete")],
        Pane::Breakpoints => &[
            ("bp scope", "scope"),
            ("bp condition", "condition"),
            ("bp log", "logpoint"),
            ("bp enable", "on/off"),
            ("bp delete", "delete"),
        ],
        Pane::Output => &[("clear output", "clear")],
        Pane::Config => &[
            ("config start", "start"),
            ("open", "edit"),
            ("config add", "add"),
            ("config map", "map"),
            ("config unset", "unset"),
            ("config delete", "delete"),
        ],
        Pane::Events => &[],
        Pane::History => &[("open", "open stop"), ("right", "column")],
    };
    let common: &[(&str, &str)] = &[
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
    for (command, label) in specific.iter().chain(common) {
        if one_tab && *command == "tab next" {
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
    fn tabs_keep_full_names_and_the_clock_while_they_fit() {
        let tabs = [(3, true), (3, false)];
        // " api ■ " and " web " plus the clock.
        assert_eq!(tabs_width(&tabs, usize::MAX), 12);
        assert_eq!(fit_tabs(&tabs, 0, 12 + CLOCK), TabFit { name: 3, clock: true, shown: 0..2 });
    }

    #[test]
    fn names_shrink_before_the_clock_goes_and_the_clock_before_tabs() {
        let tabs = [(12, true), (9, true), (4, false)];
        let full = tabs_width(&tabs, usize::MAX);
        let fit = fit_tabs(&tabs, 1, full + CLOCK - 5);
        assert_eq!((fit.clock, fit.shown.clone()), (true, 0..3));
        assert!(fit.name < 12 && tabs_width(&tabs, fit.name) + CLOCK <= full + CLOCK - 5);

        let shortest = tabs_width(&tabs, MIN_NAME);
        assert_eq!(fit_tabs(&tabs, 1, shortest + 1), TabFit { name: MIN_NAME, clock: false, shown: 0..3 });
    }

    #[test]
    fn tabs_far_from_the_active_one_give_way_last() {
        let tabs = [(8, true); 9];
        let fit = fit_tabs(&tabs, 6, 40);
        assert_eq!((fit.name, fit.clock), (MIN_NAME, false));
        assert!(fit.shown.contains(&6) && fit.shown.len() < 9);
        assert!(tabs_width(&tabs[fit.shown.clone()], MIN_NAME) + 5 <= 40);
    }
}
