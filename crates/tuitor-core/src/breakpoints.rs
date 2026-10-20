//! Breakpoints live in the engine, not in a session: they survive restarts and are
//! sent to every session whose adapter handles the file's extension.
//!
//! Each breakpoint belongs to a set, and one set is active. Every enabled breakpoint goes
//! to the adapter whatever its set: a stop at the active set's breakpoints waits for the
//! user, one at another set's is read and goes on by itself, so switching sets needs no
//! word to the adapter and works while the program runs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use tuitor_dap::types::{self as dap, SourceBreakpoint};

use crate::session::SessionId;

pub type BreakpointId = u32;

/// A set has no name: it is known by its colour, one of the UI's palette of [`SET_COLORS`],
/// and this is the colour's place in it.
pub type SetId = u8;

/// How many sets there can be: as many as there are colours that stay apart on screen.
pub const SET_COLORS: SetId = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Breakpoint {
    pub id: BreakpointId,
    pub path: PathBuf,
    pub line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    /// Makes this a logpoint: the adapter prints the message instead of stopping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_message: Option<String>,
    /// Which hits stop, in the adapter's terms (`>= 3`, `% 10`; gdb: how many to pass over).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_condition: Option<String>,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    /// Breakpoints saved before there were sets are in the first.
    #[serde(default)]
    pub set: SetId,
    #[serde(skip)]
    pub hits: u32,
    #[serde(skip)]
    pub bindings: HashMap<SessionId, Binding>,
}

fn enabled_default() -> bool {
    true
}

/// What one session's adapter made of a breakpoint.
#[derive(Debug, Clone)]
pub struct Binding {
    pub adapter_id: Option<i64>,
    pub verified: bool,
    pub message: Option<String>,
}

impl Breakpoint {
    pub fn is_logpoint(&self) -> bool {
        self.log_message.is_some()
    }

    /// Verified by at least one session; `None` when no session has seen it yet.
    pub fn verified(&self) -> Option<bool> {
        if self.bindings.is_empty() { None } else { Some(self.bindings.values().any(|b| b.verified)) }
    }

    pub fn adapter_message(&self) -> Option<&str> {
        self.bindings.values().find_map(|b| b.message.as_deref())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    Added(BreakpointId),
    Removed(BreakpointId),
}

#[derive(Debug)]
pub struct Breakpoints {
    /// Sorted by (path, line).
    items: Vec<Breakpoint>,
    next_id: BreakpointId,
    /// The sets there are, in the order they were made; never empty.
    sets: Vec<SetId>,
    /// The set whose breakpoints stop the program.
    active: SetId,
}

impl Default for Breakpoints {
    fn default() -> Self {
        Self { items: Vec::new(), next_id: 0, sets: vec![0], active: 0 }
    }
}

impl Breakpoints {
    /// Breakpoints and sets as saved. Saved before there were sets, they are all in one, the
    /// first colour's; a breakpoint whose set is gone from the file goes to the first set.
    pub fn from_saved(mut items: Vec<Breakpoint>, saved_sets: &[SetId], active: Option<SetId>) -> Self {
        let mut sets: Vec<SetId> = Vec::new();
        for set in saved_sets.iter().copied().filter(|s| *s < SET_COLORS) {
            if !sets.contains(&set) {
                sets.push(set);
            }
        }
        let first = *sets.first().unwrap_or(&0);
        if sets.is_empty() {
            sets.push(first);
        }
        for bp in items.iter_mut().filter(|b| !sets.contains(&b.set)) {
            bp.set = first;
        }
        items.sort_by(|a, b| (&a.path, a.line).cmp(&(&b.path, b.line)));
        let next_id = items.iter().map(|b| b.id).max().unwrap_or(0);
        let active = active.filter(|a| sets.contains(a)).unwrap_or(first);
        Self { items, next_id, sets, active }
    }

    pub fn saved(&self) -> &[Breakpoint] {
        &self.items
    }

    pub fn iter(&self) -> impl Iterator<Item = &Breakpoint> {
        self.items.iter()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, id: BreakpointId) -> Option<&Breakpoint> {
        self.items.iter().find(|b| b.id == id)
    }

