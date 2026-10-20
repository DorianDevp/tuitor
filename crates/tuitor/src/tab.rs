//! Project tabs. Each tab is one project with an engine of its own, so the debuggers
//! of different tabs share nothing: an adapter that fails or exits takes only its own
//! tab's sessions with it. A tab also keeps its own views, cursors and history.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tuitor_core::config::Adapters;
use tuitor_core::project::Project;
use tuitor_core::session::SessionId;
use tuitor_core::{Engine, EngineMsg, EngineOptions, persist};

use crate::app::Pane;
use crate::command::{BpScope, View};
use crate::compare::Comparison;
use crate::flow::{FlowState, Item, executing};
use crate::search::Search;

pub type TabId = u32;

/// Tabs in order, one of them active; never empty. A tab keeps its id while others open
/// and close, so a message for a closed tab finds nothing rather than the wrong tab.
pub struct Tabs<T> {
    items: Vec<(TabId, T)>,
    active: usize,
    next_id: TabId,
}

impl<T> Tabs<T> {
    pub fn new(first: T) -> Self {
        Tabs { items: vec![(1, first)], active: 0, next_id: 2 }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn active(&self) -> &T {
        &self.items[self.active].1
    }

    pub fn active_mut(&mut self) -> &mut T {
        &mut self.items[self.active].1
    }

    pub fn active_id(&self) -> TabId {
        self.items[self.active].0
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn iter(&self) -> impl Iterator<Item = (TabId, &T)> {
        self.items.iter().map(|(id, tab)| (*id, tab))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (TabId, &mut T)> {
        self.items.iter_mut().map(|(id, tab)| (*id, tab))
    }

    pub fn get(&self, id: TabId) -> Option<&T> {
        self.items.iter().find(|(i, _)| *i == id).map(|(_, tab)| tab)
    }

    pub fn get_mut(&mut self, id: TabId) -> Option<&mut T> {
        self.items.iter_mut().find(|(i, _)| *i == id).map(|(_, tab)| tab)
    }

    /// Adds a tab after the others and makes it active.
    pub fn push(&mut self, tab: T) -> TabId {
        let id = self.allocate();
        self.items.push((id, tab));
        self.active = self.items.len() - 1;
        id
    }

    /// Puts `tab` where the active tab is, under a new id; returns the tab it replaces.
    pub fn replace_active(&mut self, tab: T) -> (TabId, T) {
        let id = self.allocate();
        let (_, old) = std::mem::replace(&mut self.items[self.active], (id, tab));
        (id, old)
    }

    pub fn activate(&mut self, id: TabId) -> bool {
        let Some(index) = self.items.iter().position(|(i, _)| *i == id) else { return false };
        self.active = index;
        true
    }

    /// Activates the tab `delta` places away, wrapping around.
    pub fn cycle(&mut self, delta: isize) {
        let len = self.items.len() as isize;
        self.active = (self.active as isize + delta).rem_euclid(len) as usize;
    }

    /// Removes a tab; its right neighbour becomes active if it was. Closing the last tab
    /// leaves `empty()` in its place.
    pub fn close(&mut self, id: TabId, empty: impl FnOnce() -> T) -> Option<T> {
        let index = self.items.iter().position(|(i, _)| *i == id)?;
        if self.items.len() == 1 {
            return Some(self.replace_active(empty()).1);
        }
        let (_, tab) = self.items.remove(index);
        if index < self.active || self.active == self.items.len() {
            self.active -= 1;
        }
        Some(tab)
    }

    fn allocate(&mut self) -> TabId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

/// Passes one engine's messages on, labelled with its tab, until the engine is gone.
/// The loop then reads every tab's messages from one channel.
pub fn forward<T: Send + 'static>(tab: TabId, mut rx: UnboundedReceiver<T>, tx: UnboundedSender<(TabId, T)>) {
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if tx.send((tab, msg)).is_err() {
                break;
            }
        }
    });
}

/// A selection in a list plus the scroll offset the renderer keeps it visible with.
#[derive(Debug, Default)]
pub struct Cursor {
    pub pos: usize,
    offset: Cell<usize>,
}

impl Cursor {
    pub fn move_by(&mut self, delta: isize, len: usize) {
        if len == 0 {
            self.pos = 0;
            return;
        }
        self.pos = self.pos.saturating_add_signed(delta).min(len - 1);
    }

    pub fn clamp(&mut self, len: usize) {
        self.pos = self.pos.min(len.saturating_sub(1));
    }

    /// First visible row for a window of `height` rows over `len` rows.
    pub fn window(&self, height: usize, len: usize) -> usize {
        let mut offset = self.offset.get().min(len.saturating_sub(height));
        if self.pos < offset {
            offset = self.pos;
        } else if height > 0 && self.pos >= offset + height {
            offset = self.pos + 1 - height;
        }
        self.offset.set(offset);
        offset
    }
}

