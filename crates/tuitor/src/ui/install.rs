//! INSTALL: what installing an adapter downloads from where, or runs, before `y` says so;
//! then its steps, and what the adapters find once it is done.

use std::time::Duration;

use ratatui::layout::Rect;
use ratatui::style::Style;
use tuitor_core::config::home_relative;
use tuitor_core::install::Plan;

use super::paint::{Painter, Panel, seg, wrap_hard};
use super::theme::*;
use crate::app::App;
use crate::installer::{Installer, State, found};

pub fn draw(p: &mut Painter<'_>, app: &App, installer: &Installer, screen: Rect) {
    p.dim(screen);
    let width = screen.width.saturating_sub(8).min(112);
    let room = usize::from(width.saturating_sub(6));
    let lines = lines(app, installer, room);
    let height = (u16::try_from(lines.len()).unwrap_or(u16::MAX) + 4).clamp(8, screen.height - super::HEADER - 4);
    let rect = Rect::new((screen.width - width) / 2, super::HEADER + 2, width, height);
    p.fill(rect, BG);
    let (state, color) = match (&installer.state, &installer.plan) {
        (State::Asking, Plan::Distro { .. }) => ("your distribution's", DIM),
        (State::Asking, _) => ("asks first", DIM),
        (State::Running(_), _) => ("installing…", DIM),
        (State::Done(Ok(found)), _) if found.iter().all(|l| l.starts_with('✓')) => ("installed", DIM),
        (State::Done(_), _) => ("failed", RED),
    };
    let sub = vec![seg(installer.package().name(), fg(DIM))];
    p.panel(rect, &Panel { title: "INSTALL", sub, right: vec![seg(state, fg(color))], keys: vec![], focus: true });
    if installer.running() {
        // Drawn again soon, with the steps taken meanwhile.
        app.ui.animate(Duration::from_millis(150));
    }
    let inner = super::paint::inner(rect);
    for (row, (text, style)) in lines.iter().enumerate().take(usize::from(inner.height.saturating_sub(1))) {
        p.text(inner.x + 2, inner.y + row as u16, text, *style, inner.right().saturating_sub(1));
    }
    footer(p, installer, inner);
}

/// The plan while it asks; the steps and the result once it runs.
fn lines(app: &App, installer: &Installer, room: usize) -> Vec<(String, Style)> {
    let mut lines: Vec<(String, Style)> = Vec::new();
    let mut say = |text: &str, style: Style| lines.extend(wrap_hard(text, room).into_iter().map(|line| (line, style)));
    if let State::Asking = installer.state {
        match &installer.plan {
            Plan::Release(release) => {
                say(&format!("{} {}, as its authors release it on GitHub:", release.package, release.version), fg(FG));
                say(&format!("    {}", release.url), bold(FG));
                let replacing = if release.dir.exists() { ", in place of what is there" } else { "" };
                say(&format!("into {}{replacing},", home_relative(&release.dir)), fg(DIM));
                say("downloaded with curl and unpacked, as README's \"Installing adapters\" does.", fg(FAINT));
                for tool in release.missing_tools() {
                    say(&format!("It needs `{tool}`, which is not on PATH."), fg(RED));
                }
            }
            Plan::Go(go) => {
                say("Delve, built by Go, as its installation guide says:", fg(FG));
                say(&format!("    {}", go.command()), bold(FG));
                say(
                    "Go puts dlv in its bin directory (GOBIN, else bin in GOPATH), which tuitor needs on PATH.",
                    fg(FAINT),
                );
            }
            Plan::Distro { package, command } => {
                say(
                    &format!("{package} comes with your distribution; tuitor does not install it, as that takes root:"),
                    fg(FG),
                );
                say(&format!("    {command}"), bold(FG));
                // What there is already.
                let found = found(*package, &app.adapters, app.tab().engine.workspace());
                for line in found {
                    say(&line, fg(if line.starts_with('✓') { DIM } else { FAINT }));
                }
            }
        }
        return lines;
    }
    let done = !installer.running();
    let last = installer.steps.len().saturating_sub(1);
    for (i, step) in installer.steps.iter().enumerate() {
        let current = i == last && !done;
        let text = if current { format!("› {step}…") } else { format!("· {step}") };
        say(&text, fg(if current { FG } else { DIM }));
    }
    match &installer.state {
        State::Done(Ok(found)) => {
            say("", fg(FAINT));
            for line in found {
                say(line, if line.starts_with('✓') { fg(DIM) } else { fg(RED) });
            }
        }
        State::Done(Err(err)) => {
            say("", fg(FAINT));
            say(&format!("✗ {err}"), fg(RED));
        }
        State::Asking | State::Running(_) => {}
    }
    lines
}

fn footer(p: &mut Painter<'_>, installer: &Installer, inner: Rect) {
    let actions: &[(&str, &str)] = match (&installer.state, &installer.plan) {
        (State::Asking, Plan::Distro { .. }) => &[("⏎", "copy the command"), ("esc", "close")],
        (State::Asking, _) if installer.can_start() => &[("y", "install"), ("n", "cancel")],
        (State::Asking, _) => &[("esc", "close")],
        (State::Running(_), _) => &[("esc", "close; installing goes on")],
        (State::Done(_), _) => &[("⏎", "close")],
    };
    let y = inner.bottom().saturating_sub(1);
    let mut x = inner.x + 1;
    for (key, label) in actions {
        x = p.text(x, y, key, bold(KEY), inner.right());
        x = p.text(x, y, &format!(" {label}   "), fg(FAINT), inner.right());
    }
}
