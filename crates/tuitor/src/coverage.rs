//! Which breakpoints the runs reached, and from where: a test or the program as it runs for
//! real (a request, a command, `main`). tuitor runs neither; it tells a run by its stacks: a
//! run with a frame in a test file (`*Test.php`, `*.spec.ts`, `*_test.go`, a `tests/`
//! directory) came from that test, any other from the program. A breakpoint the program
//! reaches and no test does is a place the tests do not cover, by the calls that were made,
//! whatever the coverage of lines says.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use tuitor_core::breakpoints::BreakpointId;
use tuitor_core::snapshot::{Frame, Snapshot};

use crate::app::{App, Outcome, Pane};
use crate::command::View;
use crate::tab::Tab;

/// The runs that reached a breakpoint, by where they came from.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Reach {
    /// Runs of the program, not of a test.
    pub runtime: usize,
    /// The tests whose runs reached it, by their frames' names.
    pub tests: BTreeSet<String>,
}

impl Reach {
    /// The program reaches it and no test does.
    pub fn runtime_only(&self) -> bool {
        self.runtime > 0 && self.tests.is_empty()
    }
}

/// Where a run came from: the test whose frame its stacks have, outermost first.
fn test_of(stops: &[Snapshot], root: &Path) -> Option<String> {
    stops.iter().find_map(|stop| stop.frames.iter().rev().find(|f| is_test(f, root)).map(|f| f.name.clone()))
}

fn is_test(frame: &Frame, root: &Path) -> bool {
    let Some(path) = frame.path.as_deref().filter(|_| !frame.library) else { return false };
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let relative = path.strip_prefix(root).unwrap_or(path);
    let in_tests =
        relative.parent().is_some_and(|dir| dir.iter().any(|d| ["tests", "test", "__tests__"].iter().any(|t| d == *t)));
    file.ends_with("_test.go") || [".spec.", ".test.", "Test.php"].iter().any(|p| file.contains(p)) || in_tests
}

impl Tab {
    /// For each breakpoint the runs in memory hit, those of this tuitor and those read back
    /// from disk, the runs that reached it.
    pub fn reach(&self) -> HashMap<BreakpointId, Reach> {
        let root = self.engine.workspace();
        let sessions = self.engine.sessions();
        let mut reach: HashMap<BreakpointId, Reach> = HashMap::new();
        for run in self.flows.in_memory(sessions) {
            let Some(session) = sessions.iter().find(|s| s.id == run.session) else { continue };
            let stops = &session.history[run.stops.clone()];
            let test = test_of(stops, root);
            let hit: BTreeSet<BreakpointId> = stops.iter().flat_map(|s| s.hit_breakpoints.iter().copied()).collect();
            for id in hit {
                let entry = reach.entry(id).or_default();
                match &test {
                    Some(name) => {
                        entry.tests.insert(name.clone());
                    }
                    None => entry.runtime += 1,
                }
            }
        }
        reach
    }
}

impl App {
    /// `/coverage`: which breakpoints the program's runs reached and the tests' did not, and
    /// the other way round, over the runs in memory.
    pub(crate) fn coverage(&mut self) -> Outcome {
        let tab = self.tab();
        let reach = tab.reach();
        if reach.is_empty() {
            return Err(
                "no run hit a breakpoint yet: run the program and its tests with breakpoints on the path".into()
            );
        }
        let bps = tab.engine.breakpoints();
        let place = |id: &BreakpointId| {
            bps.get(*id)
                .map(|b| format!("{}:{}", b.path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(), b.line))
        };
        let mut runtime_only: Vec<String> =
            reach.iter().filter(|(_, r)| r.runtime_only()).filter_map(|(id, _)| place(id)).collect();
        runtime_only.sort();
        let both = reach.values().filter(|r| r.runtime > 0 && !r.tests.is_empty()).count();
        let tests_only = reach.values().filter(|r| r.runtime == 0).count();
        let tab = self.tab_mut();
        tab.ui.view = View::Project;
        tab.ui.zoom = false;
        tab.set_focus(Pane::Breakpoints);
        let gap = if runtime_only.is_empty() {
            "none by the program alone".to_owned()
        } else {
            format!("{} by the program alone: {}", runtime_only.len(), runtime_only.join(", "))
        };
        Ok(format!(
            "{} breakpoints reached: {both} by the program and tests, {gap}; {tests_only} by tests alone",
            reach.len()
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn frame(name: &str, path: &str, library: bool) -> Frame {
        serde_json::from_value(serde_json::json!({
            "id": 0, "name": name, "path": path, "source_name": null, "source_reference": null,
            "line": 1, "column": 0, "library": library, "scopes": null
        }))
        .expect("frame")
    }

    #[test]
    fn a_frame_in_a_test_file_or_directory_is_a_test() {
        let root = PathBuf::from("/w");
        assert!(is_test(&frame("MoneyTest->testBulkRebate", "/w/tests/MoneyTest.php", false), &root));
        assert!(is_test(&frame("walk", "/w/src/lib/cart.spec.ts", false), &root));
        assert!(is_test(&frame("TestTotal", "/w/orders_test.go", false), &root));
        assert!(!is_test(&frame("show", "/w/src/Controller/OrderController.php", false), &root));
        // PHPUnit's own frames are in vendor, not the project's tests.
        assert!(!is_test(&frame("run", "/w/vendor/phpunit/phpunit/src/Framework/TestCase.php", true), &root));
    }
}
