//! Breakpoints live in the engine, not in a session: they survive restarts and are
//! sent to every session whose adapter handles the file's extension.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tuitor_dap::types::{self as dap, SourceBreakpoint};

use crate::session::SessionId;

pub type BreakpointId = u32;

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

#[derive(Debug, Default)]
pub struct Breakpoints {
    /// Sorted by (path, line).
    items: Vec<Breakpoint>,
    next_id: BreakpointId,
}

impl Breakpoints {
    pub fn from_saved(mut items: Vec<Breakpoint>) -> Self {
        items.sort_by(|a, b| (&a.path, a.line).cmp(&(&b.path, b.line)));
        let next_id = items.iter().map(|b| b.id).max().unwrap_or(0);
        Self { items, next_id }
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
        let mut restored = Breakpoints::from_saved(bps.saved().to_vec());
        assert_eq!(restored.add(PathBuf::from("/a.rs"), 3), 3);
    }
}
