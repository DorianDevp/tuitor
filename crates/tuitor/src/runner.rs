//! The interactive loop: terminal input, the messages of every tab's engine, a clock
//! tick, and a redraw when an animation on screen next changes.

use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc::UnboundedReceiver;
use tuitor_core::EngineMsg;
use tuitor_core::config::LaunchConfig;

use crate::app::App;
use crate::tab::TabId;
use crate::ui;

/// Messages of every tab's engine, each labelled with its tab.
pub type Inbox = UnboundedReceiver<(TabId, EngineMsg)>;

pub async fn run(mut app: App, mut inbox: Inbox, start: Option<LaunchConfig>) -> Result<()> {
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app, &mut inbox, start).await;
    ratatui::restore();
    app.shutdown().await;
    result
}

async fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    inbox: &mut Inbox,
    config: Option<LaunchConfig>,
) -> Result<()> {
    if let Some(config) = config {
        app.start(config);
    }
    let mut input = EventStream::new();
    // The time in the top bar needs one redraw a second.
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let started = Instant::now();
    loop {
        app.ui.clock = Some(started.elapsed());
        terminal.draw(|frame| ui::draw(frame, app))?;
        if app.quit {
            return Ok(());
        }
        let frame = app.ui.animating.get();
        tokio::select! {
            event = input.next() => match event {
                Some(Ok(Event::Key(key))) if key.kind != KeyEventKind::Release => app.on_key(key),
                Some(Ok(_)) => {}
                Some(Err(err)) => return Err(err.into()),
                None => return Ok(()),
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
        }
    }
}
