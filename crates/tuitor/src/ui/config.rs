//! CONFIG: where the project is and what kind it is, its launch configurations with the
//! one `S` starts, and the fields of the selected one. Those of launch.json are shown
//! as they are; tuitor's own are edited with `/config`.

use ratatui::layout::Rect;
use serde_json::Value;
use tuitor_core::config::home_relative;
use tuitor_core::project::{Configuration, Project, Source, starts_browser, value_text};

use super::paint::{Painter, Panel, Seg, seg, truncate, truncate_left, wrap};
use super::theme::*;
use crate::app::App;
use crate::model::ConfigRow;
use crate::tab::Tab;

/// One line of the pane: text on the left, a note on the right, and the cursor row it
/// shows, if it is one.
struct Line {
    row: Option<usize>,
    left: Vec<Seg<'static>>,
    right: Vec<Seg<'static>>,
}

impl Line {
    fn text(left: Vec<Seg<'static>>) -> Line {
        Line { row: None, left, right: Vec::new() }
    }
}

pub fn draw(p: &mut Painter<'_>, app: &App, area: Rect, focus: bool) {
    let tab = app.tab();
    let sub = match &tab.project {
        Some(project) if project.kinds.is_empty() => vec![seg("no known kind", fg(FAINT))],
        Some(project) => {
            let kinds: Vec<&str> = project.kinds.iter().map(|k| k.name()).collect();
            vec![seg(kinds.join(" · "), fg(DIM))]
        }
        None => Vec::new(),
    };
    let keys =
        if focus { vec![("⏎", "edit"), ("a", "add"), ("m", "map"), ("d", "del")] } else { vec![("S", "start")] };
    p.panel(area, &Panel { title: "CONFIG", sub, right: vec![], keys, focus });

    let inner = super::paint::inner(area);
    let Some(project) = &tab.project else {
        p.text(inner.x + 2, inner.y, "no project in this tab · O opens one", fg(FAINT), inner.right());
        return;
    };
    let lines = lines(app, tab, project, usize::from(inner.width));
    let height = usize::from(inner.height);
    let cursor = lines.iter().position(|line| line.row == Some(tab.ui.config.pos)).unwrap_or(0);
    let offset = (cursor + 2).saturating_sub(height);
    for (i, line) in lines.iter().enumerate().skip(offset).take(height) {
        let y = inner.y + (i - offset) as u16;
        if focus && line.row.is_some() && line.row == Some(tab.ui.config.pos) {
            p.band(inner.x, y, inner.width, SEL2);
        }
        let end = p.segs(inner.x + 1, y, &line.left, inner.right().saturating_sub(1));
        if !line.right.is_empty() {
            p.segs_right(inner.right().saturating_sub(1), y, &line.right, end + 2);
        }
    }
}