    pub fn at(&self, path: &Path, line: u32) -> Option<&Breakpoint> {
        self.items.iter().find(|b| b.path == path && b.line == line)
    }

    pub fn in_file<'a>(&'a self, path: &'a Path) -> impl Iterator<Item = &'a Breakpoint> + 'a {
        self.items.iter().filter(move |b| b.path == path)
    }

    pub fn files(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = self.items.iter().map(|b| b.path.clone()).collect();
        files.dedup();
        files
    }

    pub fn toggle(&mut self, path: PathBuf, line: u32) -> Toggle {
        if let Some(existing) = self.at(&path, line).map(|b| b.id) {
            self.remove(existing);
            Toggle::Removed(existing)
        } else {
            Toggle::Added(self.add(path, line))
        }
    }

    pub fn add(&mut self, path: PathBuf, line: u32) -> BreakpointId {
        self.next_id += 1;
        let bp = Breakpoint {
            id: self.next_id,
            path,
            line,
            condition: None,
            log_message: None,
            hit_condition: None,
            enabled: true,
            set: self.active,
            hits: 0,
            bindings: HashMap::new(),
        };
        let at = self.items.partition_point(|b| (&b.path, b.line) < (&bp.path, bp.line));
        self.items.insert(at, bp);
        self.next_id
    }

    pub fn remove(&mut self, id: BreakpointId) -> Option<Breakpoint> {
        let index = self.items.iter().position(|b| b.id == id)?;
        Some(self.items.remove(index))
    }

    /// Removes every breakpoint; the sets stay.
    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn update(&mut self, id: BreakpointId, f: impl FnOnce(&mut Breakpoint)) -> Option<&Breakpoint> {
        let bp = self.items.iter_mut().find(|b| b.id == id)?;
        f(bp);
        Some(bp)
    }

    /// The enabled breakpoints of a file as DAP wants them, plus their ids in the same order.
    /// Hit conditions go only to an adapter that counts hits (`hits`): one that does not may
    /// refuse the breakpoint, or take the count for a condition.
    pub fn for_adapter(&self, path: &Path, hits: bool) -> (Vec<BreakpointId>, Vec<SourceBreakpoint>) {
        self.in_file(path)
            .filter(|b| b.enabled)
            .map(|b| {
                let source = SourceBreakpoint {
                    line: i64::from(b.line),
                    condition: b.condition.clone(),
                    hit_condition: b.hit_condition.clone().filter(|_| hits),
                    log_message: b.log_message.clone(),
                };
                (b.id, source)
            })
            .unzip()
    }

    /// Records the adapter's answer to `setBreakpoints` (same order as sent).
    pub fn bind(&mut self, session: SessionId, path: &Path, ids: &[BreakpointId], results: &[dap::Breakpoint]) {
        for bp in self.items.iter_mut().filter(|b| b.path == path) {
            bp.bindings.remove(&session);
        }
        for (id, result) in ids.iter().zip(results) {
            if let Some(bp) = self.items.iter_mut().find(|b| b.id == *id) {
                bp.bindings.insert(
                    session,
                    Binding { adapter_id: result.id, verified: result.verified, message: result.message.clone() },
                );
            }
        }
    }

    /// Applies a `breakpoint` event (the adapter changed its mind about verification).
    pub fn rebind(&mut self, session: SessionId, result: &dap::Breakpoint) {
        let Some(adapter_id) = result.id else { return };
        for bp in &mut self.items {
            if let Some(binding) = bp.bindings.get_mut(&session).filter(|b| b.adapter_id == Some(adapter_id)) {
                binding.verified = result.verified;
                binding.message.clone_from(&result.message);
            }
        }
    }

    pub fn unbind_session(&mut self, session: SessionId) {
        for bp in &mut self.items {
            bp.bindings.remove(&session);
        }
    }

    pub fn by_adapter_id(&self, session: SessionId, adapter_id: i64) -> Option<BreakpointId> {
        self.items
            .iter()
            .find(|b| b.bindings.get(&session).is_some_and(|x| x.adapter_id == Some(adapter_id)))
            .map(|b| b.id)
    }

