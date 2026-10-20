//! Headless runs: commands from a file against a fake screen, with screens saved as
//! ANSI. Used for end-to-end checks and screenshots.
//!
//! ```text
//! # comments and blank lines are ignored
//! /bp toggle src/main.rs:12      any command line
//! @key g c                       press keys (an editor `edit` opens runs right away)
//! @wait stop                     until a new stop is recorded, in any tab
//! @wait running | exited         until the active tab's session is in that state
//! @sleep 300                     milliseconds
//! @clock 700                     animations at 700 ms (by default they hold still)
//! @dump overview.ans             save the screen
//! ```

use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use tuitor_core::config::LaunchConfig;
use tuitor_core::session::SessionState;
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::runner::Inbox;
use crate::signals::Signals;
use crate::ui;

const WAIT: Duration = Duration::from_secs(20);

/// Runs the script, or as much of it as comes before an error or a signal, then ends
/// every session; returns the status to exit with.
pub async fn run(
    mut app: App,
    mut rx: Inbox,
    config: Option<LaunchConfig>,
    script: &Path,
    size: (u16, u16),
    mut signals: Signals,
) -> Result<ExitCode> {
    app.headless = true;
    let text = std::fs::read_to_string(script).with_context(|| format!("reading {}", script.display()))?;
    if let Some(config) = config {
        app.start(config);
    }
    let result = tokio::select! {
        result = run_lines(&mut app, &mut rx, script, &text, size) => result.map(|()| ExitCode::SUCCESS),
        code = signals.recv() => Ok(code),
    };
    app.shutdown().await;
    result
}

async fn run_lines(
    app: &mut App,
    rx: &mut Inbox,
    script: &Path,
    text: &str,
    (width, height): (u16, u16),
) -> Result<()> {
    let base = script.parent().unwrap_or(Path::new("."));
    let mut terminal = Terminal::new(TestBackend::new(width, height))?;
    // `@wait stop` means a stop after the last action, even if it already arrived.
    let mut mark = stops(app);

    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let context = || format!("{}:{}: {line}", script.display(), number + 1);
        if let Some(rest) = line.strip_prefix('@') {
            let (directive, arg) = rest.split_once(' ').unwrap_or((rest, ""));
            match directive {
                "wait" => {
                    wait(app, rx, arg.trim(), mark).await.with_context(context)?;
                    mark = stops(app);
                }
                "sleep" => {
                    let ms: u64 = arg.trim().parse().with_context(context)?;
                    pump_for(app, rx, Duration::from_millis(ms)).await;
                }
                "clock" => {
                    let ms: u64 = arg.trim().parse().with_context(context)?;
                    app.ui.clock = Some(Duration::from_millis(ms));
                }
                "key" => {
                    mark = stops(app);
                    for key in arg.split_whitespace() {
                        app.on_key(key_event(key).with_context(context)?);
                        open_editor(app).await;
                        drain(app, rx);
                    }
                }
                "dump" => {
                    pump_for(app, rx, Duration::from_millis(150)).await;
                    terminal.draw(|frame| ui::draw(frame, app))?;
                    let path = base.join(arg.trim());
                    std::fs::write(&path, to_ansi(terminal.backend().buffer())).with_context(context)?;
                }
                other => bail!("{}: unknown directive @{other}", context()),
            }
        } else {
            mark = stops(app);
            app.execute(line, None);
            open_editor(app).await;
            pump_for(app, rx, Duration::from_millis(50)).await;
        }
        terminal.draw(|frame| ui::draw(frame, app))?;
    }
    Ok(())
}

/// Runs the editor `edit` asked for, and waits for it; there is no screen to hand it.
async fn open_editor(app: &mut App) {
    let Some(editor) = app.editor.take() else { return };
    let result = if editor.terminal { editor.run_in_terminal().await } else { editor.run_beside().await };
    app.edited(result);
}

/// Stops recorded in all tabs.
fn stops(app: &App) -> usize {
    app.tabs.iter().flat_map(|(_, tab)| tab.engine.sessions()).map(|s| s.history.len()).sum()
}

async fn wait(app: &mut App, rx: &mut Inbox, what: &str, before: usize) -> Result<()> {
    let state = |app: &App| app.tab().active().map(|s| s.state);
    let done = |app: &App| match what {
        "stop" => stops(app) > before,
        "running" => state(app) == Some(SessionState::Running),
        "exited" => state(app) == Some(SessionState::Exited),
        _ => true,
    };
    if !matches!(what, "stop" | "running" | "exited") {
        bail!("@wait expects stop, running or exited");
    }
    let deadline = Instant::now() + WAIT;
    while !done(app) {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Some((tab, msg))) => {
                app.on_engine(tab, msg);
                drain(app, rx);
            }
            _ => bail!("timed out waiting for {what}"),
        }
    }
    // Let the capture that follows a stop arrive.
    pump_for(app, rx, Duration::from_millis(200)).await;
    Ok(())
}

async fn pump_for(app: &mut App, rx: &mut Inbox, duration: Duration) {
    let deadline = Instant::now() + duration;
    while let Ok(Some((tab, msg))) =
        tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), rx.recv()).await
    {
        app.on_engine(tab, msg);
        drain(app, rx);
    }
    app.after_engine();
    app.poll();
}

fn drain(app: &mut App, rx: &mut Inbox) {
    while let Ok((tab, msg)) = rx.try_recv() {
        app.on_engine(tab, msg);
    }
    app.after_engine();
}

fn key_event(text: &str) -> Result<KeyEvent> {
    let (mut mods, mut rest) = (KeyModifiers::NONE, text);
    loop {
        if let Some(r) = rest.strip_prefix("ctrl-").filter(|r| !r.is_empty()) {
            (mods, rest) = (mods | KeyModifiers::CONTROL, r);
        } else if let Some(r) = rest.strip_prefix("shift-").filter(|r| !r.is_empty()) {
            (mods, rest) = (mods | KeyModifiers::SHIFT, r);
        } else {
            break;
        }
    }
    let code = match rest {
        "enter" => KeyCode::Enter,
        "esc" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "space" => KeyCode::Char(' '),
        "backspace" => KeyCode::Backspace,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        c if c.chars().count() == 1 => KeyCode::Char(c.chars().next().unwrap_or(' ')),
        other => bail!("unknown key `{other}`"),
    };
    Ok(KeyEvent::new(code, mods))
}

/// Truecolor ANSI for `cat` in a terminal.
pub fn to_ansi(buffer: &Buffer) -> String {
    let mut out = String::from("\x1b[2J\x1b[H");
    for y in 0..buffer.area.height {
        let mut last = None;
        let mut skip = 0;
        for x in 0..buffer.area.width {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let Some(cell) = buffer.cell((x, y)) else { continue };
            let style = (cell.fg, cell.bg, cell.modifier);
            if last != Some(style) {
                out.push_str("\x1b[0");
                if cell.modifier.contains(Modifier::BOLD) {
                    out.push_str(";1");
                }
                if cell.modifier.contains(Modifier::ITALIC) {
                    out.push_str(";3");
                }
                push_color(&mut out, cell.fg, 38);
                push_color(&mut out, cell.bg, 48);
                out.push('m');
                last = Some(style);
            }
            out.push_str(cell.symbol());
            skip = cell.symbol().width().saturating_sub(1);
        }
        out.push_str("\x1b[0m\n");
    }
    out
}

fn push_color(out: &mut String, color: Color, base: u8) {
    if let Color::Rgb(r, g, b) = color {
        let _ = write!(out, ";{base};2;{r};{g};{b}");
    }
}