fn lines(app: &App, tab: &Tab, project: &Project, width: usize) -> Vec<Line> {
    // Paths are cut from the left: their end tells them apart.
    let path = |path: &std::path::Path, room: usize| truncate_left(&home_relative(path), room).into_owned();
    let mut lines = vec![Line::text(vec![seg(path(&project.dir, width.saturating_sub(2)), fg(DIM))])];
    for error in &project.errors {
        lines.push(Line::text(vec![seg(error.clone(), fg(RED_DIM))]));
    }
    lines.push(Line::text(Vec::new()));
    lines.push(Line {
        row: None,
        left: vec![seg("CONFIGURATIONS", bold(FAINT))],
        right: vec![seg("FROM", bold(FAINT))],
    });
    if project.configurations.is_empty() {
        let hint = "none yet · a adds one: php, rust, go, c, js, gdb, pwa-chrome, pwa-node";
        lines.push(Line::text(vec![seg(hint, italic(FAINT))]));
    }
    let name_width = project.configurations.iter().map(|c| c.name().chars().count()).max().unwrap_or(0).min(width / 2);
    let rows = tab.config_rows();
    for (row, entry) in rows.iter().enumerate() {
        let ConfigRow::Configuration(index) = *entry else { continue };
        let config = &project.configurations[index];
        // The one CONFIG shows is the one `S` starts.
        let marker = if index == tab.ui.config_sel { seg("S ", bold(KEY)) } else { seg("  ", fg(FAINT)) };
        let name = format!("{:<name_width$}  ", truncate(config.name(), name_width));
        lines.push(Line {
            row: Some(row),
            left: vec![marker, seg(name, fg(FG)), seg(config.adapter().to_owned(), fg(DIM))],
            right: vec![seg(source_label(project, config), fg(FAINT))],
        });
    }

    let Some(config) = tab.selected_configuration() else { return lines };
    lines.push(Line::text(Vec::new()));
    let note = match &config.source {
        Source::Tuitor => "tuitor's own · saved as you change it",
        Source::LaunchJson(_) => "read-only · change it in launch.json",
    };
    lines.push(Line {
        row: None,
        left: vec![seg(config.name().to_owned(), bold(FG))],
        right: vec![seg(note, fg(FAINT))],
    });
    let key_width = config.fields.keys().map(|k| k.chars().count()).max().unwrap_or(0).min(16);
    let mut first_mapping = true;
    for (row, entry) in rows.iter().enumerate() {
        let (key, value) = match entry {
            ConfigRow::Configuration(_) => continue,
            ConfigRow::Field(key) => (key.clone(), field_value(app, config, key)),
            // Mappings are one per row, under one key.
            ConfigRow::Mapping(remote) => {
                let key = if first_mapping { "pathMappings".to_owned() } else { String::new() };
                first_mapping = false;
                let room = width.saturating_sub(key_width + 6 + remote.chars().count() + 3);
                (key, mapping(config, remote, room))
            }
        };
        let mut left = vec![seg(format!("  {:<key_width$}  ", truncate(&key, key_width)), fg(DIM))];
        left.extend(value);
        lines.push(Line { row: Some(row), left, right: Vec::new() });
    }
    if starts_browser(config.adapter()) && !config.fields.contains_key("runtimeExecutable") {
        lines.extend(browser_hint(config, width.saturating_sub(4)));
    }
    lines.push(Line::text(Vec::new()));
    let kept = "tuitor's own are kept in ";
    let store = path(app.store.path(), width.saturating_sub(2 + kept.len()));
    lines.push(Line::text(vec![seg(kept, fg(GHOST)), seg(store, fg(GHOST))]));
    lines
}

fn source_label(project: &Project, config: &Configuration) -> String {
    match &config.source {
        Source::Tuitor => "tuitor".to_owned(),
        Source::LaunchJson(path) => path.strip_prefix(&project.dir).unwrap_or(path).display().to_string(),
    }
}

/// A field's value as it is typed after `/config set`; a `type` without an adapter says so.
fn field_value(app: &App, config: &Configuration, key: &str) -> Vec<Seg<'static>> {
    let Some(value) = config.fields.get(key) else { return Vec::new() };
    let mut segs = vec![seg(value_text(value), fg(FG))];
    if key == "type" && app.adapters.get(config.adapter()).is_none() {
        segs.push(seg("  no such adapter yet", italic(FAINT)));
    }
    segs
}

/// What a browser configuration without `runtimeExecutable` starts: Google Chrome, which
/// js-debug looks for and fails without. tuitor names the browser it finds on PATH in one it
/// makes, so this one is from launch.json, or was made where there was none.
fn browser_hint(config: &Configuration, width: usize) -> Vec<Line> {
    let how = match config.source {
        Source::Tuitor => "/config set runtimeExecutable ‹path›",
        Source::LaunchJson(_) => "set runtimeExecutable in launch.json",
    };
    let what = "No runtimeExecutable, so js-debug starts Google Chrome, and fails where there is none.";
    let fix = format!("For Chromium or another browser: {how}");
    let lines = |text: &str, style| {
        wrap(text, width).into_iter().map(move |line| Line::text(vec![seg(format!("  {line}"), style)]))
    };
    lines(what, italic(YELLOW_DIM)).chain(lines(&fix, italic(FAINT))).collect()
}

/// `remote → local`, the local path cut from the left to `room` columns.
fn mapping(config: &Configuration, remote: &str, room: usize) -> Vec<Seg<'static>> {
    let local = config.fields.get("pathMappings").and_then(|m| m.get(remote)).and_then(Value::as_str).unwrap_or("");
    let local = truncate_left(&home_relative(std::path::Path::new(local)), room).into_owned();
    vec![seg(remote.to_owned(), fg(FG)), seg(" → ", fg(FAINT)), seg(local, fg(FG))]
}