/// What SOURCE shows: the execution point (follow) or a file the user opened.
#[derive(Debug, Default)]
pub struct SourceState {
    /// The file the user opened or moved the cursor in; the next automatic change
    /// (`Tab::jump_to_execution`) drops it.
    pub browse: Option<PathBuf>,
    /// 1-based line of the cursor.
    pub cursor: u32,
    /// First visible line (1-based), kept by the renderer.
    pub top: Cell<u32>,
    /// Lines on screen, kept by the renderer.
    pub rows: Cell<u32>,
    /// Centre the cursor vertically on the next render.
    pub recenter: Cell<bool>,
}

impl SourceState {
    /// A page further on (`forward`) or back over a file of `lines` lines, as vim's ^f and ^b:
    /// two lines of the page before stay on the screen, and so does the cursor. With no
    /// further to go, the cursor goes to the last line, or the first.
    pub fn scroll(&mut self, forward: bool, lines: u32) {
        let (top, rows) = (self.top.get().max(1), self.rows.get().max(1));
        let step = rows.saturating_sub(2).max(1);
        let last_top = lines.saturating_sub(rows) + 1;
        let new_top = if forward { (top + step).min(last_top) } else { top.saturating_sub(step).max(1) };
        self.top.set(new_top);
        self.cursor = match (new_top == top, forward) {
            (true, true) => lines.max(1),
            (true, false) => 1,
            (false, _) => self.cursor.clamp(new_top, new_top + rows - 1),
        };
    }

    /// Puts the cursor on `line`, in the middle of the screen when it is off it.
    pub fn go_to(&mut self, line: u32) {
        self.cursor = line;
        let top = self.top.get();
        if line < top || line >= top + self.rows.get().max(1) {
            self.recenter.set(true);
        }
    }
}

/// How a tab is looked at: its view, focus, cursors and the stop being shown.
pub struct TabUi {
    pub view: View,
    pub(crate) focus: HashMap<View, Pane>,
    pub zoom: bool,
    pub session: Option<SessionId>,
    /// Stop number shown per session; absent means the latest (live) stop.
    pub viewing: HashMap<SessionId, u32>,
    pub source: SourceState,
    /// What was searched for in SOURCE, highlighted until `esc`.
    pub search: Option<Search>,
    pub locals: Cursor,
    pub stack: Cursor,
    pub show_vendor: bool,
    pub watch: Cursor,
    pub bps: Cursor,
    pub bp_scope: BpScope,
    /// Lines up from the bottom of OUTPUT; 0 follows new output.
    pub output_back: usize,
    pub events: Cursor,
    pub history_col: usize,
    /// Row of CONFIG under the cursor.
    pub config: Cursor,
    /// The configuration CONFIG shows the fields of; `S` starts it.
    pub config_sel: usize,
    /// The line of FLOW under the cursor, once moved; see [`Tab::flow_cursor`].
    pub flow: Cursor,
    /// FLOW's cursor is on the file where execution is, until it is moved.
    pub flow_follow: bool,
    /// FLOW lists library files one by one instead of one line per package.
    pub flow_vendor: bool,
    pub outline: Cursor,
    /// What COMPARE shows, once `/compare` said; until then its default.
    pub comparison: Option<Comparison>,
    /// The line of COMPARE under the cursor, and its column: the breakpoint, or a side.
    pub compare: Cursor,
    pub compare_col: usize,
    pub(crate) seen_stops: HashMap<SessionId, usize>,
}

impl Default for TabUi {
    fn default() -> Self {
        TabUi {
            view: View::Overview,
            focus: HashMap::new(),
            zoom: false,
            session: None,
            viewing: HashMap::new(),
            source: SourceState::default(),
            search: None,
            locals: Cursor::default(),
            stack: Cursor::default(),
            show_vendor: false,
            watch: Cursor::default(),
            bps: Cursor::default(),
            bp_scope: BpScope::Session,
            output_back: 0,
            events: Cursor::default(),
            history_col: 0,
            config: Cursor::default(),
            config_sel: 0,
            flow: Cursor::default(),
            flow_follow: true,
            flow_vendor: false,
            outline: Cursor::default(),
            comparison: None,
            compare: Cursor::default(),
            compare_col: 0,
            seen_stops: HashMap::new(),
        }
    }
}

pub struct Tab {
    /// `None` in a tab no project was opened in yet.
    pub project: Option<Project>,
    /// An empty tab has an engine too, with nowhere to start a session, so that every
    /// view draws the same way in every tab.
    pub engine: Engine,
    pub ui: TabUi,
    /// Files for the finder, listed on first use.
    pub files: Option<Vec<String>>,
    pub flows: FlowState,
}

impl Tab {
    pub fn empty(adapters: &Adapters) -> Tab {
        let options = EngineOptions { adapters: adapters.clone(), workspace: PathBuf::new(), state_file: None };
        let (engine, _) = Engine::new(options);
        Tab { project: None, engine, ui: TabUi::default(), files: None, flows: FlowState::default() }
    }

    /// A tab for `project`, and the channel its engine reports on.
    pub fn open(project: Project, adapters: &Adapters) -> (Tab, UnboundedReceiver<EngineMsg>) {
        let (engine, rx) = Engine::new(EngineOptions {
            adapters: adapters.clone(),
            workspace: project.dir.clone(),
            state_file: persist::state_file(&project.dir),
        });
        let flows = FlowState::open(&project.dir);
        (Tab { project: Some(project), engine, ui: TabUi::default(), files: None, flows }, rx)
    }

