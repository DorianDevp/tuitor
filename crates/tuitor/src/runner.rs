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
use crate::job::{self, Job, Turn};
use crate::signals::Signals;
use crate::tab::TabId;
use crate::ui;

/// Messages of every tab's engine, each labelled with its tab.
pub type Inbox = UnboundedReceiver<(TabId, EngineMsg)>;

#[cfg(unix)]
type Remote = crate::remote::Asked;
/// Nothing comes from other programs without Unix sockets.
#[cfg(not(unix))]
type Remote = std::convert::Infallible;

/// Runs until the user quits or a signal says to, then ends every session; returns the
/// status to exit with.
pub async fn run(
    mut app: App,
    mut inbox: Inbox,
    start: Option<LaunchConfig>,
    mut signals: Signals,
) -> Result<ExitCode> {
    // Command lines from editors, `tuitor send`; the socket goes when the listener is dropped.
    #[cfg(unix)]
    let (_listener, mut remote) = match crate::remote::listen() {
        Ok((listener, rx)) => (Some(listener), rx),
        Err(err) => {
            tracing::warn!("not listening for `tuitor send`: {err:#}");
            app.execute_error(&format!("`tuitor send` cannot reach this tuitor: {err:#}"));
            (None, tokio::sync::mpsc::unbounded_channel().1)
        }
    };
    #[cfg(not(unix))]
    let mut remote = tokio::sync::mpsc::unbounded_channel::<Remote>().1;
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app, &mut inbox, &mut remote, start, &mut signals).await;
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
    remote: &mut UnboundedReceiver<Remote>,
    config: Option<LaunchConfig>,
    signals: &mut Signals,
) -> Result<ExitCode> {
    if let Some(config) = config {
        app.start(config);
    }
    let mut input = EventStream::new();
    let input = &mut input;
    // Editors running beside tuitor, until they say how opening the file went.
    let mut beside = JoinSet::new();
    // The editor that ran in tuitor's terminal and stopped, ctrl-z in it, for `edit` to resume.
    let mut stopped = Stopped(None);
    let stopped = &mut stopped.0;
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
                    if std::mem::take(&mut app.suspend) {
                        let result = suspend(terminal, input).await?;
                        app.resumed(result);
                    }
                    run_editor(terminal, input, app, &mut beside, stopped).await?;
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
                // An adapter that asked for a terminal, as BuildKit does for a build step's shell.
                app.take_terminal();
                run_editor(terminal, input, app, &mut beside, stopped).await?;
            }
            Some(asked) = remote.recv() => {
                #[cfg(unix)]
                {
                    let reply = app.remote(&asked.request);
                    let _ = asked.answer.send(reply);
                    if std::mem::take(&mut app.suspend) {
                        let result = suspend(terminal, input).await?;
                        app.resumed(result);
                    }
                    run_editor(terminal, input, app, &mut beside, stopped).await?;
                }
                #[cfg(not(unix))]
                match asked {}
            }
            _ = tick.tick() => {}
            _ = tokio::time::sleep(frame.unwrap_or_default()), if frame.is_some() => {}
            Some(done) = beside.join_next() => app.edited(done.unwrap_or_else(|err| Err(err.to_string()))),
            code = signals.recv() => return Ok(code),
        }
    }
}

/// Runs the editor, or the command, the app asked for: in tuitor's terminal, which it has until
/// it stops or exits, or beside tuitor; or resumes the one that stopped.
async fn run_editor(
    terminal: &mut DefaultTerminal,
    input: &mut EventStream,
    app: &mut App,
    beside: &mut JoinSet<Result<String, String>>,
    stopped: &mut Option<Job>,
) -> Result<()> {
    let resume = std::mem::take(&mut app.resume_editor).then(|| stopped.take()).flatten();
    let turn = match (resume, app.editor.take()) {
        (Some(job), _) => Some(in_terminal(terminal, input, Ok(job)).await?),
        (None, Some(editor)) if editor.terminal => Some(in_terminal(terminal, input, Err(&editor)).await?),
        (None, Some(editor)) => {
            beside.spawn(editor.run_beside());
            None
        }
        (None, None) => None,
    };
    match turn {
        Some(Turn::Stopped(job)) => {
            app.editor_stopped(job.name());
            *stopped = Some(job);
        }
        Some(Turn::Ended(result)) => app.edited(result),
        None => {}
    }
    Ok(())
}

/// An editor that stopped, hung up when tuitor's loop ends however it ends.
struct Stopped(Option<Job>);

impl Drop for Stopped {
    fn drop(&mut self) {
        if let Some(job) = self.0.take() {
            job.hang_up();
        }
    }
}

/// Hands the terminal to an editor, `job` one that stopped or else a new one started from
/// `editor`, until it stops or exits: the screen is the shell's again, and keys go to the
/// editor, since crossterm reads them only for a stream that is polled.
async fn in_terminal(
    terminal: &mut DefaultTerminal,
    input: &mut EventStream,
    job: Result<Job, &Editor>,
) -> Result<Turn> {
    release(terminal, input)?;
    let turn = match job {
        Ok(job) => {
            job.resume();
            job.turn().await
        }
        Err(editor) => match Job::start(editor) {
            Ok(job) => job.turn().await,
            Err(err) => Turn::Ended(Err(err)),
        },
    };
    reclaim(terminal)?;
    Ok(turn)
}

/// ctrl-z: stops tuitor as a job of the shell that started it, the screen the shell's again,
/// until the shell continues it (`fg`); refused when no shell would.
async fn suspend(terminal: &mut DefaultTerminal, input: &mut EventStream) -> Result<Result<String, String>> {
    if !job::can_suspend() {
        return Ok(Err("nothing would bring tuitor back: it was not started by a shell with job control".into()));
    }
    #[cfg(unix)]
    let mut continued = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::from_raw(
        nix::sys::signal::Signal::SIGCONT as i32,
    ))?;
    release(terminal, input)?;
    let stopped = job::suspend_self();
    #[cfg(unix)]
    if stopped.is_ok() {
        continued.recv().await;
    }
    reclaim(terminal)?;
    Ok(stopped.map(|()| "back from the shell".into()))
}

/// Puts the terminal back as the shell had it, for another program to have.
fn release(terminal: &mut DefaultTerminal, input: &mut EventStream) -> Result<()> {
    // The stream's thread would go on reading keys; a new one starts reading when polled.
    drop(std::mem::replace(input, EventStream::new()));
    ratatui::try_restore()?;
    terminal.show_cursor()?;
    Ok(())
}

/// Takes the terminal back for tuitor after another program had it.
fn reclaim(terminal: &mut DefaultTerminal) -> Result<()> {
    enable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), EnterAlternateScreen, Clear(ClearType::All))?;
    // ratatui draws only what changed since its last frame, but the other program drew over
    // that; a new terminal draws all of the next one. (`Terminal::clear` would ask the
    // terminal where the cursor is, which not every terminal answers.)
    *terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    Ok(())
}
