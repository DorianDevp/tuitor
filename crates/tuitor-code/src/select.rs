//! Which lines of a function on a path get a breakpoint: few enough to step through, the
//! ones that say how execution got to the next function and with what state.

/// Why a line is worth a breakpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// The line the path leads to.
    Target,
    /// It calls the next function on the path.
    Calls(String),
    /// The first line of a function's body.
    Entry,
    /// It writes this variable or field, last before the path goes on.
    Writes(String),
    /// It calls this other function of the project.
    Uses(String),
}

impl Why {
    /// Which reason a line shows when it has several.
    fn rank(&self) -> u8 {
        match self {
            Why::Target => 0,
            Why::Calls(_) => 1,
            Why::Entry => 2,
            Why::Writes(_) => 3,
            Why::Uses(_) => 4,
        }
    }
}

/// What is known of a function's lines (0-based).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Facts {
    /// The first line of its body; none for a script's top level.
    pub entry: Option<u32>,
    /// Lines that write a variable or a field, and its name.
    pub writes: Vec<(u32, String)>,
    /// Lines that call a function of the project, and its name.
    pub uses: Vec<(u32, String)>,
}

/// The lines of one function on a path, in order:
/// - its entry;
/// - `calls`, where it calls the next function on the path (none in the last function);
/// - `target`, the line the path leads to (only in the last function);
/// - between the entry and the last call (or the target), the last write of each variable
///   and the last call of each other function of the project, at most `room` of them,
///   those nearest the call first: they made the state the next function gets.
pub fn choose(facts: &Facts, calls: &[(u32, String)], target: Option<u32>, room: usize) -> Vec<(u32, Why)> {
    let until = calls.iter().map(|(line, _)| *line).chain(target).max().unwrap_or(0);
    let from = facts.entry.unwrap_or(0);
    let inside = |line: u32| (from..until).contains(&line);
    let mut state: Vec<(u32, Why)> = Vec::new();
    for (line, name) in facts.writes.iter().rev().filter(|(line, _)| inside(*line)) {
        if !state.iter().any(|(_, why)| *why == Why::Writes(name.clone())) {
            state.push((*line, Why::Writes(name.clone())));
        }
    }
    for (line, name) in facts.uses.iter().rev().filter(|(line, _)| inside(*line)) {
        let known = state.iter().any(|(at, why)| *at == *line || *why == Why::Uses(name.clone()));
        if !known && !calls.iter().any(|(at, _)| at == line) {
            state.push((*line, Why::Uses(name.clone())));
        }
    }
    state.sort_by_key(|(line, why)| (std::cmp::Reverse(*line), why.rank()));
    state.dedup_by_key(|(line, _)| *line);
    state.truncate(room);

    let mut lines: Vec<(u32, Why)> = facts.entry.map(|entry| (entry, Why::Entry)).into_iter().collect();
    lines.extend(calls.iter().map(|(line, name)| (*line, Why::Calls(name.clone()))));
    lines.extend(target.map(|line| (line, Why::Target)));
    lines.extend(state);
    lines.sort_by_key(|(line, why)| (*line, why.rank()));
    lines.dedup_by_key(|(line, _)| *line);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(entry: u32, writes: &[(u32, &str)], uses: &[(u32, &str)]) -> Facts {
        let owned = |list: &[(u32, &str)]| list.iter().map(|(line, name)| (*line, (*name).to_owned())).collect();
        Facts { entry: Some(entry), writes: owned(writes), uses: owned(uses) }
    }

    #[test]
    fn a_caller_keeps_its_entry_the_state_before_the_call_and_the_call() {
        // OrderService::totalFor: `$order = $this->orders->find($id)` is its first line.
        let total_for = facts(
            21,
            &[(21, "$order"), (22, "$total"), (24, "$total"), (27, "$total"), (29, "total")],
            &[(21, "find"), (24, "lineTotal"), (30, "dispatch")],
        );
        let chosen = choose(&total_for, &[(24, "lineTotal".into())], None, 4);
        assert_eq!(
            chosen,
            [(21, Why::Entry), (22, Why::Writes("$total".into())), (24, Why::Calls("lineTotal".into()))]
        );
    }

    #[test]
    fn the_last_function_ends_at_the_target() {
        let line_total = facts(12, &[(12, "$amount"), (14, "$amount")], &[]);
        assert_eq!(choose(&line_total, &[], Some(14), 4), [(12, Why::Entry), (14, Why::Target)]);
        // The cursor on the declaration: the entry is the target's way in.
        assert_eq!(choose(&line_total, &[], Some(10), 4), [(10, Why::Target), (12, Why::Entry)]);
    }

    #[test]
    fn state_is_kept_nearest_the_call_within_room() {
        let busy = facts(
            1,
            &[(2, "a"), (3, "b"), (4, "c"), (5, "d"), (6, "a"), (7, "e")],
            &[(8, "audit"), (9, "audit"), (11, "after")],
        );
        let chosen = choose(&busy, &[(10, "next".into())], None, 3);
        assert_eq!(
            chosen,
            [
                (1, Why::Entry),
                (6, Why::Writes("a".into())),
                (7, Why::Writes("e".into())),
                (9, Why::Uses("audit".into()))
            ]
            .into_iter()
            .chain([(10, Why::Calls("next".into()))])
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_script_has_no_entry_and_several_calls_are_all_kept() {
        let script = Facts { entry: None, writes: vec![(90, "$orders".into()), (92, "$revenue".into())], uses: vec![] };
        let calls = [(94, "orderTotal".into()), (99, "orderTotal".into())];
        let chosen = choose(&script, &calls, None, 4);
        let lines: Vec<u32> = chosen.iter().map(|(line, _)| *line).collect();
        assert_eq!(lines, [90, 92, 94, 99]);
    }
}