    pub fn record_hit(&mut self, id: BreakpointId) {
        if let Some(bp) = self.items.iter_mut().find(|b| b.id == id) {
            bp.hits += 1;
        }
    }

    // ---- sets -----------------------------------------------------------------------

    pub fn sets(&self) -> &[SetId] {
        &self.sets
    }

    pub fn active_set(&self) -> SetId {
        self.active
    }

    /// Makes `set` the one whose breakpoints stop the program.
    pub fn activate(&mut self, set: SetId) -> Result<()> {
        if !self.sets.contains(&set) {
            bail!("there is no such set");
        }
        self.active = set;
        Ok(())
    }

    /// The set `delta` places from `set`, in the order the sets were made, around the end.
    pub fn neighbour(&self, set: SetId, delta: isize) -> SetId {
        let len = self.sets.len() as isize;
        let index = self.sets.iter().position(|s| *s == set).unwrap_or(0) as isize;
        self.sets[(index + delta).rem_euclid(len) as usize]
    }

    /// A set in the first colour no set has, made the active one, so that the breakpoints
    /// added next go to it.
    pub fn new_set(&mut self) -> Result<SetId> {
        let Some(set) = (0..SET_COLORS).find(|s| !self.sets.contains(s)) else {
            bail!("there are {SET_COLORS} sets, one for each colour; /set delete makes room");
        };
        self.sets.push(set);
        self.active = set;
        Ok(set)
    }

    pub fn move_to(&mut self, id: BreakpointId, set: SetId) -> Result<()> {
        if !self.sets.contains(&set) {
            bail!("there is no such set");
        }
        let bp = self.items.iter_mut().find(|b| b.id == id).ok_or_else(|| anyhow::anyhow!("no breakpoint #{id}"))?;
        bp.set = set;
        Ok(())
    }

    pub fn in_set(&self, set: SetId) -> impl Iterator<Item = &Breakpoint> {
        self.items.iter().filter(move |b| b.set == set)
    }

    /// Deletes `set`. Its breakpoints go to `to`, or with it when `to` is `None`; those are
    /// returned. The last set cannot go. When the active set goes, `to` or the first set
    /// left becomes active.
    pub fn delete_set(&mut self, set: SetId, to: Option<SetId>) -> Result<Vec<Breakpoint>> {
        if !self.sets.contains(&set) {
            bail!("there is no such set");
        }
        if self.sets.len() == 1 {
            bail!("the last set cannot go: every breakpoint is in one");
        }
        if let Some(to) = to
            && (to == set || !self.sets.contains(&to))
        {
            bail!("its breakpoints can go only to another set there is");
        }
        let removed = match to {
            Some(to) => {
                for bp in self.items.iter_mut().filter(|b| b.set == set) {
                    bp.set = to;
                }
                Vec::new()
            }
            None => {
                let (gone, kept) = std::mem::take(&mut self.items).into_iter().partition(|b| b.set == set);
                self.items = kept;
                gone
            }
        };
        self.sets.retain(|s| *s != set);
        if self.active == set {
            self.active = to.unwrap_or(self.sets[0]);
        }
        Ok(removed)
    }

    /// Whether a stop at breakpoints `hits` goes on by itself: they are all of sets other
    /// than the active one. A stop at none, or at one that is gone, waits.
    pub fn passes(&self, hits: &[BreakpointId]) -> bool {
        !hits.is_empty() && hits.iter().all(|id| self.get(*id).is_some_and(|b| b.set != self.active))
    }

    /// Whether any enabled breakpoint is of a set other than the active one, so that a stop
    /// at a breakpoint the adapter does not name may be one to go on from.
    pub fn any_passing(&self) -> bool {
        self.items.iter().any(|b| b.enabled && b.set != self.active)
    }

