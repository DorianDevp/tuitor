//! The interactive loop: terminal input, engine messages and a clock tick.

use std::time::Duration;

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc::UnboundedReceiver;
use tuitor_core::EngineMsg;
use tuitor_core::config::LaunchConfig;

use crate::app::App;
use crate::ui;

pub async fn run(mut app: App, mut rx: UnboundedReceiver<EngineMsg>, start: Option<LaunchConfig>) -> Result<()> {
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app, &mut rx, start).await;
    ratatui::restore();
    app.engine.shutdown().await;
    result
}

pub fn start(app: &mut App, config: Option<LaunchConfig>) {
    let Some(config) = config else { return };
    let name = config.name.clone();
    match app.engine.start(config) {
        Ok(sid) => app.ui.session = Some(sid),
        Err(err) => app.execute_error(&format!("{name}: {err:#}")),
    }
}

async fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    rx: &mut UnboundedReceiver<EngineMsg>,
    config: Option<LaunchConfig>,
) -> Result<()> {
    start(app, config);
    let mut input = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;
        if app.quit {
            return Ok(());
        }
        tokio::select! {
            event = input.next() => match event {
                Some(Ok(Event::Key(key))) if key.kind != KeyEventKind::Release => app.on_key(key),
                Some(Ok(_)) => {}
                Some(Err(err)) => return Err(err.into()),
                None => return Ok(()),
            },
            Some(msg) = rx.recv() => {
                app.on_engine(msg);
                while let Ok(msg) = rx.try_recv() {
                    app.on_engine(msg);
                }
                app.after_engine();
            }
            _ = tick.tick() => {}
        }
    }
}
