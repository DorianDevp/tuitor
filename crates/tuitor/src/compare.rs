//! COMPARE for a tab: what two breakpoint sets recorded, each the stops at its breakpoints in
//! one run of this tab's sessions, by default the latest run that has any. FLOW remembers
//! earlier tuitors' runs by their stacks alone, without values, so the runs compared are
//! those of this one.

use tuitor_core::breakpoints::SetId;
use tuitor_core::compare::{Entry, Recording, align};
use tuitor_core::session::SessionId;

use crate::app::{App, Outcome, Pane};
use crate::command::View;
use crate::flow::RunRef;
use crate::sets::parse_set;
use crate::tab::Tab;
use crate::ui::theme::set_name;

/// One side: a set, in a run given by its session and first stop, or `None` for the latest
/// run that recorded the set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Side {
    pub set: SetId,
    pub run: Option<(SessionId, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Comparison {
    pub a: Side,
    pub b: Side,
}

/// A side as it stands now: its run and its stops.
pub struct Path<'a> {
    pub set: SetId,
    pub run: RunRef,
    pub recording: Recording<'a>,
}

/// Two sides lined up.
pub struct Compared<'a> {
    pub a: Path<'a>,
    pub b: Path<'a>,
    pub entries: Vec<Entry<'a>>,
}

impl Tab {
    /// The runs that recorded `set`, oldest first.
    pub fn runs_of(&self, set: SetId) -> Vec<RunRef> {
        let sessions = self.engine.sessions();
        let recorded = |run: &RunRef| {
            let history = sessions.iter().find(|s| s.id == run.session).map(|s| &s.history[run.stops.clone()]);
            history.is_some_and(|stops| stops.iter().any(|s| s.set == Some(set)))
        };
        self.flows.in_memory(sessions).into_iter().filter(recorded).collect()
    }

    fn path(&self, side: Side) -> Option<Path<'_>> {
        let runs = self.runs_of(side.set);
        let run = match side.run {
            Some((session, start)) => runs.into_iter().find(|r| r.session == session && r.stops.start == start)?,
            None => runs.into_iter().last()?,
        };
        let session = self.engine.session(run.session)?;
        let stops = &session.history[run.stops.clone()];
        let recording =
            Recording { stops: stops.iter().filter(|s| s.set == Some(side.set)).collect(), start: stops.first()?.time };
        Some(Path { set: side.set, run, recording })
    }

    /// What COMPARE shows: what `/compare` asked for, else [`Tab::default_comparison`].
    pub fn comparison(&self) -> Option<Comparison> {
        self.ui.comparison.or_else(|| self.default_comparison())
    }

    /// The active set against the other set that recorded last; with no other, the active
    /// set against itself in the run before its latest.
    fn default_comparison(&self) -> Option<Comparison> {
        let bps = self.engine.breakpoints();
        let active = bps.active_set();
        let last_run = |set| self.runs_of(set).last().map(|r| self.recorded_at(r));
        let other = bps.sets().iter().copied().filter(|s| *s != active).max_by_key(|s| last_run(*s));
        if let Some(other) = other.filter(|s| last_run(*s).is_some()) {
            return Some(Comparison { a: Side { set: active, run: None }, b: Side { set: other, run: None } });
        }
        let runs = self.runs_of(active);
        let before = runs.len().checked_sub(2).map(|i| &runs[i])?;
        let a = Side { set: active, run: Some((before.session, before.stops.start)) };
        Some(Comparison { a, b: Side { set: active, run: None } })
    }

    /// When a run's last stop was recorded.
    fn recorded_at(&self, run: &RunRef) -> Option<chrono::DateTime<chrono::Local>> {
        let session = self.engine.session(run.session)?;
        Some(session.history.get(run.stops.end.checked_sub(1)?)?.time)
    }

    /// `side` in the run `delta` runs from its own among those that recorded its set.
    fn shifted(&self, side: Side, delta: isize) -> Side {
        let runs = self.runs_of(side.set);
        let current = match side.run {
            Some(key) => runs.iter().position(|r| (r.session, r.stops.start) == key),
            None => runs.len().checked_sub(1),
        };
        let Some(current) = current else { return side };
        let run = &runs[current.saturating_add_signed(delta).min(runs.len() - 1)];
        Side { run: Some((run.session, run.stops.start)), ..side }
    }

    pub fn compared(&self) -> Option<Compared<'_>> {
        let comparison = self.comparison()?;
        let (a, b) = (self.path(comparison.a)?, self.path(comparison.b)?);
        let entries = align(&a.recording, &b.recording);
        Some(Compared { a, b, entries })
    }
}