    /// The set of the first of `hits` that is still there.
    pub fn set_of(&self, hits: &[BreakpointId]) -> Option<SetId> {
        hits.iter().find_map(|id| self.get(*id)).map(|b| b.set)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_adds_then_removes_and_keeps_order() {
        let mut bps = Breakpoints::default();
        let a = PathBuf::from("/src/b.php");
        let b = PathBuf::from("/src/a.php");
        assert_eq!(bps.toggle(a.clone(), 20), Toggle::Added(1));
        assert_eq!(bps.toggle(b.clone(), 5), Toggle::Added(2));
        assert_eq!(bps.toggle(a.clone(), 3), Toggle::Added(3));
        let order: Vec<_> = bps.iter().map(|b| (b.path.clone(), b.line)).collect();
        assert_eq!(order, [(b.clone(), 5), (a.clone(), 3), (a.clone(), 20)]);
        assert_eq!(bps.toggle(a.clone(), 20), Toggle::Removed(1));
        assert_eq!(bps.len(), 2);
    }

    #[test]
    fn adapter_payload_skips_disabled_and_binds_in_order() {
        let mut bps = Breakpoints::default();
        let file = PathBuf::from("/src/OrderService.php");
        let first = bps.add(file.clone(), 136);
        let second = bps.add(file.clone(), 141);
        bps.update(first, |b| b.enabled = false);
        let (ids, payload) = bps.for_adapter(&file, true);
        assert_eq!(ids, [second]);
        assert_eq!(payload[0].line, 141);

        let reply = dap::Breakpoint { id: Some(7), verified: true, ..Default::default() };
        bps.bind(1, &file, &ids, &[reply]);
        assert_eq!(bps.by_adapter_id(1, 7), Some(second));
        assert_eq!(bps.get(second).and_then(Breakpoint::verified), Some(true));
        assert_eq!(bps.get(first).and_then(Breakpoint::verified), None);
    }

    #[test]
    fn hit_conditions_are_kept_and_sent_only_where_hits_count() {
        let mut bps = Breakpoints::default();
        let file = PathBuf::from("/src/orders.php");
        let id = bps.add(file.clone(), 68);
        bps.update(id, |b| b.hit_condition = Some(">= 3".into()));
        assert_eq!(bps.for_adapter(&file, true).1[0].hit_condition.as_deref(), Some(">= 3"));
        assert_eq!(bps.for_adapter(&file, false).1[0].hit_condition, None);

        // Saved with the breakpoint, and read back.
        let text = serde_json::to_string(bps.saved()).expect("save");
        assert!(text.contains(r#""hit_condition":">= 3""#), "{text}");
        let restored: Vec<Breakpoint> = serde_json::from_str(&text).expect("load");
        assert_eq!(restored[0].hit_condition.as_deref(), Some(">= 3"));
        // Breakpoints saved before there were hit conditions have none.
        let old: Vec<Breakpoint> = serde_json::from_str(r#"[{"id":1,"path":"/a.rs","line":3}]"#).expect("old");
        assert_eq!((old[0].hit_condition.as_ref(), old[0].enabled), (None, true));
    }

    #[test]
    fn ids_continue_after_restore() {
        let mut bps = Breakpoints::default();
        bps.add(PathBuf::from("/a.rs"), 1);
        bps.add(PathBuf::from("/a.rs"), 2);
        let mut restored = Breakpoints::from_saved(bps.saved().to_vec(), bps.sets(), Some(bps.active_set()));
        assert_eq!(restored.add(PathBuf::from("/a.rs"), 3), 3);
    }

    #[test]
    fn breakpoints_saved_before_sets_are_in_the_first_set() {
        let old: Vec<Breakpoint> =
            serde_json::from_str(r#"[{"id":1,"path":"/a.rs","line":3},{"id":2,"path":"/a.rs","line":9}]"#)
                .expect("old");
        let bps = Breakpoints::from_saved(old, &[], None);
        assert_eq!((bps.sets(), bps.active_set()), (&[0][..], 0));
        assert!(bps.iter().all(|b| b.set == 0));

        // A set gone from the file, or one out of the palette, takes its breakpoints to the first.
        let mut saved = bps.saved().to_vec();
        saved[1].set = 3;
        let bps = Breakpoints::from_saved(saved, &[2, 2, 9], Some(4));
        assert_eq!((bps.sets(), bps.active_set()), (&[2][..], 2));
        assert_eq!(bps.iter().map(|b| b.set).collect::<Vec<_>>(), [2, 2]);
    }

    #[test]
    fn sets_are_saved_with_their_breakpoints() {
        let mut bps = Breakpoints::default();
        bps.add(PathBuf::from("/a.rs"), 1);
        let teal = bps.new_set().expect("set");
        let b = bps.add(PathBuf::from("/a.rs"), 2);
        let text = serde_json::to_string(bps.saved()).expect("save");
        let restored = Breakpoints::from_saved(serde_json::from_str(&text).expect("load"), bps.sets(), Some(teal));
        assert_eq!((restored.sets(), restored.active_set()), (&[0, teal][..], teal));
        assert_eq!(restored.get(b).map(|b| b.set), Some(teal));
        assert_eq!(restored.get(1).map(|b| b.set), Some(0));
    }

    #[test]
    fn new_breakpoints_go_to_the_active_set_and_sets_take_the_free_colours() {
        let mut bps = Breakpoints::default();
        let first = bps.add(PathBuf::from("/a.rs"), 1);
        let (one, two) = (bps.new_set().expect("set"), bps.new_set().expect("set"));
        assert_eq!((one, two, bps.active_set()), (1, 2, 2));
        let second = bps.add(PathBuf::from("/a.rs"), 2);
        assert_eq!(bps.get(first).map(|b| b.set), Some(0));
        assert_eq!(bps.get(second).map(|b| b.set), Some(2));
        // A deleted set's colour is free again; then there is one set per colour, and no more.
        bps.delete_set(one, Some(0)).expect("delete");
        assert_eq!(bps.new_set().expect("set"), 1);
        while bps.sets().len() < usize::from(SET_COLORS) {
            bps.new_set().expect("set");
        }
        assert!(bps.new_set().is_err());
        assert_eq!(bps.neighbour(bps.sets()[0], -1), *bps.sets().last().expect("sets"));
    }

    #[test]
    fn only_hits_of_other_sets_all_of_them_pass() {
        let mut bps = Breakpoints::default();
        let rose = bps.add(PathBuf::from("/a.rs"), 1);
        let teal_set = bps.new_set().expect("set");
        let teal = bps.add(PathBuf::from("/a.rs"), 2);
        bps.activate(0).expect("activate");
        assert!(bps.passes(&[teal]));
        assert!(!bps.passes(&[rose]));
        assert!(!bps.passes(&[teal, rose]), "a hit of the active set stops");
        assert!(!bps.passes(&[]), "a stop at no breakpoint waits");
        assert!(!bps.passes(&[99]), "nor does one at a breakpoint that is gone");
        assert!(bps.any_passing());
        assert_eq!(bps.set_of(&[99, teal]), Some(teal_set));

        // Switching sets turns it around, with nothing sent to the adapter.
        bps.activate(teal_set).expect("activate");
        assert!(bps.passes(&[rose]) && !bps.passes(&[teal]));
        // A disabled breakpoint, which the adapter does not have, cannot be hit.
        bps.update(rose, |b| b.enabled = false);
        assert!(!bps.any_passing());
    }

    #[test]
    fn a_deleted_set_moves_its_breakpoints_or_takes_them_along() {
        let mut bps = Breakpoints::default();
        let teal = bps.new_set().expect("set");
        let a = bps.add(PathBuf::from("/a.rs"), 1);
        let orchid = bps.new_set().expect("set");
        let b = bps.add(PathBuf::from("/b.rs"), 2);
        assert!(bps.delete_set(teal, Some(teal)).is_err());
        assert!(bps.delete_set(teal, Some(4)).is_err());

        assert!(bps.delete_set(teal, Some(orchid)).expect("move").is_empty());
        assert_eq!(bps.get(a).map(|b| b.set), Some(orchid));
        assert_eq!(bps.sets(), [0, orchid]);

        // The active set going leaves the first one active, with its breakpoints gone.
        let gone = bps.delete_set(orchid, None).expect("delete");
        assert_eq!(gone.iter().map(|b| b.id).collect::<Vec<_>>(), [a, b]);
        assert_eq!((bps.sets(), bps.active_set(), bps.len()), (&[0][..], 0, 0));
        assert!(bps.delete_set(0, None).is_err(), "the last set stays");
    }
}
