//! The TIMELINE of HISTORY: for each value HISTORY shows, the stops at which it changed,
//! with what it was before. It says nothing about whether a change is wrong.

use std::collections::HashMap;

use tuitor_core::snapshot::Snapshot;

/// A value as HISTORY shows it at a stop: the watch of that name, else the top-level
/// variable, on one line.
pub fn cell(snapshot: &Snapshot, column: &str) -> Option<String> {
    let value = match snapshot.watch(column) {
        Some(result) => result.clone().unwrap_or_else(|_| "err".into()),
        None => snapshot.local_value(column)?.to_owned(),
    };
    Some(value.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// One value's changes, oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub name: String,
    pub changes: Vec<Change>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The stop it changed at.
    pub number: u32,
    pub was: String,
    pub now: String,
}

/// The values that changed at least once over `history`: the watches, then the top-level
/// variables in the order they first appear. A value is compared with what it was at the
/// last stop in the same function, so that another function's variable of the same name, a
/// stop where it is out of scope, or a watch that reads another variable in another function
/// (watches are evaluated in the top frame), is no change.
pub fn tracks(history: &[Snapshot], watches: &[String]) -> Vec<Track> {
    let mut names: Vec<String> = watches.to_vec();
    for snapshot in history {
        let scopes = snapshot.scopes(0).unwrap_or_default().iter().filter(|s| s.default_open);
        for var in scopes.flat_map(|s| s.vars.iter().flatten()) {
            if !names.contains(&var.name) {
                names.push(var.name.clone());
            }
        }
    }
    names
        .into_iter()
        .map(|name| {
            let watch = watches.contains(&name);
            let changes = changes(history, &name, watch);
            Track { name, changes }
        })
        .filter(|track| !track.changes.is_empty())
        .collect()
}

fn changes(history: &[Snapshot], name: &str, watch: bool) -> Vec<Change> {
    let mut last: HashMap<&str, String> = HashMap::new();
    let mut out = Vec::new();
    for snapshot in history {
        // A watch that could not be evaluated there has no value there.
        let value = if watch { snapshot.watch(name).and_then(|v| v.clone().ok()) } else { cell(snapshot, name) };
        let Some(now) = value.map(|v| v.split_whitespace().collect::<Vec<_>>().join(" ")) else { continue };
        let function = snapshot.top().map_or("", |f| f.name.as_str());
        if let Some(was) = last.insert(function, now.clone())
            && was != now
        {
            out.push(Change { number: snapshot.number, was, now });
        }
    }
    out
}

impl Track {
    /// The change at stop `number`, if it changed there.
    pub fn at(&self, number: u32) -> Option<&Change> {
        self.changes.iter().find(|c| c.number == number)
    }

    /// The change after stop `number`, or with `forward` false the one before it.
    pub fn next(&self, number: u32, forward: bool) -> Option<&Change> {
        if forward {
            self.changes.iter().find(|c| c.number > number)
        } else {
            self.changes.iter().rev().find(|c| c.number < number)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuitor_core::snapshot::{Frame, ScopeVars, Var, WatchValue};

    /// Stop `number` in `function` with variables `vars` (name, value).
    fn stop(number: u32, function: &str, vars: &[(&str, &str)]) -> Snapshot {
        let vars = vars
            .iter()
            .map(|(name, value)| Var {
                name: (*name).into(),
                value: (*value).into(),
                type_name: None,
                evaluate_name: None,
                reference: 0,
                child_count: None,
                children: None,
            })
            .collect();
        let frame = Frame {
            id: 1,
            name: function.into(),
            path: None,
            source_name: None,
            source_reference: None,
            line: 1,
            column: 1,
            library: false,
            scopes: Some(vec![ScopeVars {
                name: "Locals".into(),
                default_open: true,
                reference: 1,
                expensive: false,
                vars: Some(vars),
            }]),
        };
        Snapshot {
            number,
            time: chrono::Local::now(),
            thread_id: 1,
            reason: "breakpoint".into(),
            description: None,
            hit_breakpoints: vec![],
            set: None,
            silent: false,
            frames: vec![frame],
            watches: vec![],
            live: false,
        }
    }

    fn numbers(track: &Track) -> Vec<u32> {
        track.changes.iter().map(|c| c.number).collect()
    }

    #[test]
    fn a_variable_changes_against_its_last_value_in_the_same_function() {
        let history = [
            stop(1, "orderTotal", &[("total", "0"), ("i", "0")]),
            // Another function's `total`, and a stop where it is not in scope: no change.
            stop(2, "lineTotal", &[("total", "8900")]),
            stop(3, "discount", &[("amount", "16770")]),
            stop(4, "orderTotal", &[("total", "8900"), ("i", "0")]),
            stop(5, "orderTotal", &[("total", "13800"), ("i", "1")]),
        ];
        let tracks = tracks(&history, &[]);
        let names: Vec<&str> = tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["total", "i"], "amount never changed");
        assert_eq!(numbers(&tracks[0]), [4, 5]);
        assert_eq!(tracks[0].changes[0], Change { number: 4, was: "0".into(), now: "8900".into() });
        assert_eq!(numbers(&tracks[1]), [5]);
    }

    #[test]
    fn a_watch_changes_where_it_was_evaluated_in_the_same_function_and_comes_first() {
        let mut history = [
            stop(1, "a", &[("x", "1")]),
            stop(2, "b", &[("x", "1")]),
            stop(3, "a", &[("x", "2")]),
            stop(4, "a", &[("x", "2")]),
        ];
        let values = [Ok("10"), Ok("99"), Err("not defined"), Ok("12")];
        for (snapshot, value) in history.iter_mut().zip(values) {
            let value = value.map(str::to_owned).map_err(str::to_owned);
            snapshot.watches = vec![WatchValue { expr: "total".into(), value }];
        }
        let tracks = tracks(&history, &["total".to_owned()]);
        assert_eq!(tracks.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["total", "x"]);
        assert_eq!(numbers(&tracks[0]), [4], "b's value and the error are no change");
        assert_eq!((tracks[1].next(1, true).map(|c| c.number), tracks[1].next(3, false)), (Some(3), None));
        assert!(tracks[1].at(3).is_some() && tracks[1].at(2).is_none());
    }
}