/// `rose`, or `rose@3` for run #3.
fn parse_side(tab: &Tab, text: &str) -> Result<Side, String> {
    let (name, run) = match text.split_once('@') {
        Some((name, run)) => (name, Some(run.parse::<u32>().map_err(|_| format!("expected a run number: {text}"))?)),
        None => (text, None),
    };
    let set = parse_set(name)?;
    let Some(number) = run else { return Ok(Side { set, run: None }) };
    let found = tab.runs_of(set).into_iter().find(|r| r.number == Some(number));
    let run = found.ok_or_else(|| format!("{} recorded nothing in run #{number} of this tuitor", set_name(set)))?;
    Ok(Side { set, run: Some((run.session, run.stops.start)) })
}

impl App {
    /// `/compare [a] [b]`: each `set` or `set@run`. One set alone is compared with the active
    /// set; none, the default (see [`Tab::default_comparison`]).
    pub(crate) fn compare(&mut self, arg: Option<String>) -> Outcome {
        let tab = self.tab();
        let words: Vec<&str> = arg.as_deref().unwrap_or_default().split_whitespace().collect();
        let active = Side { set: tab.engine.breakpoints().active_set(), run: None };
        let comparison = match words.as_slice() {
            [] => tab.default_comparison(),
            [b] => Some(Comparison { a: active, b: parse_side(tab, b)? }),
            [a, b] => Some(Comparison { a: parse_side(tab, a)?, b: parse_side(tab, b)? }),
            _ => return Err("expected two sets at most: /compare rose teal, /compare rose@3 rose@5".into()),
        };
        let comparison =
            comparison.ok_or("nothing to compare yet: no set recorded a stop in two runs, nor two sets")?;
        let tab = self.tab_mut();
        tab.ui.comparison = Some(comparison);
        tab.ui.compare.pos = 0;
        tab.ui.view = View::Compare;
        tab.ui.zoom = false;
        tab.set_focus(Pane::Compare);
        self.compare_summary()
    }

    /// `[` and `]` in COMPARE: the side in the column under the cursor, or both, to the run
    /// before or after that recorded its set.
    pub(crate) fn compare_run(&mut self, delta: isize) -> Outcome {
        let tab = self.tab_mut();
        let mut comparison = tab.comparison().ok_or("nothing to compare yet")?;
        let col = tab.ui.compare_col;
        if col != 2 {
            comparison.a = tab.shifted(comparison.a, delta);
        }
        if col != 1 {
            comparison.b = tab.shifted(comparison.b, delta);
        }
        tab.ui.comparison = Some(comparison);
        tab.ui.compare.pos = 0;
        self.compare_summary()
    }

    /// `rose run #3 against teal run #3: 2 hits on both, 3 on rose's alone…`
    fn compare_summary(&self) -> Outcome {
        let tab = self.tab();
        let compared = tab.compared().ok_or("one of the sets has recorded nothing")?;
        let sessions = tab.engine.sessions();
        let both = compared.entries.iter().filter(|e| e.both()).count();
        let only = |a: bool| compared.entries.iter().filter(|e| !e.both() && e.a.is_some() == a).count();
        let side = |path: &Path<'_>| format!("{} {}", set_name(path.set), path.run.label(sessions));
        Ok(format!(
            "{} against {}: {both} hits on both, {} on the first alone, {} on the second alone",
            side(&compared.a),
            side(&compared.b),
            only(true),
            only(false)
        ))
    }

    /// `enter` in COMPARE: on the breakpoint, its line in CODE; on a side, that stop in HISTORY.
    pub(crate) fn compare_open(&mut self) -> Outcome {
        let tab = self.tab();
        let compared = tab.compared().ok_or("nothing to compare yet")?;
        let entry = compared.entries.get(tab.ui.compare.pos).ok_or("nothing selected")?;
        let (path, side) = match tab.ui.compare_col {
            1 => (&compared.a, entry.a),
            2 => (&compared.b, entry.b),
            _ => {
                let bp = tab.engine.breakpoints().get(entry.breakpoint).ok_or("that breakpoint is gone")?;
                let (file, line) = (bp.path.clone(), bp.line);
                let tab = self.tab_mut();
                tab.ui.view = View::Code;
                tab.browse(file, line);
                return Ok(String::new());
            }
        };
        let stop = side.ok_or("this path did not hit it")?;
        let (sid, number) = (path.run.session, stop.number);
        let tab = self.tab_mut();
        tab.ui.session = Some(sid);
        tab.view_stop(sid, number);
        tab.ui.view = View::History;
        tab.ui.zoom = false;
        tab.set_focus(Pane::History);
        Ok(format!("stop #{number}"))
    }
}
