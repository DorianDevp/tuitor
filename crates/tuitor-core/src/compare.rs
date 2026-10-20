//! Compares what two breakpoint sets recorded, each the stops at its breakpoints in one run,
//! taken as paths. A stop lines up with the stop at the same breakpoint on the other path
//! that is the same hit of it: the first with the first, the second with the second. The
//! others are one path's alone. Lined-up stops are compared value by value.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Local, TimeDelta};

use crate::breakpoints::BreakpointId;
use crate::snapshot::{Snapshot, Var};

/// The stops of one set in one run, oldest first.
pub struct Recording<'a> {
    pub stops: Vec<&'a Snapshot>,
    /// When the run began; times along the path count from there.
    pub start: DateTime<Local>,
}

/// A breakpoint's n-th hit, on one path or both.
#[derive(Debug, Clone, Copy)]
pub struct Entry<'a> {
    pub breakpoint: BreakpointId,
    /// From 1, per breakpoint and path.
    pub occurrence: u32,
    pub a: Option<&'a Snapshot>,
    pub b: Option<&'a Snapshot>,
}

impl Entry<'_> {
    pub fn both(&self) -> bool {
        self.a.is_some() && self.b.is_some()
    }
}

type Key = (BreakpointId, u32);

/// The entries of two paths, in the order of the paths. The hits both have keep the order
/// they have on `a`; a hit one path has alone comes after the furthest of them that came
/// before it on its own path, by its time since then. Two paths of the same run share no
/// hits when their sets share no breakpoints, and are then in the order things happened.
pub fn align<'a>(a: &Recording<'a>, b: &Recording<'a>) -> Vec<Entry<'a>> {
    let (on_a, on_b) = (hits(a), hits(b));
    let a_at: HashMap<Key, &Snapshot> = on_a.iter().copied().collect();
    let b_at: HashMap<Key, &Snapshot> = on_b.iter().copied().collect();
    let rank: HashMap<Key, usize> =
        on_a.iter().filter(|(key, _)| b_at.contains_key(key)).enumerate().map(|(i, (key, _))| (*key, i + 1)).collect();

    let mut placed: Vec<((usize, TimeDelta, bool), Entry<'a>)> = Vec::new();
    for (path, other, side_b) in [(&on_a, &b_at, false), (&on_b, &a_at, true)] {
        let start = if side_b { b.start } else { a.start };
        for ((key, snapshot), place) in path.iter().zip(places(path, &rank, start)) {
            let shared = other.get(key).copied();
            if shared.is_some() && side_b {
                continue; // placed with `a`
            }
            let (a, b) = if side_b { (shared, Some(*snapshot)) } else { (Some(*snapshot), shared) };
            let entry = Entry { breakpoint: key.0, occurrence: key.1, a, b };
            placed.push(((place.0, place.1, side_b), entry));
        }
    }
    placed.sort_by_key(|(place, _)| *place);
    placed.into_iter().map(|(_, entry)| entry).collect()
}

/// The stops of a path keyed by breakpoint and the how-manieth hit of it each is.
fn hits<'a>(recording: &Recording<'a>) -> Vec<(Key, &'a Snapshot)> {
    let mut seen: HashMap<BreakpointId, u32> = HashMap::new();
    let keyed = recording.stops.iter().filter_map(|stop| {
        let breakpoint = *stop.hit_breakpoints.first()?;
        let count = seen.entry(breakpoint).or_default();
        *count += 1;
        Some(((breakpoint, *count), *stop))
    });
    keyed.collect()
}

/// Where each stop of `path` goes: the furthest rank, along `a`, of the shared hits at or
/// before it (0 for none), and how long after that hit, or after `start`, it came.
fn places(path: &[(Key, &Snapshot)], rank: &HashMap<Key, usize>, start: DateTime<Local>) -> Vec<(usize, TimeDelta)> {
    let mut anchor = (0, start);
    let mut out = Vec::with_capacity(path.len());
    for (key, snapshot) in path {
        if let Some(rank) = rank.get(key).filter(|rank| **rank > anchor.0) {
            anchor = (*rank, snapshot.time);
        }
        out.push((anchor.0, snapshot.time - anchor.1));
    }
    out
}

