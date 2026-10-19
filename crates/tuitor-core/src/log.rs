//! What the debuggee printed and what the adapters said, kept in bounded buffers.

use std::collections::VecDeque;

use chrono::{DateTime, Local};

use crate::session::SessionId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputKind {
    Stdout,
    Stderr,
    /// Adapter messages meant for the user (`console`, `important`).
    Console,
    /// Lines the adapter process wrote outside the protocol.
    Adapter,
    /// Results of `/eval`.
    Repl,
}

impl OutputKind {
    pub(crate) fn from_category(category: Option<&str>) -> Option<Self> {
        Some(match category {
            Some("stdout") => Self::Stdout,
            Some("stderr") => Self::Stderr,
            Some("telemetry") => return None,
            _ => Self::Console,
        })
    }
}

#[derive(Debug, Clone)]
pub struct OutputLine {
    pub time: DateTime<Local>,
    pub session: SessionId,
    pub kind: OutputKind,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct EventLine {
    pub time: DateTime<Local>,
    pub session: SessionId,
    pub name: String,
    pub detail: String,
}

/// Ring buffer that drops the oldest entries.
#[derive(Debug)]
pub struct Bounded<T> {
    items: VecDeque<T>,
    cap: usize,
}

impl<T> Bounded<T> {
    pub fn new(cap: usize) -> Self {
        Self { items: VecDeque::with_capacity(cap.min(1024)), cap }
    }

    pub fn push(&mut self, item: T) {
        if self.items.len() == self.cap {
            self.items.pop_front();
        }
        self.items.push_back(item);
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + ExactSizeIterator {
        self.items.iter()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }
}