    /// The line of FLOW under the cursor among `items`: the file where execution is until
    /// the cursor is moved.
    pub fn flow_cursor(&self, items: &[Item]) -> usize {
        let flow = self.flow();
        let followed = flow.as_ref().filter(|_| self.ui.flow_follow).and_then(|view| executing(&view.flow, items));
        followed.unwrap_or(self.ui.flow.pos).min(items.len().saturating_sub(1))
    }

    /// The project's directory name; tabs are told apart by it.
    pub fn name(&self) -> String {
        self.project.as_ref().map_or_else(|| "new".to_owned(), Project::name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SOURCE with `rows` rows from line `top`, the cursor on `cursor`.
    fn source(top: u32, rows: u32, cursor: u32) -> SourceState {
        let state = SourceState { cursor, ..SourceState::default() };
        state.top.set(top);
        state.rows.set(rows);
        state
    }

    #[test]
    fn scrolling_moves_a_page_less_two_lines_and_keeps_the_cursor_on_it() {
        // 20 rows over 100 lines: the next page starts 18 lines on.
        let mut s = source(1, 20, 5);
        s.scroll(true, 100);
        assert_eq!((s.top.get(), s.cursor), (19, 19));
        // A cursor still on the new page stays where it is.
        let mut s = source(1, 20, 25);
        s.scroll(true, 100);
        assert_eq!((s.top.get(), s.cursor), (19, 25));
        s.scroll(false, 100);
        assert_eq!((s.top.get(), s.cursor), (1, 20));
        // The last page ends at the last line; then the cursor goes there.
        let mut s = source(75, 20, 80);
        s.scroll(true, 100);
        assert_eq!((s.top.get(), s.cursor), (81, 81));
        s.scroll(true, 100);
        assert_eq!((s.top.get(), s.cursor), (81, 100));
        // And back at the top, to the first line.
        let mut s = source(1, 20, 12);
        s.scroll(false, 100);
        assert_eq!((s.top.get(), s.cursor), (1, 1));
        // A file shorter than the screen does not scroll.
        let mut s = source(1, 20, 3);
        s.scroll(true, 10);
        assert_eq!((s.top.get(), s.cursor), (1, 10));
    }

    #[test]
    fn going_to_a_line_off_the_screen_centres_it() {
        let mut s = source(1, 20, 5);
        s.go_to(12);
        assert_eq!((s.cursor, s.recenter.get()), (12, false));
        s.go_to(60);
        assert_eq!((s.cursor, s.recenter.get()), (60, true));
    }

    #[test]
    fn ids_stay_with_their_tab() {
        let mut tabs = Tabs::new("api");
        let web = tabs.push("web");
        let ledger = tabs.push("ledger");
        assert_eq!((tabs.active_id(), *tabs.active()), (ledger, "ledger"));
        assert_eq!(tabs.close(web, || "new"), Some("web"));
        // A message for the closed tab is not delivered to another one.
        assert_eq!(tabs.get(web), None);
        assert_eq!(tabs.get(ledger), Some(&"ledger"));
        let (replaced, old) = tabs.replace_active("shop");
        assert_eq!(old, "ledger");
        assert_ne!(replaced, ledger);
        assert_eq!(tabs.get(ledger), None);
    }

    #[test]
    fn closing_activates_a_neighbour_and_never_leaves_no_tab() {
        let mut tabs = Tabs::new("a");
        let b = tabs.push("b");
        let c = tabs.push("c");
        tabs.activate(b);
        tabs.close(b, || "new");
        assert_eq!(*tabs.active(), "c");
        tabs.close(c, || "new");
        assert_eq!(*tabs.active(), "a");
        let a = tabs.active_id();
        assert_eq!(tabs.close(a, || "new"), Some("a"));
        assert_eq!((tabs.len(), *tabs.active()), (1, "new"));
    }

    #[test]
    fn cycling_wraps_around() {
        let mut tabs = Tabs::new("a");
        tabs.push("b");
        tabs.push("c");
        tabs.cycle(1);
        assert_eq!(*tabs.active(), "a");
        tabs.cycle(-1);
        assert_eq!(*tabs.active(), "c");
        tabs.cycle(-4);
        assert_eq!(*tabs.active(), "b");
    }

    #[tokio::test]
    async fn every_engine_reaches_one_inbox_labelled_with_its_tab() {
        let (inbox, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let (api, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let (web, web_rx) = tokio::sync::mpsc::unbounded_channel();
        forward(1, api_rx, inbox.clone());
        forward(2, web_rx, inbox);
        api.send("stopped").expect("send");
        web.send("output").expect("send");
        let mut got = vec![rx.recv().await, rx.recv().await];
        got.sort();
        assert_eq!(got, [Some((1, "stopped")), Some((2, "output"))]);
        // The inbox closes once every engine is gone.
        drop((api, web));
        assert_eq!(rx.recv().await, None);
    }
}