/// How a value differs from one path to the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delta {
    Same(String),
    Differs(String, String),
    /// Only the second path has it.
    Appears(String),
    /// Only the first path has it.
    Disappears(String),
}

impl Delta {
    pub fn differs(&self) -> bool {
        !matches!(self, Delta::Same(_))
    }
}

/// What a stop read, as names and values: the watches that could be evaluated there, as
/// `watch ‹expression›`, then the variables of the top frame's arguments and locals, at
/// every level read, as `name.field.field`.
pub fn state(snapshot: &Snapshot) -> Vec<(String, String)> {
    let watches = snapshot.watches.iter().filter_map(|w| Some((format!("watch {}", w.expr), w.value.clone().ok()?)));
    let mut out: Vec<(String, String)> = watches.collect();
    let scopes = snapshot.scopes(0).unwrap_or_default().iter().filter(|s| s.default_open);
    for vars in scopes.filter_map(|s| s.vars.as_deref()) {
        flatten("", vars, &mut out);
    }
    out
}

fn flatten(prefix: &str, vars: &[Var], out: &mut Vec<(String, String)>) {
    for var in vars {
        let name = if prefix.is_empty() { var.name.clone() } else { format!("{prefix}.{}", var.name) };
        out.push((name.clone(), var.value.clone()));
        if let Some(children) = &var.children {
            flatten(&name, children, out);
        }
    }
}

