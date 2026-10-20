//! The interactive loop: terminal input, the messages of every tab's engine, a clock
//! tick, a redraw when an animation on screen next changes, editors that `edit` opened,
//! and signals to end.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyEventKind};
use crossterm::terminal::{Clear, ClearType, EnterAlternateScreen, enable_raw_mode};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::{DefaultTerminal, Terminal};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinSet;
use tuitor_core::EngineMsg;
use tuitor_core::config::LaunchConfig;

use crate::app::App;
use crate::editor::Editor;
use crate::signals::Signals;
use crate::tab::TabId;
use crate::ui;

/// Messages of every tab's engine, each labelled with its tab.
pub type Inbox = UnboundedReceiver<(TabId, EngineMsg)>;

/// Runs until the user quits or a signal says to, then ends every session; returns the
/// status to exit with.
pub async fn run(
    mut app: App,
    mut inbox: Inbox,
    start: Option<LaunchConfig>,
    mut signals: Signals,
) -> Result<ExitCode> {
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app, &mut inbox, start, &mut signals).await;
    restore(terminal);
    app.shutdown().await;
    result
}

/// Puts the terminal back as it was. After SIGHUP it may be gone; `ratatui::restore` would
/// then report failing on stderr, which is gone too, and printing there panics.
fn restore(mut terminal: DefaultTerminal) {
    if let Err(err) = ratatui::try_restore().and_then(|()| terminal.show_cursor()) {
        tracing::warn!("restoring the terminal: {err}");
        // Dropped, it would try to show the cursor again and report failing to.
        std::mem::forget(terminal);
    }
}

async fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    inbox: &mut Inbox,
    config: Option<LaunchConfig>,
    signals: &mut Signals,
) -> Result<ExitCode> {
    if let Some(config) = config {
        app.start(config);
    }
    let mut input = EventStream::new();
    // Editors running beside tuitor, until they say how opening the file went.
    let mut beside = JoinSet::new();
    // The time in the top bar needs one redraw a second.
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let started = Instant::now();
    loop {
        app.poll();
        app.ui.clock = Some(started.elapsed());
        terminal.draw(|frame| ui::draw(frame, app))?;
        if app.quit {
            return Ok(ExitCode::SUCCESS);
        }
        let frame = app.ui.animating.get();
        tokio::select! {
            event = input.next() => match event {
                Some(Ok(Event::Key(key))) if key.kind != KeyEventKind::Release => {
                    app.on_key(key);
                    match app.editor.take() {
                        Some(editor) if editor.terminal => {
                            let result = in_terminal(terminal, &mut input, &editor).await?;
                            app.edited(result);
                        }
                        Some(editor) => {
                            beside.spawn(editor.run_beside());
                        }
                        None => {}
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(err)) => return Err(err.into()),
                None => return Ok(ExitCode::SUCCESS),
            },
            Some((tab, msg)) = inbox.recv() => {
                app.on_engine(tab, msg);
                while let Ok((tab, msg)) = inbox.try_recv() {
                    app.on_engine(tab, msg);
                }
                app.after_engine();
            }
            _ = tick.tick() => {}
            _ = tokio::time::sleep(frame.unwrap_or_default()), if frame.is_some() => {}
            Some(done) = beside.join_next() => app.edited(done.unwrap_or_else(|err| Err(err.to_string()))),
            code = signals.recv() => return Ok(code),
        }
    }
}

/// Hands the terminal to `editor` until it exits: the screen is the shell's again, and keys
/// go to the editor, since crossterm reads them only for a stream that is polled.
async fn in_terminal(
    terminal: &mut DefaultTerminal,
    input: &mut EventStream,
    editor: &Editor,
) -> Result<Result<String, String>> {
    // The stream's thread would go on reading keys; a new one starts reading when polled.
    drop(std::mem::replace(input, EventStream::new()));
    ratatui::try_restore()?;
    terminal.show_cursor()?;
    let result = editor.run_in_terminal().await;
    enable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), EnterAlternateScreen, Clear(ClearType::All))?;
    // ratatui draws only what changed since its last frame, but the editor drew over that;
    // a new terminal draws all of the next one. (`Terminal::clear` would ask the terminal
    // where the cursor is, which not every terminal answers.)
    *terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    Ok(result)
}
