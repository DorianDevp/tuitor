//! Drawing. Every frame is rendered from scratch from `App`; nothing here mutates
//! state except the scroll offsets and the animation flag kept in `Cell`s.

mod breakpoints;
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

use std::time::Duration;

use chrono::Local;
use ratatui::Frame;
use ratatui::layout::Rect;
use tuitor_core::Level;
use tuitor_core::session::SessionState;

use crate::app::{App, Pane};
use crate::command::View;
use crate::overlay::Overlay;
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
    if app.ui.view == View::History && !app.ui.zoom {
        history::value(&mut p, app, history_value_area(body));
    }
    status(&mut p, app, Rect::new(0, area.height - 2, area.width, 1));
    hints(&mut p, app, Rect::new(0, area.height - 1, area.width, 1));

    match &app.ui.overlay {
        Some(Overlay::Palette(palette)) => overlays::palette(&mut p, app, palette, area),
        Some(Overlay::Finder(finder)) => overlays::finder(&mut p, app, finder, area),
        None => {}
    }
}

fn draw_pane(p: &mut Painter<'_>, app: &App, pane: Pane, area: Rect) {
    let focus = app.focus() == pane && app.ui.overlay.is_none();
    match pane {
        Pane::Source => source::draw(p, app, area, focus),
        Pane::Locals => locals::draw(p, app, area, focus),
        Pane::Stack => stack::draw(p, app, area, focus),
        Pane::Watch => watch::draw(p, app, area, focus),
        Pane::Breakpoints => breakpoints::draw(p, app, area, focus, app.ui.view == View::Breakpoints || app.ui.zoom),
        Pane::Output => logs::output(p, app, area, focus),
        Pane::Events => logs::events(p, app, area, focus),
        Pane::History => history::table(p, app, area, focus),
    }
}

fn pct(total: u16, percent: u16) -> u16 {
    total * percent / 100
}

/// Pane rectangles for the current view (one when zoomed).
fn layout(app: &App, body: Rect) -> Vec<(Pane, Rect)> {
    if app.ui.zoom {
        return vec![(app.focus(), body)];
    }
    let (x, y, w, h) = (body.x, body.y, body.width, body.height);
    match app.ui.view {
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
        View::Breakpoints => vec![(Pane::Breakpoints, body)],
    }
}

fn history_value_area(body: Rect) -> Rect {
    let table_h = pct(body.height, 48).max(8);
    Rect::new(body.x, body.y + table_h, body.width / 2, body.height - table_h)
}

fn topbar(p: &mut Painter<'_>, app: &App, area: Rect) {
    p.fill(area, BAR);
    let y = area.y;
    let right = topbar_right(app);
    let tabs: u16 = View::ALL.iter().map(|view| view.name().len() as u16 + 2).sum();
    // The logo gives way first: it drops its name before a session is cut. Around
    // it: the margin, " │ " before the tabs and the gap after them.
    let room = area.width.saturating_sub(1 + 3 + tabs + 2 + paint::width(&right));
    let mut x = logo::bar(p, 1, y, room, bar_clock(app));
    x = p.text(x + 1, y, "│", fg(GHOST), area.right()) + 1;
    for view in View::ALL {
        let label = format!(" {} ", view.name().to_uppercase());
        if view == app.ui.view {
            let start = x;
            x = p.text(x, y, &label, bold(WHITE), area.right());
            p.band(start, y, x - start, SEL2);
        } else {
            x = p.text(x, y, &label, fg(FAINT), area.right());
        }
    }
    let views_end = x + 2;
    p.segs_right(area.right(), y, &right, views_end);
}

/// Time for the top bar's waves, or `None` to rest them. They move while nothing
/// waits to be read: with no session, or while the active one starts, runs or has
/// ended, so they also quietly show a running program. They rest while it is
/// stopped or something is open over them.
fn bar_clock(app: &App) -> Option<Duration> {
    let stopped = app.active().is_some_and(|s| s.state == SessionState::Stopped);
    let clock = app.ui.clock.filter(|_| !stopped && app.ui.overlay.is_none());
    if let Some(t) = clock {
        app.ui.animate(logo::next_wave(t));
    }
    clock
}

/// The sessions and the time, right-aligned in the top bar.
fn topbar_right(app: &App) -> Vec<Seg<'static>> {
    let mut right = Vec::new();
    for session in app.engine.sessions() {
        let active = app.ui.session == Some(session.id);
        let (glyph, color) = match session.state {
            SessionState::Stopped => ("■", if active { AMBER } else { FAINT }),
            SessionState::Running => ("▶", FAINT),
            SessionState::Starting => ("⧗", FAINT),
            SessionState::Exited => ("✕", GHOST),
        };
        let name_style = if active {
            bold(WHITE)
        } else if session.is_alive() {
            fg(tag(&session.tag))
        } else {
            fg(GHOST)
        };
        right.push(seg(session.name.clone(), name_style));
        right.push(seg(format!(" {glyph} "), bold(color)));
        right.push(seg("│ ", fg(GHOST)));
    }
    right.push(seg(Local::now().format("%H:%M:%S ").to_string(), fg(DIM)));
    right
}

fn context(p: &mut Painter<'_>, app: &App, area: Rect) {
    let y = area.y;
    let sep = || seg("  ·  ", fg(GHOST));
    if app.in_history()
        && let (Some(session), Some(snapshot)) = (app.active(), app.viewed())
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

    let Some(session) = app.active() else {
        let configs = app.configs.len();
        let segs = [
            seg("no session", fg(DIM)),
            sep(),
            seg(format!("{configs} launch configuration{}", if configs == 1 { "" } else { "s" }), fg(FAINT)),
            sep(),
            seg("S", bold(KEY)),
            seg(" start one   ", fg(FAINT)),
            seg("f", bold(KEY)),
            seg(" find a file   ", fg(FAINT)),
            seg("b", bold(KEY)),
            seg(" breakpoints work before starting", fg(FAINT)),
        ];
        p.segs(1, y, &segs, area.right());
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
            if let Some(snapshot) = app.viewed() {
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
    if let Some(path) = &app.ui.source.browse {
        let rel = path.strip_prefix(app.engine.workspace()).unwrap_or(path);
        segs.push(sep());
        segs.push(seg("browsing ", fg(FAINT)));
        segs.push(seg(rel.display().to_string(), fg(FG)));
        segs.push(seg("  . follows execution", fg(GHOST)));
    }
    p.segs(1, y, &segs, area.right());
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
        ("zoom", "zoom"),
        ("palette", "command"),
        ("help", "keys"),
    ];
    let mut x = 1;
    for (command, label) in specific.iter().chain(common) {
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