/// Every value either stop read, in the order of the first then of what only the second has.
/// A name read twice at one stop (a shadowed local) counts once, the first time.
pub fn differences(a: &Snapshot, b: &Snapshot) -> Vec<(String, Delta)> {
    let (a, b) = (state(a), state(b));
    let mut b_values: HashMap<&str, &str> = HashMap::new();
    for (name, value) in &b {
        b_values.entry(name).or_insert(value);
    }
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = Vec::new();
    for (name, value) in &a {
        if !seen.insert(name) {
            continue;
        }
        let delta = match b_values.get(name.as_str()) {
            Some(other) if *other == value => Delta::Same(value.clone()),
            Some(other) => Delta::Differs(value.clone(), (*other).to_owned()),
            None => Delta::Disappears(value.clone()),
        };
        out.push((name.clone(), delta));
    }
    for (name, value) in &b {
        if seen.insert(name) {
            out.push((name.clone(), Delta::Appears(value.clone())));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{Frame, ScopeVars, WatchValue};

    fn var(name: &str, value: &str) -> Var {
        Var {
            name: name.into(),
            value: value.into(),
            type_name: None,
            evaluate_name: None,
            reference: 0,
            child_count: None,
            children: None,
        }
    }

    /// Stop `number` at breakpoint `bp`, `ms` after the epoch, with `vars`.
    fn stop(number: u32, bp: BreakpointId, ms: i64, vars: Vec<Var>) -> Snapshot {
        let frame = Frame {
            id: 1,
            name: "orderTotal".into(),
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
            time: epoch() + TimeDelta::milliseconds(ms),
            thread_id: 1,
            reason: "breakpoint".into(),
            description: None,
            hit_breakpoints: vec![bp],
            sets: vec![crate::breakpoints::SetId::FIRST_OF_OLD],
            silent: false,
            frames: vec![frame],
            watches: vec![],
            live: false,
        }
    }

    fn epoch() -> DateTime<Local> {
        DateTime::from_timestamp(1_800_000_000, 0).expect("time").with_timezone(&Local)
    }

    fn recording(stops: &[Snapshot], start_ms: i64) -> Recording<'_> {
        Recording { stops: stops.iter().collect(), start: epoch() + TimeDelta::milliseconds(start_ms) }
    }

    /// Entries as (breakpoint, occurrence, number on a, number on b).
    fn shape(entries: &[Entry<'_>]) -> Vec<(BreakpointId, u32, Option<u32>, Option<u32>)> {
        entries.iter().map(|e| (e.breakpoint, e.occurrence, e.a.map(|s| s.number), e.b.map(|s| s.number))).collect()
    }

    #[test]
    fn hits_of_the_same_breakpoint_line_up_by_occurrence() {
        // Two runs: the second hits 3 once more, and 7 only once.
        let first = [stop(1, 3, 10, vec![]), stop(2, 7, 20, vec![]), stop(3, 3, 30, vec![]), stop(4, 7, 40, vec![])];
        let second =
            [stop(1, 3, 110, vec![]), stop(2, 3, 120, vec![]), stop(3, 7, 130, vec![]), stop(4, 3, 140, vec![])];
        let entries = align(&recording(&first, 0), &recording(&second, 100));
        assert_eq!(
            shape(&entries),
            [
                (3, 1, Some(1), Some(1)),
                (7, 1, Some(2), Some(3)),
                (3, 2, Some(3), Some(2)),
                (7, 2, Some(4), None),
                (3, 3, None, Some(4)),
            ]
        );
        assert!(entries[0].both() && !entries[3].both());
    }

    #[test]
    fn hits_one_path_has_alone_follow_the_shared_hit_before_them_by_time() {
        // Shared: 1 (first), then 9; a has 2 after 1, b has 5 after 1 and 6 after 9.
        let a = [stop(1, 1, 0, vec![]), stop(2, 2, 50, vec![]), stop(3, 9, 100, vec![])];
        let b =
            [stop(1, 1, 1000, vec![]), stop(2, 5, 1010, vec![]), stop(3, 9, 1500, vec![]), stop(4, 6, 1600, vec![])];
        let entries = align(&recording(&a, 0), &recording(&b, 1000));
        assert_eq!(
            shape(&entries),
            [
                (1, 1, Some(1), Some(1)),
                (5, 1, None, Some(2)),
                (2, 1, Some(2), None),
                (9, 1, Some(3), Some(3)),
                (6, 1, None, Some(4))
            ]
        );
    }

    #[test]
    fn two_sets_of_one_run_share_no_hits_and_go_in_the_order_they_happened() {
        let run = [stop(1, 1, 0, vec![]), stop(2, 4, 10, vec![]), stop(3, 4, 20, vec![]), stop(4, 1, 30, vec![])];
        let rose = Recording { stops: vec![&run[0], &run[3]], start: epoch() };
        let teal = Recording { stops: vec![&run[1], &run[2]], start: epoch() };
        let entries = align(&rose, &teal);
        assert_eq!(
            shape(&entries),
            [(1, 1, Some(1), None), (4, 1, None, Some(2)), (4, 2, None, Some(3)), (1, 2, Some(4), None)]
        );
    }

    #[test]
    fn values_differ_appear_and_disappear() {
        let mut order = var("$order", "Order");
        order.children = Some(vec![var("status", "Pending"), var("id", "1001")]);
        let mut a = stop(1, 1, 0, vec![var("$total", "8900"), order.clone(), var("$i", "0")]);
        a.watches = vec![
            WatchValue { expr: "$total * 2".into(), value: Ok("17800".into()) },
            WatchValue { expr: "$nope".into(), value: Err("undefined".into()) },
        ];
        order.children = Some(vec![var("status", "Assigned"), var("id", "1001")]);
        let b = stop(1, 1, 0, vec![var("$total", "8900"), order, var("$owner", "null")]);
        let deltas = differences(&a, &b);
        assert_eq!(
            deltas,
            [
                ("watch $total * 2".to_owned(), Delta::Disappears("17800".into())),
                ("$total".to_owned(), Delta::Same("8900".into())),
                ("$order".to_owned(), Delta::Same("Order".into())),
                ("$order.status".to_owned(), Delta::Differs("Pending".into(), "Assigned".into())),
                ("$order.id".to_owned(), Delta::Same("1001".into())),
                ("$i".to_owned(), Delta::Disappears("0".into())),
                ("$owner".to_owned(), Delta::Appears("null".into())),
            ]
        );
        assert_eq!(deltas.iter().filter(|(_, d)| d.differs()).count(), 4);
    }
}
