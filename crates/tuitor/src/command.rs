//! Every user action is a command. Keys are aliases for command lines and the
//! palette runs the same lines, so `c` and `/continue` cannot drift apart.

use std::fmt;

use crate::app::Pane;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum View {
    Overview,
    Code,
    History,
    /// The project's breakpoints and launch configurations.
    Project,
}

impl View {
    pub const ALL: [View; 4] = [View::Overview, View::Code, View::History, View::Project];

    pub fn name(self) -> &'static str {
        match self {
            View::Overview => "overview",
            View::Code => "code",
            View::History => "history",
            View::Project => "project",
        }
    }

    /// A view, or a pane to show in its view.
    fn parse(arg: &str) -> Result<(View, Option<Pane>), String> {
        Ok(match arg {
            "overview" | "o" => (View::Overview, None),
            "code" | "c" => (View::Code, None),
            "history" | "h" => (View::History, None),
            "project" | "p" => (View::Project, None),
            "breakpoints" | "bp" | "b" => (View::Project, Some(Pane::Breakpoints)),
            "config" => (View::Project, Some(Pane::Config)),
            other => return Err(format!("unknown view `{other}` (overview, code, history, project)")),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BpScope {
    Session,
    All,
}

/// `file:line`, `:line` or `line`; a missing file means the file shown in SOURCE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub file: Option<String>,
    pub line: u32,
}

impl Location {
    pub fn parse(arg: &str) -> Result<Location, String> {
        let (file, line) = match arg.rsplit_once(':') {
            Some((file, line)) => (Some(file).filter(|f| !f.is_empty()), line),
            None => (None, arg),
        };
        let line = line.trim().parse().map_err(|_| format!("expected file:line, got `{arg}`"))?;
        Ok(Location { file: file.map(str::to_owned), line })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Continue,
    Next,
    StepIn,
    StepOut,
    Pause,
    Stop,
    Restart,
    HistoryPrev,
    HistoryNext,
    HistoryLive,
    BpToggle(Option<Location>),
    BpCondition(Option<String>),
    BpLog(Option<String>),
    BpDelete,
    BpEnable,
    BpScope(Option<BpScope>),
    BpClear,
    WatchAdd(Option<String>),
    WatchDelete,
    Eval(Option<String>),
    Copy,
    View(View, Option<Pane>),
    PaneNext,
    PanePrev,
    Zoom,
    Close,
    Up,
    Down,
    PageUp,
    PageDown,
    Top,
    Bottom,
    Left,
    Right,
    Open,
    Follow,
    Vendor,
    Find(Option<String>),
    SessionNext,
    SessionPrev,
    SessionStart(Option<String>),
    TabNext,
    TabPrev,
    TabNew,
    TabOpen(Option<String>),
    TabClose,
    ConfigStart,
    ConfigSet(Option<String>),
    ConfigUnset(Option<String>),
    ConfigMap(Option<String>),
    ConfigAdd(Option<String>),
    ConfigDelete,
    ClearOutput,
    Palette(Option<String>),
    Help,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Run,
    History,
    Breakpoints,
    Data,
    View,
    Navigate,
    Session,
    Tab,
    Config,
    App,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arg {
    None,
    /// Filled from context (cursor, selection) when omitted.
    Optional(&'static str),
    /// The palette asks for it.
    Required(&'static str),
}

pub struct Spec {
    pub name: &'static str,
    pub arg: Arg,
    pub help: &'static str,
    pub group: Group,
    parse: fn(Option<&str>) -> Result<Action, String>,
}

impl fmt::Debug for Spec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name)
    }
}

impl Spec {
    pub fn arg_hint(&self) -> &'static str {
        match self.arg {
            Arg::None => "",
            Arg::Optional(hint) | Arg::Required(hint) => hint,
        }
    }
}

macro_rules! simple {
    ($action:expr) => {
        |_| Ok($action)
    };
}

fn text(arg: Option<&str>) -> Option<String> {
    arg.map(str::to_owned)
}

pub static COMMANDS: &[Spec] = &[
    Spec {
        name: "continue",
        arg: Arg::None,
        help: "run until the next breakpoint",
        group: Group::Run,
        parse: simple!(Action::Continue),
    },
    Spec {
        name: "next",
        arg: Arg::None,
        help: "step over the current line",
        group: Group::Run,
        parse: simple!(Action::Next),
    },
    Spec {
        name: "step in",
        arg: Arg::None,
        help: "step into the call on this line",
        group: Group::Run,
        parse: simple!(Action::StepIn),
    },
    Spec {
        name: "step out",
        arg: Arg::None,
        help: "run until the current function returns",
        group: Group::Run,
        parse: simple!(Action::StepOut),
    },
    Spec {
        name: "pause",
        arg: Arg::None,
        help: "interrupt the running program",
        group: Group::Run,
        parse: simple!(Action::Pause),
    },
    Spec {
        name: "stop",
        arg: Arg::None,
        help: "terminate the session",
        group: Group::Run,
        parse: simple!(Action::Stop),
    },
    Spec {
        name: "restart",
        arg: Arg::None,
        help: "start the session's configuration again",
        group: Group::Run,
        parse: simple!(Action::Restart),
    },
    Spec {
        name: "history prev",
        arg: Arg::None,
        help: "show the previous stop (read-only)",
        group: Group::History,
        parse: simple!(Action::HistoryPrev),
    },
    Spec {
        name: "history next",
        arg: Arg::None,
        help: "show the next stop",
        group: Group::History,
        parse: simple!(Action::HistoryNext),
    },
    Spec {
        name: "history live",
        arg: Arg::None,
        help: "back to the latest stop",
        group: Group::History,
        parse: simple!(Action::HistoryLive),
    },
    Spec {
        name: "bp toggle",
        arg: Arg::Optional("‹file:line›"),
        help: "toggle a breakpoint (SOURCE cursor by default)",
        group: Group::Breakpoints,
        parse: |arg| arg.map(Location::parse).transpose().map(Action::BpToggle),
    },
    Spec {
        name: "bp condition",
        arg: Arg::Optional("‹expr›"),
        help: "stop only when the expression is true",
        group: Group::Breakpoints,
        parse: |a| Ok(Action::BpCondition(text(a))),
    },
    Spec {
        name: "bp log",
        arg: Arg::Optional("‹message›"),
        help: "log instead of stopping ({expr} interpolates)",
        group: Group::Breakpoints,
        parse: |a| Ok(Action::BpLog(text(a))),
    },
    Spec {
        name: "bp delete",
        arg: Arg::None,
        help: "delete the selected breakpoint",
        group: Group::Breakpoints,
        parse: simple!(Action::BpDelete),
    },
    Spec {
        name: "bp enable",
        arg: Arg::None,
        help: "enable/disable the selected breakpoint or exception filter",
        group: Group::Breakpoints,
        parse: simple!(Action::BpEnable),
    },
    Spec {
        name: "bp scope",
        arg: Arg::Optional("‹session|all›"),
        help: "list breakpoints of this session or of all",
        group: Group::Breakpoints,
        parse: |arg| match arg {
            None => Ok(Action::BpScope(None)),
            Some("session") => Ok(Action::BpScope(Some(BpScope::Session))),
            Some("all") => Ok(Action::BpScope(Some(BpScope::All))),
            Some(other) => Err(format!("unknown scope `{other}` (session, all)")),
        },
    },
    Spec {
        name: "bp clear",
        arg: Arg::None,
        help: "delete all breakpoints",
        group: Group::Breakpoints,
        parse: simple!(Action::BpClear),
    },
    Spec {
        name: "watch add",
        arg: Arg::Optional("‹expr›"),
        help: "watch an expression at every stop",
        group: Group::Data,
        parse: |a| Ok(Action::WatchAdd(text(a))),
    },
    Spec {
        name: "watch delete",
        arg: Arg::None,
        help: "remove the selected watch",
        group: Group::Data,
        parse: simple!(Action::WatchDelete),
    },
    Spec {
        name: "eval",
        arg: Arg::Required("‹expr›"),
        help: "evaluate in the current frame",
        group: Group::Data,
        parse: |a| Ok(Action::Eval(text(a))),
    },
    Spec {
        name: "copy",
        arg: Arg::None,
        help: "copy the selected value to the clipboard",
        group: Group::Data,
        parse: simple!(Action::Copy),
    },
    Spec {
        name: "view",
        arg: Arg::Required("‹overview|code|history|project›"),
        help: "switch view (breakpoints and config show PROJECT)",
        group: Group::View,
        parse: |a| View::parse(a.unwrap_or_default()).map(|(view, pane)| Action::View(view, pane)),
    },
    Spec {
        name: "pane next",
        arg: Arg::None,
        help: "focus the next pane",
        group: Group::View,
        parse: simple!(Action::PaneNext),
    },
    Spec {
        name: "pane prev",
        arg: Arg::None,
        help: "focus the previous pane",
        group: Group::View,
        parse: simple!(Action::PanePrev),
    },
    Spec {
        name: "zoom",
        arg: Arg::None,
        help: "maximize the focused pane (again to restore)",
        group: Group::View,
        parse: simple!(Action::Zoom),
    },
    Spec {
        name: "close",
        arg: Arg::None,
        help: "close popup or zoom",
        group: Group::View,
        parse: simple!(Action::Close),
    },
    Spec {
        name: "follow",
        arg: Arg::None,
        help: "SOURCE follows execution again",
        group: Group::View,
        parse: simple!(Action::Follow),
    },
    Spec {
        name: "vendor",
        arg: Arg::None,
        help: "show or collapse library frames",
        group: Group::View,
        parse: simple!(Action::Vendor),
    },
    Spec { name: "up", arg: Arg::None, help: "move up", group: Group::Navigate, parse: simple!(Action::Up) },
    Spec { name: "down", arg: Arg::None, help: "move down", group: Group::Navigate, parse: simple!(Action::Down) },
    Spec {
        name: "page up",
        arg: Arg::None,
        help: "move a page up",
        group: Group::Navigate,
        parse: simple!(Action::PageUp),
    },
    Spec {
        name: "page down",
        arg: Arg::None,
        help: "move a page down",
        group: Group::Navigate,
        parse: simple!(Action::PageDown),
    },
    Spec {
        name: "top",
        arg: Arg::None,
        help: "go to the first row",
        group: Group::Navigate,
        parse: simple!(Action::Top),
    },
    Spec {
        name: "bottom",
        arg: Arg::None,
        help: "go to the last row",
        group: Group::Navigate,
        parse: simple!(Action::Bottom),
    },
    Spec {
        name: "left",
        arg: Arg::None,
        help: "collapse / previous column",
        group: Group::Navigate,
        parse: simple!(Action::Left),
    },
    Spec {
        name: "right",
        arg: Arg::None,
        help: "expand / next column",
        group: Group::Navigate,
        parse: simple!(Action::Right),
    },
    Spec {
        name: "open",
        arg: Arg::None,
        help: "expand, jump to or open the selection",
        group: Group::Navigate,
        parse: simple!(Action::Open),
    },
    Spec {
        name: "find",
        arg: Arg::Optional("‹query›"),
        help: "fuzzy find files and breakpoints",
        group: Group::Navigate,
        parse: |a| Ok(Action::Find(text(a))),
    },
    Spec {
        name: "session next",
        arg: Arg::None,
        help: "make the next session active",
        group: Group::Session,
        parse: simple!(Action::SessionNext),
    },
    Spec {
        name: "session prev",
        arg: Arg::None,
        help: "make the previous session active",
        group: Group::Session,
        parse: simple!(Action::SessionPrev),
    },
    Spec {
        name: "session start",
        arg: Arg::Required("‹config›"),
        help: "start a launch configuration",
        group: Group::Session,
        parse: |a| Ok(Action::SessionStart(text(a))),
    },
    Spec {
        name: "tab next",
        arg: Arg::None,
        help: "show the next project tab",
        group: Group::Tab,
        parse: simple!(Action::TabNext),
    },
    Spec {
        name: "tab prev",
        arg: Arg::None,
        help: "show the previous project tab",
        group: Group::Tab,
        parse: simple!(Action::TabPrev),
    },
    Spec {
        name: "tab new",
        arg: Arg::None,
        help: "add a tab and pick a project to open in it",
        group: Group::Tab,
        parse: simple!(Action::TabNew),
    },
    Spec {
        name: "tab open",
        arg: Arg::Optional("‹dir›"),
        help: "open a project in a new tab, or in this one if it is empty",
        group: Group::Tab,
        parse: |a| Ok(Action::TabOpen(text(a))),
    },
    Spec {
        name: "tab close",
        arg: Arg::None,
        help: "close this tab and stop its debuggers",
        group: Group::Tab,
        parse: simple!(Action::TabClose),
    },
    Spec {
        name: "config start",
        arg: Arg::None,
        help: "start the configuration selected in CONFIG",
        group: Group::Config,
        parse: simple!(Action::ConfigStart),
    },
    Spec {
        name: "config set",
        arg: Arg::Optional("‹field value›"),
        help: "set a field of the selected configuration (JSON or text)",
        group: Group::Config,
        parse: |a| Ok(Action::ConfigSet(text(a))),
    },
    Spec {
        name: "config unset",
        arg: Arg::Optional("‹field›"),
        help: "remove a field of the selected configuration",
        group: Group::Config,
        parse: |a| Ok(Action::ConfigUnset(text(a))),
    },
    Spec {
        name: "config map",
        arg: Arg::Required("‹remote=local›"),
        help: "map a path the adapter sees to one here (remote= removes it)",
        group: Group::Config,
        parse: |a| Ok(Action::ConfigMap(text(a))),
    },
    Spec {
        name: "config add",
        arg: Arg::Required("‹type›"),
        help: "add a configuration: php, rust, c, js, gdb, pwa-chrome, pwa-node…",
        group: Group::Config,
        parse: |a| Ok(Action::ConfigAdd(text(a))),
    },
    Spec {
        name: "config delete",
        arg: Arg::None,
        help: "delete the selected configuration",
        group: Group::Config,
        parse: simple!(Action::ConfigDelete),
    },
    Spec {
        name: "clear output",
        arg: Arg::None,
        help: "clear the OUTPUT pane",
        group: Group::App,
        parse: simple!(Action::ClearOutput),
    },
    Spec {
        name: "palette",
        arg: Arg::Optional("‹text›"),
        help: "open the command line",
        group: Group::App,
        parse: |a| Ok(Action::Palette(text(a))),
    },
    Spec {
        name: "help",
        arg: Arg::None,
        help: "list commands and their keys",
        group: Group::App,
        parse: simple!(Action::Help),
    },
    Spec {
        name: "quit",
        arg: Arg::None,
        help: "disconnect everything and exit",
        group: Group::App,
        parse: simple!(Action::Quit),
    },
];

/// A parsed command line.
#[derive(Debug)]
pub struct Invocation {
    pub spec: &'static Spec,
    pub action: Action,
}

pub fn find(name: &str) -> Option<&'static Spec> {
    COMMANDS.iter().find(|s| s.name == name)
}

/// Splits `/bp condition $i > 2` into the longest matching command name and its argument.
pub fn split(line: &str) -> Option<(&'static Spec, Option<&str>)> {
    let line = line.trim().trim_start_matches('/').trim();
    COMMANDS
        .iter()
        .filter(|spec| line == spec.name || line.strip_prefix(spec.name).is_some_and(|rest| rest.starts_with(' ')))
        .max_by_key(|spec| spec.name.len())
        .map(|spec| {
            let rest = line[spec.name.len()..].trim();
            (spec, Some(rest).filter(|r| !r.is_empty()))
        })
}

pub fn parse(line: &str) -> Result<Invocation, String> {
    let (spec, arg) = split(line).ok_or_else(|| {
        let shown = line.trim().trim_start_matches('/');
        format!("unknown command `{shown}`")
    })?;
    let action = (spec.parse)(arg)?;
    Ok(Invocation { spec, action })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longest_name_wins_and_rest_is_argument() {
        let inv = parse("/bp condition $i > 2").expect("parse");
        assert_eq!(inv.spec.name, "bp condition");
        assert_eq!(inv.action, Action::BpCondition(Some("$i > 2".into())));
        assert_eq!(parse("next").expect("next").action, Action::Next);
    }

    #[test]
    fn locations() {
        assert_eq!(
            parse("bp toggle src/OrderService.php:141").expect("parse").action,
            Action::BpToggle(Some(Location { file: Some("src/OrderService.php".into()), line: 141 }))
        );
        assert_eq!(Location::parse(":29"), Ok(Location { file: None, line: 29 }));
        assert!(parse("bp toggle nope").is_err());
    }

    #[test]
    fn rejects_unknown_and_prefix_without_space() {
        assert!(parse("/frobnicate").is_err());
        assert!(parse("continuex").is_err());
        assert_eq!(parse("view h").expect("view").action, Action::View(View::History, None));
        assert_eq!(parse("view bp").expect("view").action, Action::View(View::Project, Some(Pane::Breakpoints)));
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<_> = COMMANDS.iter().map(|s| s.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), COMMANDS.len());
    }
}
