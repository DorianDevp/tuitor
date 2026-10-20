//! Paths through a line: the call trees from where execution starts (a controller, `main`, a
//! test, a script's top level, or a function nothing calls) down through the function the
//! line is in, and on into what each function on the way calls; found by asking a language
//! server who calls each function in turn, up, and what each calls, down.
//!
//! Callers come from `callHierarchy/incomingCalls`, or, from a server without it, from the
//! `textDocument/references` to a function that call it, each put in the function
//! `textDocument/documentSymbol` says it is in. Callees come from
//! `callHierarchy/outgoingCalls`, or from the `textDocument/definition` of each name the text
//! calls, kept when it is a function of the project. A call in a closure counts as one from
//! the function the closure is written in. Calls the stacks of earlier runs showed join them,
//! marked as seen at run time: no server sees what a container, an event or an interface
//! calls.
//!
//! Up, the graph goes as far as where execution starts. Down, from every function on a way up
//! to the target, it goes a few calls further, into the project's code only, and keeps the
//! calls already known; then each place execution starts is unfolded into a tree of calls
//! (see [`crate::tree`]).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::config::Language;
use crate::lsp::{self, Pos, Span, Symbol, WRITE};
use crate::select::Facts;
use crate::text::{self, Lexer, Text};
use crate::tree::{Cut, Entry, Found, Function, Root, Stage};

/// What paths are built from: a language server's answers. Tests answer with what servers
/// answered.
pub(crate) trait Ask {
    /// Whether it answers `callHierarchy/*`.
    fn call_hierarchy(&self) -> bool;
    /// Gives the server `text` as the content of `path`, before it is asked about it.
    fn open(&mut self, path: &Path, text: &Text) -> Result<()>;
    async fn ask(&mut self, method: &'static str, params: Value) -> Result<Value>;
}

/// How far to look.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    /// Calls up from the target to where execution starts.
    pub depth: usize,
    /// Calls down from a function on a way to the target into what else it calls.
    pub below: usize,
    /// Trees, one per place execution starts, those nearest the target first.
    pub roots: usize,
    /// Callers followed per function.
    pub callers: usize,
    /// Calls followed per function, the first in its text first.
    pub callees: usize,
    /// Requests for callers, and as many for callees; past them no function is asked about
    /// (one already asked about is finished) and the graph found so far is the answer.
    pub requests: usize,
    /// Stages in all the trees.
    pub stages: usize,
    /// Requests for which lines write; past them the text alone says.
    pub lines: usize,
    /// Lines of state kept per stage, besides its entry and calls.
    pub state: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            depth: 8,
            below: 3,
            roots: 8,
            callers: 10,
            callees: 12,
            requests: 300,
            stages: 400,
            lines: 600,
            state: 3,
        }
    }
}

/// Requests for the writes of each function.
const LINE_REQUESTS: usize = 16;
/// Partial chains kept while the ways up are put together, so that a dense graph cannot run
/// away.
const CHAINS: usize = 4096;
/// Names a function calls whose definitions are asked for, from a server without call
/// hierarchy.
const NAMES: usize = 24;

/// A call an earlier run's stack showed: from `line` (1-based) of `path`, into the function
/// named `callee` (as the debugger names it) in `callee_path`.
#[derive(Debug, Clone, PartialEq)]
pub struct Observed {
    pub path: PathBuf,
    pub line: u32,
    pub callee: String,
    pub callee_path: PathBuf,
}

type Key = (PathBuf, Pos);

/// A function as the finder knows it.
#[derive(Debug, Clone)]
struct Def {
    /// With its class or type, for showing.
    name: String,
    /// As declared.
    short: String,
    container: Option<String>,
    path: PathBuf,
    range: Span,
    /// Its name: what requests about it point at.
    at: Pos,
    /// The top level of a file, where calls outside any function are made.
    script: bool,
    /// What `callHierarchy` knows it by, once asked.
    item: Option<Value>,
}

impl Def {
    fn key(&self) -> Key {
        (self.path.clone(), self.at)
    }
}

struct Node {
    def: Def,
    /// Calls into it.
    callers: Vec<Edge>,
    /// Calls it makes.
    callees: Vec<Edge>,
    /// Its callers were asked for.
    up: bool,
    /// Its calls were asked for.
    down: bool,
    /// Lines (0-based) that call a function of the project, with the function's name.
    uses: Vec<(u32, String)>,
}

/// A call between two nodes; `other` is the caller in `callers`, the callee in `callees`.
#[derive(Debug, Clone)]
struct Edge {
    other: usize,
    /// Lines (0-based) of the caller that make the call.
    sites: Vec<u32>,
    /// Only a run's stack showed it.
    observed: bool,
}

/// What requests are spent on, each with a budget of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Phase {
    Up,
    Down,
    Lines,
}

/// The lines of a function that are its own: a script's, outside its functions.
struct Own {
    range: Span,
    nested: Vec<Span>,
}

impl Own {
    fn has(&self, line: u32) -> bool {
        self.range.has_line(line) && !self.nested.iter().any(|span| span.has_line(line))
    }
}

/// What is asked: the paths through `line` (1-based) of `file`, in the project in `root`,
/// with the calls earlier runs showed.
pub(crate) struct Question<'a> {
    pub root: &'a Path,
    pub file: &'a Path,
    pub line: u32,
    pub observed: &'a [Observed],
    pub limits: Limits,
}

/// Finds the trees, telling `progress` what it asks about as it goes.
pub(crate) async fn find<A: Ask>(
    ask: &mut A,
    language: Language,
    question: Question<'_>,
    progress: impl FnMut(String) + Send,
) -> Result<Found> {
    let Question { root, file, line, observed, limits } = question;
    let mut finder = Finder {
        ask,
        language,
        root: root.to_path_buf(),
        limits,
        observed,
        texts: HashMap::new(),
        symbols: HashMap::new(),
        nodes: Vec::new(),
        index: HashMap::new(),
        requests: 0,
        spent: HashMap::new(),
        phase: Phase::Up,
        notes: Vec::new(),
        progress,
    };
    let line = line.saturating_sub(1);
    let target = finder.function_at(file, line).await?;
    let target = finder.add(target);
    finder.explore_up(target).await;
    let (roots, spine) = finder.roots(target);
    finder.phase = Phase::Down;
    finder.explore_down(&spine).await;
    let spine: HashSet<usize> = spine.into_iter().collect();
    let stages = finder.unfold(&roots, &spine, target);
    finder.phase = Phase::Lines;
    finder.found(stages, target, line).await
}

struct Finder<'a, A, P> {
    ask: &'a mut A,
    language: Language,
    root: PathBuf,
    limits: Limits,
    observed: &'a [Observed],
    texts: HashMap<PathBuf, Arc<Text>>,
    symbols: HashMap<PathBuf, Arc<Vec<Symbol>>>,
    nodes: Vec<Node>,
    index: HashMap<Key, usize>,
    requests: usize,
    /// Requests per phase.
    spent: HashMap<Phase, usize>,
    phase: Phase,
    notes: Vec<String>,
    progress: P,
}

/// Stages being unfolded from the graph; each stage's `function` is a node until the end.
struct Unfolding {
    stages: Vec<Stage>,
    /// Out of room for stages.
    full: bool,
}

impl<A: Ask, P: FnMut(String) + Send> Finder<'_, A, P> {
    async fn request(&mut self, method: &'static str, params: Value) -> Result<Value> {
        self.requests += 1;
        *self.spent.entry(self.phase).or_default() += 1;
        self.ask.ask(method, params).await
    }

    /// Whether the requests of the phase are spent.
    fn spent(&self, phase: Phase) -> bool {
        let budget = match phase {
            Phase::Up | Phase::Down => self.limits.requests,
            Phase::Lines => self.limits.lines,
        };
        self.spent.get(&phase).copied().unwrap_or(0) >= budget
    }

    fn text(&mut self, path: &Path) -> Result<Arc<Text>> {
        if let Some(text) = self.texts.get(path) {
            return Ok(text.clone());
        }
        let text = Arc::new(Text::read(path)?);
        self.ask.open(path, &text)?;
        self.texts.insert(path.to_path_buf(), text.clone());
        Ok(text)
    }

    async fn symbols(&mut self, path: &Path) -> Result<Arc<Vec<Symbol>>> {
        if let Some(symbols) = self.symbols.get(path) {
            return Ok(symbols.clone());
        }
        self.text(path)?;
        let answer =
            self.request("textDocument/documentSymbol", json!({ "textDocument": lsp::document(path) })).await?;
        let symbols = Arc::new(lsp::symbols(&answer));
        self.symbols.insert(path.to_path_buf(), symbols.clone());
        Ok(symbols)
    }

    /// The function `line` (0-based) of `path` is in.
    async fn function_at(&mut self, path: &Path, line: u32) -> Result<Def> {
        let symbols = self.symbols(path).await?;
        match self.function(path, &symbols, |s| s.range.has_line(line)) {
            Some(function) => Ok(function),
            None => bail!("line {} of {} is in no function the server knows of", line + 1, file_name(path)),
        }
    }

    /// The function `pos` of `path` is in, or the file's top level.
    async fn enclosing(&mut self, path: &Path, pos: Pos) -> Result<Def> {
        let symbols = self.symbols(path).await?;
        match self.function(path, &symbols, |s| s.range.contains(pos)) {
            Some(function) => Ok(function),
            None => self.script(path),
        }
    }

    /// The function declared at `span` of `path`, as a definition or a call hierarchy item
    /// gives it: its name in the span, or on the line the span starts. None when what is
    /// there is no function: a variable, a field, a class.
    async fn declared(&mut self, path: &Path, span: Span) -> Result<Option<Def>> {
        let symbols = self.symbols(path).await?;
        let def = self.function(path, &symbols, |s| s.range.contains(span.start) || span.contains(s.selection.start));
        Ok(def.filter(|def| span.contains(def.at) || def.at.line == span.start.line))
    }

    /// The function of `path` a debugger calls `name`.
    async fn named(&mut self, path: &Path, name: &str) -> Result<Option<Def>> {
        let symbols = self.symbols(path).await?;
        let short = short_name(name);
        let mut all = Vec::new();
        flatten(&symbols, &mut all);
        let Some(symbol) = all.into_iter().find(|s| is_function(s) && short_name(&s.name) == short) else {
            return Ok(None);
        };
        let at = symbol.selection.start;
        Ok(self.function(path, &symbols, |s| s.range.contains(at)).filter(|def| def.at == at))
    }

    /// The top level of `path`, as a function that calls what is called outside functions.
    fn script(&mut self, path: &Path) -> Result<Def> {
        let last = self.text(path)?.last_line();
        let name = file_name(path);
        Ok(Def {
            short: name.clone(),
            name,
            container: None,
            path: path.to_path_buf(),
            range: Span { start: Pos::default(), end: Pos::new(last, 0) },
            at: Pos::default(),
            script: true,
            item: None,
        })
    }

    /// The innermost named function among `symbols` that `inside` holds for. A closure is
    /// part of the function it is written in.
    fn function(&self, path: &Path, symbols: &[Symbol], inside: impl Fn(&Symbol) -> bool) -> Option<Def> {
        let mut chain: Vec<&Symbol> = Vec::new();
        let mut level = symbols;
        // The narrowest at each level: a flat list has a class and its methods side by side.
        while let Some(symbol) = level.iter().filter(|s| inside(s)).min_by_key(|s| lines(s.range)) {
            chain.push(symbol);
            level = &symbol.children;
        }
        let at = chain.iter().rposition(|s| is_function(s))?;
        let symbol = chain[at];
        // What it is declared in: a class, an `impl`, a module (Rust's `mod tests`).
        let parent = chain[..at].iter().rev().find(|s| !is_function(s) && s.kind != lsp::FILE);
        let container = parent.map(|s| owner(&s.name));
        let name = match (parent, &container) {
            (Some(parent), Some(owner)) if !matches!(parent.kind, lsp::MODULE | lsp::NAMESPACE | lsp::PACKAGE) => {
                format!("{owner}{}{}", self.separator(), symbol.name)
            }
            _ => symbol.name.clone(),
        };
        Some(Def {
            name,
            short: symbol.name.clone(),
            container,
            path: path.to_path_buf(),
            range: symbol.range,
            at: symbol.selection.start,
            script: false,
            item: None,
        })
    }

    fn separator(&self) -> &'static str {
        match self.language {
            Language::Php | Language::Rust => "::",
            Language::TypeScript | Language::Go => ".",
        }
    }

    fn add(&mut self, def: Def) -> usize {
        if let Some(&n) = self.index.get(&def.key()) {
            if self.nodes[n].def.item.is_none() {
                self.nodes[n].def.item = def.item;
            }
            return n;
        }
        self.index.insert(def.key(), self.nodes.len());
        self.nodes.push(Node {
            def,
            callers: Vec::new(),
            callees: Vec::new(),
            up: false,
            down: false,
            uses: Vec::new(),
        });
        self.nodes.len() - 1
    }

    /// The lines of `def` that are its own.
    async fn own(&mut self, def: &Def) -> Result<Own> {
        let mut nested = Vec::new();
        if def.script {
            collect_functions(&self.symbols(&def.path).await?, &mut nested);
        }
        Ok(Own { range: def.range, nested })
    }

    // ---- up ---------------------------------------------------------------------------

    /// Asks for the callers of the target, then theirs, nearest first, up to an entry point,
    /// a function nothing calls or the limits.
    async fn explore_up(&mut self, target: usize) {
        let mut queue = VecDeque::from([(target, 0)]);
        let (mut cut, mut crowded) = (HashSet::new(), Vec::new());
        while let Some((n, depth)) = queue.pop_front() {
            if self.nodes[n].up || self.entry(n).is_some() {
                continue;
            }
            if depth >= self.limits.depth {
                cut.insert(n);
                continue;
            }
            if self.spent(Phase::Up) {
                self.notes.push(format!("stopped asking for callers after {} requests", self.limits.requests));
                break;
            }
            (self.progress)(format!("callers of {}", self.nodes[n].def.name));
            let callers = match self.callers(n).await {
                Ok(callers) => callers,
                Err(err) => {
                    self.notes.push(format!("callers of {}: {err:#}", self.nodes[n].def.name));
                    Vec::new()
                }
            };
            self.nodes[n].up = true;
            if callers.len() > self.limits.callers {
                crowded.push(format!("{} ({})", self.nodes[n].def.name, callers.len()));
            }
            for (def, sites, observed) in callers.into_iter().take(self.limits.callers) {
                let from = self.add(def);
                self.nodes[n].callers.push(Edge { other: from, sites, observed });
                queue.push_back((from, depth + 1));
            }
        }
        if !cut.is_empty() {
            let (n, depth) = (cut.len(), self.limits.depth);
            let calls = if depth == 1 { "call" } else { "calls" };
            self.notes.push(format!(
                "{n} function{} at the limit of {depth} {calls} up: their callers were not asked for",
                plural(n)
            ));
        }
        if !crowded.is_empty() {
            let first = self.limits.callers;
            self.notes.push(format!("only {first} callers followed of {}", crowded.join(", ")));
        }
    }

    /// Who calls node `n`, once each, from which lines, and whether only a run showed it.
    async fn callers(&mut self, n: usize) -> Result<Vec<(Def, Vec<u32>, bool)>> {
        let def = self.nodes[n].def.clone();
        let mut found: Vec<(Def, Vec<u32>, bool)> = Vec::new();
        let called = if self.ask.call_hierarchy() { self.incoming(n).await? } else { self.referrers(&def).await? };
        for (caller, site) in called {
            match found.iter_mut().find(|(known, ..)| known.key() == caller.key()) {
                Some((_, sites, _)) => sites.push(site),
                None => found.push((caller, vec![site], false)),
            }
        }
        for (caller, site) in self.seen_callers(&def).await? {
            if !found.iter().any(|(known, ..)| known.key() == caller.key()) {
                found.push((caller, vec![site], true));
            }
        }
        for (_, sites, _) in &mut found {
            sites.sort_unstable();
            sites.dedup();
        }
        Ok(found)
    }

    /// Callers from `callHierarchy/incomingCalls`, a line per call.
    async fn incoming(&mut self, n: usize) -> Result<Vec<(Def, u32)>> {
        let Some(item) = self.item(n).await? else { return Ok(Vec::new()) };
        let answer = self.request("callHierarchy/incomingCalls", json!({ "item": item })).await?;
        let mut callers = Vec::new();
        for call in answer.as_array().into_iter().flatten() {
            let from = &call["from"];
            let Some(path) = from["uri"].as_str().and_then(lsp::path) else { continue };
            let Some(at) = from.get("selectionRange").and_then(Span::read).map(|s| s.start) else { continue };
            let mut caller =
                if is_script_item(from, &path) { self.script(&path)? } else { self.enclosing(&path, at).await? };
            if caller.at == at {
                caller.item = Some(from.clone());
            }
            let sites = call["fromRanges"].as_array().into_iter().flatten().filter_map(Span::read);
            callers.extend(sites.map(|site| (caller.clone(), site.start.line)));
        }
        Ok(callers)
    }

    /// The `callHierarchy` item of node `n`, asked for once.
    async fn item(&mut self, n: usize) -> Result<Option<Value>> {
        let def = &self.nodes[n].def;
        if let Some(item) = &def.item {
            return Ok(Some(item.clone()));
        }
        if def.script {
            return Ok(None);
        }
        let (path, at) = (def.path.clone(), def.at);
        let params = json!({ "textDocument": lsp::document(&path), "position": at.json() });
        let answer = self.request("textDocument/prepareCallHierarchy", params).await?;
        let items = answer.as_array().cloned().unwrap_or_default();
        let starts_at = |item: &&Value| item.get("selectionRange").and_then(Span::read).is_some_and(|s| s.start == at);
        let item = items.iter().find(starts_at).or(items.first()).cloned();
        self.nodes[n].def.item = item.clone();
        Ok(item)
    }

    /// Callers from `textDocument/references`: those that call it, each in the function it
    /// is in.
    async fn referrers(&mut self, def: &Def) -> Result<Vec<(Def, u32)>> {
        let params = json!({
            "textDocument": lsp::document(&def.path),
            "position": def.at.json(),
            "context": { "includeDeclaration": false },
        });
        let answer = self.request("textDocument/references", params).await?;
        let mut callers = Vec::new();
        for location in lsp::locations(&answer) {
            let text = self.text(&location.path)?;
            let line = text.line(location.span.end.line);
            // A callback registered by name, `[$listener, 'onOrder']`, is no call.
            if !text::called(line, lsp::byte(line, location.span.end.character)) {
                continue;
            }
            let caller = self.enclosing(&location.path, location.span.start).await?;
            callers.push((caller, location.span.start.line));
        }
        Ok(callers)
    }

    /// Callers the stacks of earlier runs showed calling `def`, from the project's code: a
    /// framework's own functions would lead up through its internals.
    async fn seen_callers(&mut self, def: &Def) -> Result<Vec<(Def, u32)>> {
        let calls: Vec<(PathBuf, u32)> = self
            .observed
            .iter()
            .filter(|call| call.callee_path == def.path && short_name(&call.callee) == def.short)
            .filter(|call| self.in_project(&call.path))
            .map(|call| (call.path.clone(), call.line.saturating_sub(1)))
            .collect();
        let mut callers = Vec::new();
        for (path, line) in calls {
            let Ok(text) = self.text(&path) else { continue };
            let indent = text.line(line).len() - text.line(line).trim_start().len();
            let at = Pos::new(line, lsp::utf16(text.line(line), indent));
            callers.push((self.enclosing(&path, at).await?, line));
        }
        Ok(callers)
    }

    /// Where execution starts, if node `n` is such a place.
    fn entry(&self, n: usize) -> Option<Entry> {
        let def = &self.nodes[n].def;
        let relative = def.path.strip_prefix(&self.root).unwrap_or(&def.path);
        let file = file_name(&def.path);
        let container = def.container.as_deref().unwrap_or_default();
        let in_dir =
            |names: &[&str]| relative.parent().is_some_and(|dir| dir.iter().any(|d| names.iter().any(|n| d == *n)));
        if def.script {
            Some(Entry::Script)
        } else if def.short == "main" {
            Some(Entry::Main)
        } else if file.ends_with("_test.go")
            || [".spec.", ".test.", "Test.php"].iter().any(|part| file.contains(part))
            || in_dir(&["tests", "test", "__tests__"])
            || container.ends_with("Test")
            || container == "tests"
        {
            Some(Entry::Test)
        } else if container.ends_with("Controller")
            || file.contains(".controller.")
            || in_dir(&["Controller", "controllers"])
        {
            Some(Entry::Controller)
        } else {
            None
        }
    }

    /// The chains from the target up to where they start, target first.
    fn chains(&mut self, target: usize) -> Vec<(Vec<usize>, Root)> {
        let mut done = Vec::new();
        let mut queue = VecDeque::from([vec![target]]);
        while let Some(chain) = queue.pop_front() {
            let top = chain[chain.len() - 1];
            let node = &self.nodes[top];
            let up: Vec<usize> =
                node.callers.iter().map(|edge| edge.other).filter(|from| !chain.contains(from)).collect();
            let root = match self.entry(top) {
                Some(entry) => Some(Root::Entry(entry)),
                None if !node.up => Some(Root::Cut),
                // Called only from the chain itself: recursion.
                None if up.is_empty() => Some(Root::Uncalled),
                None => None,
            };
            if let Some(root) = root {
                done.push((chain, root));
                continue;
            }
            if queue.len() + up.len() > CHAINS {
                self.notes.push("too many ways up to follow them all".into());
                break;
            }
            for from in up {
                let mut longer = chain.clone();
                longer.push(from);
                queue.push_back(longer);
            }
        }
        done
    }

    /// Where the trees start, those nearest the target first, and the functions on the ways
    /// from them up to it: the spine of the trees, whose calls are always followed.
    fn roots(&mut self, target: usize) -> (Vec<(usize, Root)>, Vec<usize>) {
        let mut chains = self.chains(target);
        chains.sort_by_key(|(chain, root)| (root.rank(), chain.len()));
        let mut roots: Vec<(usize, Root)> = Vec::new();
        let mut left_out: Vec<usize> = Vec::new();
        let mut spine: Vec<usize> = Vec::new();
        for (chain, root) in chains {
            let top = chain[chain.len() - 1];
            if !roots.iter().any(|(known, _)| *known == top) {
                if roots.len() >= self.limits.roots {
                    if !left_out.contains(&top) {
                        left_out.push(top);
                    }
                    continue;
                }
                roots.push((top, root));
            }
            for n in chain.into_iter().rev() {
                if !spine.contains(&n) {
                    spine.push(n);
                }
            }
        }
        if !left_out.is_empty() {
            let n = left_out.len();
            self.notes.push(format!("{n} more place{} execution starts not shown", plural(n)));
        }
        (roots, spine)
    }

    // ---- down -------------------------------------------------------------------------

    /// Asks what the functions of the spine call, and what those call, nearest first, as far
    /// as `below` calls from the spine, in the project's code only.
    async fn explore_down(&mut self, spine: &[usize]) {
        let on_spine: HashSet<usize> = spine.iter().copied().collect();
        let mut queue: VecDeque<(usize, usize)> = spine.iter().map(|&n| (n, 0)).collect();
        let mut crowded = Vec::new();
        let mut deep = HashSet::new();
        while let Some((n, depth)) = queue.pop_front() {
            if self.nodes[n].down {
                continue;
            }
            if depth >= self.limits.below {
                deep.insert(n);
                continue;
            }
            if self.spent(Phase::Down) {
                self.notes.push(format!("stopped asking what functions call after {} requests", self.limits.requests));
                break;
            }
            (self.progress)(format!("calls of {}", self.nodes[n].def.name));
            let calls = match self.callees(n).await {
                Ok(calls) => calls,
                Err(err) => {
                    self.notes.push(format!("calls of {}: {err:#}", self.nodes[n].def.name));
                    Vec::new()
                }
            };
            self.nodes[n].down = true;
            if calls.len() > self.limits.callees {
                crowded.push(format!("{} ({})", self.nodes[n].def.name, calls.len()));
            }
            let mut uses = Vec::new();
            for (k, (def, sites, observed)) in calls.into_iter().enumerate() {
                if !observed {
                    uses.extend(sites.iter().map(|line| (*line, def.short.clone())));
                }
                if k >= self.limits.callees {
                    continue;
                }
                let to = self.add(def);
                self.nodes[n].callees.push(Edge { other: to, sites, observed });
                if !on_spine.contains(&to) {
                    queue.push_back((to, depth + 1));
                }
            }
            uses.sort();
            uses.dedup();
            self.nodes[n].uses = uses;
        }
        if !crowded.is_empty() {
            let first = self.limits.callees;
            self.notes.push(format!("only the first {first} calls followed of {}", crowded.join(", ")));
        }
        let deep: Vec<usize> = deep.into_iter().filter(|n| !self.nodes[*n].down).collect();
        if !deep.is_empty() {
            let (n, below) = (deep.len(), self.limits.below);
            self.notes.push(format!(
                "{n} function{} {below} call{} below the paths: what {} call{} was not asked for (…)",
                plural(n),
                plural(below),
                if n == 1 { "it" } else { "they" },
                if n == 1 { "s" } else { "" },
            ));
        }
    }

    /// What node `n` calls in the project, once each, from which lines (0-based), and whether
    /// only a run showed it; the first in the text first.
    async fn callees(&mut self, n: usize) -> Result<Vec<(Def, Vec<u32>, bool)>> {
        let def = self.nodes[n].def.clone();
        let own = self.own(&def).await?;
        let item = if self.ask.call_hierarchy() { self.item(n).await? } else { None };
        let called = match item {
            Some(item) => self.outgoing(item, &own).await?,
            None => self.defined(&def, &own).await?,
        };
        let mut found: Vec<(Def, Vec<u32>, bool)> = Vec::new();
        for (callee, site) in called {
            match found.iter_mut().find(|(known, ..)| known.key() == callee.key()) {
                Some((_, sites, _)) => sites.push(site),
                None => found.push((callee, vec![site], false)),
            }
        }
        for (callee, site) in self.seen_callees(&def, &own).await? {
            match found.iter_mut().find(|(known, ..)| known.key() == callee.key()) {
                Some((_, sites, _)) => sites.push(site),
                None => found.push((callee, vec![site], true)),
            }
        }
        for (_, sites, _) in &mut found {
            sites.sort_unstable();
            sites.dedup();
        }
        found.sort_by_key(|(callee, sites, _)| (sites.first().copied(), callee.name.clone()));
        Ok(found)
    }

    /// Callees from `callHierarchy/outgoingCalls`, a line per call.
    async fn outgoing(&mut self, item: Value, own: &Own) -> Result<Vec<(Def, u32)>> {
        let answer = self.request("callHierarchy/outgoingCalls", json!({ "item": item })).await?;
        let mut callees = Vec::new();
        for call in answer.as_array().into_iter().flatten() {
            let to = &call["to"];
            let Some(path) = to["uri"].as_str().and_then(lsp::path) else { continue };
            let Some(at) = to.get("selectionRange").and_then(Span::read) else { continue };
            if !self.in_project(&path) {
                continue;
            }
            let Some(mut callee) = self.declared(&path, at).await? else { continue };
            if callee.at == at.start {
                callee.item = Some(to.clone());
            }
            let sites = call["fromRanges"].as_array().into_iter().flatten().filter_map(Span::read);
            callees.extend(sites.filter(|s| own.has(s.start.line)).map(|s| (callee.clone(), s.start.line)));
        }
        Ok(callees)
    }

    /// Callees from the names the text calls, whose definition the server places in a
    /// function of the project.
    async fn defined(&mut self, def: &Def, own: &Own) -> Result<Vec<(Def, u32)>> {
        let text = self.text(&def.path)?;
        let mut lexer = Lexer::new(self.language);
        let mut calls: Vec<(u32, usize, String)> = Vec::new();
        for line in def.range.start.line..=def.range.end.line {
            let toks = lexer.line(text.line(line));
            if own.has(line) && (def.script || line != def.at.line) {
                calls.extend(text::calls(&toks).into_iter().map(|(name, at)| (line, at, name)));
            }
        }
        let mut decided: HashMap<String, Option<Def>> = HashMap::new();
        let mut callees = Vec::new();
        for (line, at, name) in calls {
            if !decided.contains_key(&name) {
                if decided.len() >= NAMES || self.spent(Phase::Down) {
                    continue;
                }
                let position = Pos::new(line, lsp::utf16(text.line(line), at));
                let params = json!({ "textDocument": lsp::document(&def.path), "position": position.json() });
                let answer = self.request("textDocument/definition", params).await.unwrap_or_default();
                let mut callee = None;
                for location in lsp::locations(&answer) {
                    if self.in_project(&location.path)
                        && let Some(found) = self.declared(&location.path, location.span).await?
                    {
                        callee = Some(found);
                        break;
                    }
                }
                decided.insert(name.clone(), callee);
            }
            if let Some(Some(callee)) = decided.get(&name) {
                callees.push((callee.clone(), line));
            }
        }
        Ok(callees)
    }

    /// Callees the stacks of earlier runs showed `def` calling, in the project's code.
    async fn seen_callees(&mut self, def: &Def, own: &Own) -> Result<Vec<(Def, u32)>> {
        let calls: Vec<(u32, String, PathBuf)> = self
            .observed
            .iter()
            .filter(|call| call.path == def.path && own.has(call.line.saturating_sub(1)))
            .filter(|call| self.in_project(&call.callee_path))
            .map(|call| (call.line.saturating_sub(1), call.callee.clone(), call.callee_path.clone()))
            .collect();
        let mut callees = Vec::new();
        for (line, name, path) in calls {
            if self.text(&path).is_err() {
                continue;
            }
            if let Some(callee) = self.named(&path, &name).await? {
                callees.push((callee, line));
            }
        }
        Ok(callees)
    }

    // ---- trees ------------------------------------------------------------------------

    /// The calls node `n` makes as far as the graph knows them, from either direction, the
    /// first in its text first.
    fn calls_of(&self, n: usize, into: &HashMap<usize, Vec<Edge>>) -> Vec<Edge> {
        let mut calls: Vec<Edge> = self.nodes[n].callees.clone();
        for edge in into.get(&n).into_iter().flatten() {
            match calls.iter_mut().find(|known| known.other == edge.other) {
                Some(known) => {
                    known.sites.extend(&edge.sites);
                    known.sites.sort_unstable();
                    known.sites.dedup();
                    known.observed &= edge.observed;
                }
                None => calls.push(edge.clone()),
            }
        }
        calls.sort_by_key(|edge| (edge.sites.first().copied(), self.nodes[edge.other].def.name.clone()));
        calls
    }

    /// A tree of calls from each root, depth first.
    fn unfold(&mut self, roots: &[(usize, Root)], spine: &HashSet<usize>, target: usize) -> Vec<Stage> {
        // The calls the ways up found, from the caller's side.
        let mut into: HashMap<usize, Vec<Edge>> = HashMap::new();
        for (to, node) in self.nodes.iter().enumerate() {
            for edge in &node.callers {
                let call = Edge { other: to, sites: edge.sites.clone(), observed: edge.observed };
                into.entry(edge.other).or_default().push(call);
            }
        }
        let mut out = Unfolding { stages: Vec::new(), full: false };
        for &(root, kind) in roots {
            let at = out.stages.len();
            let mut chain = Vec::new();
            let call = Edge { other: root, sites: Vec::new(), observed: false };
            self.grow(&mut out, &into, spine, target, &call, None, 0, &mut chain);
            if let Some(stage) = out.stages.get_mut(at) {
                stage.root = Some(kind);
            }
            if out.full {
                break;
            }
        }
        if out.full {
            self.notes.push(format!("the trees were cut at {} calls", self.limits.stages));
        }
        // What leads to an anchor, from the bottom up: children come after their parents.
        for i in (0..out.stages.len()).rev() {
            let leads = out.stages[i].anchor || out.stages[i].children.iter().any(|c| out.stages[*c].leads);
            out.stages[i].leads = leads;
        }
        out.stages
    }

    /// Adds the stage of `call` under `parent`, and below it the calls it makes. `below` is
    /// how many calls it is below the spine; `chain` the nodes above it.
    #[allow(clippy::too_many_arguments)]
    fn grow(
        &self,
        out: &mut Unfolding,
        into: &HashMap<usize, Vec<Edge>>,
        spine: &HashSet<usize>,
        target: usize,
        call: &Edge,
        parent: Option<usize>,
        below: usize,
        chain: &mut Vec<usize>,
    ) {
        if out.stages.len() >= self.limits.stages {
            out.full = true;
            if let Some(parent) = parent {
                out.stages[parent].cut = Some(Cut::Spent);
            }
            return;
        }
        let n = call.other;
        let index = out.stages.len();
        out.stages.push(Stage {
            function: n,
            parent,
            children: Vec::new(),
            end: index + 1,
            depth: chain.len(),
            sites: call.sites.iter().map(|line| line + 1).collect(),
            observed: call.observed,
            anchor: n == target,
            leads: false,
            cut: None,
            root: None,
        });
        if let Some(parent) = parent {
            out.stages[parent].children.push(index);
        }
        let below = if spine.contains(&n) { 0 } else { below };
        let calls = self.calls_of(n, into);
        let cut = if chain.contains(&n) {
            Some(Cut::Recursion)
        } else if below >= self.limits.below {
            (!calls.is_empty() || !self.nodes[n].down).then_some(Cut::Deep)
        } else {
            chain.push(n);
            for call in &calls {
                let next = if spine.contains(&call.other) { 0 } else { below + 1 };
                self.grow(out, into, spine, target, call, Some(index), next, chain);
                if out.full {
                    break;
                }
            }
            chain.pop();
            (!self.nodes[n].down).then_some(Cut::Spent)
        };
        let end = out.stages.len();
        let stage = &mut out.stages[index];
        stage.cut = stage.cut.or(cut);
        stage.end = end;
    }

    /// The answer: the stages with their functions, and the lines of each function read.
    async fn found(mut self, mut stages: Vec<Stage>, target: usize, line: u32) -> Result<Found> {
        // Functions in the order the trees show them.
        let mut order: Vec<usize> = Vec::new();
        for stage in &mut stages {
            let n = stage.function;
            stage.function = order.iter().position(|known| *known == n).unwrap_or_else(|| {
                order.push(n);
                order.len() - 1
            });
        }
        if !order.contains(&target) {
            order.push(target);
        }
        // Only a function that calls on, or is the target's, has state worth reading.
        let mut functions = Vec::new();
        let mut unread = 0;
        for (k, &n) in order.iter().enumerate() {
            let state = n == target || stages.iter().any(|s| s.function == k && !s.children.is_empty());
            let def = self.nodes[n].def.clone();
            if state && self.spent(Phase::Lines) {
                unread += 1;
            }
            let facts = self.facts(n, state).await;
            let line = if def.script { 1 } else { def.at.line + 1 };
            functions.push(Function { name: def.name, short: def.short, path: def.path, line, facts });
        }
        if unread > 0 {
            self.notes.push(format!("out of requests for lines: the writes of {unread} functions come from the text"));
        }
        let target = order.iter().position(|n| *n == target).unwrap_or_default();
        Ok(Found {
            functions,
            target,
            line: line + 1,
            stages,
            notes: self.notes,
            unseen: unseen(self.language).to_owned(),
            requests: self.requests,
            state: self.limits.state,
        })
    }

    /// What node `n`'s lines do: its entry, and with `state` which lines write and call into
    /// the project.
    async fn facts(&mut self, n: usize, state: bool) -> Facts {
        let def = self.nodes[n].def.clone();
        if state {
            (self.progress)(format!("lines of {}", def.name));
        }
        match self.read_facts(n, &def, state).await {
            Ok(facts) => facts,
            Err(err) => {
                self.notes.push(format!("lines of {}: {err:#}", def.name));
                Facts::default()
            }
        }
    }

    async fn read_facts(&mut self, n: usize, def: &Def, state: bool) -> Result<Facts> {
        let text = self.text(&def.path)?;
        let entry = (!def.script).then(|| match self.language {
            // Delve takes only a line with a statement, and the first line of a body can be
            // part of one that goes on over lines; on the declaration it stops as the
            // function starts.
            Language::Go => def.at.line,
            _ => {
                let at = (def.at.line, lsp::byte(text.line(def.at.line), def.at.character));
                text::entry(&text, self.language, at, def.range.end.line)
            }
        });
        if !state {
            return Ok(Facts { entry, writes: Vec::new(), uses: Vec::new() });
        }
        let own = self.own(def).await?;
        let writes = self.writes(def, &text, &own).await;
        let uses = if def.script { Vec::new() } else { self.nodes[n].uses.clone() };
        Ok(Facts { entry, writes, uses })
    }

    /// The lines that write a variable or a field, with its name. The text says which names a
    /// line assigns to; the server, asked for the highlights of each, which occurrences are
    /// the same variable and, if it tells writes from reads, which of them write.
    async fn writes(&mut self, def: &Def, text: &Text, own: &Own) -> Vec<(u32, String)> {
        let mut lexer = Lexer::new(self.language);
        let mut candidates: Vec<(u32, usize, String)> = Vec::new();
        for line in def.range.start.line..=def.range.end.line {
            let toks = lexer.line(text.line(line));
            // Delve takes no line in the middle of a statement, nor the first of one that
            // goes on over lines.
            if own.has(line) && !(self.language == Language::Go && text::continues(&toks)) {
                candidates.extend(text::writes(&toks).into_iter().map(|(name, at)| (line, at, name)));
            }
        }
        let mut covered: HashSet<(u32, usize)> = HashSet::new();
        let mut found = Vec::new();
        let mut asked = 0;
        for (line, at, name) in &candidates {
            if covered.contains(&(*line, *at)) {
                continue;
            }
            if asked >= LINE_REQUESTS || self.spent(Phase::Lines) {
                found.push((*line, name.clone()));
                continue;
            }
            asked += 1;
            let position = Pos::new(*line, lsp::utf16(text.line(*line), *at));
            let params = json!({ "textDocument": lsp::document(&def.path), "position": position.json() });
            let highlights = match self.request("textDocument/documentHighlight", params).await {
                Ok(answer) => lsp::highlights(&answer),
                // Without an answer, the text is all there is.
                Err(_) => {
                    found.push((*line, name.clone()));
                    continue;
                }
            };
            let by_kind = highlights.iter().any(|h| h.kind == WRITE);
            for highlight in highlights.iter().filter(|h| own.has(h.span.start.line)) {
                let start = highlight.span.start;
                let byte = lsp::byte(text.line(start.line), start.character);
                covered.insert((start.line, byte));
                let lexical = candidates.iter().any(|(l, b, _)| *l == start.line && *b == byte);
                if if by_kind { highlight.kind == WRITE } else { lexical } {
                    found.push((start.line, name.clone()));
                }
            }
        }
        found.sort();
        found.dedup();
        found
    }

    /// Code of the project, not of a library it has installed in it.
    fn in_project(&self, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(&self.root) else { return false };
        !relative.iter().any(|part| part == "vendor" || part == "node_modules")
    }
}

/// The calls no server sees in `language`, for a run to show.
fn unseen(language: Language) -> &'static str {
    match language {
        Language::Php => "calls a container, a router or an event dispatcher makes, and calls of callables",
        Language::TypeScript => {
            "calls from templates (Angular's bindings and events), through DI tokens, callbacks and events"
        }
        Language::Rust => "calls through trait objects, function pointers and closures passed on, and calls in macros",
        Language::Go => "calls through interfaces and function values",
    }
}

fn is_function(symbol: &Symbol) -> bool {
    matches!(symbol.kind, lsp::FUNCTION | lsp::METHOD | lsp::CONSTRUCTOR) && named(&symbol.name)
}

/// Not a closure: servers name those `map() callback`, `<function>`, `{closure}`.
fn named(name: &str) -> bool {
    !name.is_empty() && !name.contains(' ') && !name.starts_with(['<', '{'])
}

/// The type an `impl` block is for: `impl Display for Order` → `Order`.
fn owner(name: &str) -> String {
    let Some(rest) = name.strip_prefix("impl") else { return name.to_owned() };
    // Past `impl<T>`'s type parameters.
    let mut depth = 0;
    let start = rest.char_indices().find(|&(_, c)| {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            _ => {}
        }
        depth == 0 && c != '>'
    });
    let rest = start.map_or("", |(at, _)| &rest[at..]);
    rest.rsplit(" for ").next().unwrap_or(rest).trim().to_owned()
}

/// Whether a `callHierarchy` item stands for a whole file: TypeScript's script top level is
/// an item named after the file.
fn is_script_item(item: &Value, path: &Path) -> bool {
    let kind = item["kind"].as_u64().unwrap_or(0);
    kind == lsp::FILE || kind == lsp::MODULE || item["name"].as_str() == Some(file_name(path).as_str())
}

fn collect_functions(symbols: &[Symbol], out: &mut Vec<Span>) {
    for symbol in symbols {
        if is_function(symbol) {
            out.push(symbol.range);
        } else {
            collect_functions(&symbol.children, out);
        }
    }
}

fn flatten<'a>(symbols: &'a [Symbol], out: &mut Vec<&'a Symbol>) {
    for symbol in symbols {
        out.push(symbol);
        flatten(&symbol.children, out);
    }
}

/// The name a debugger gives a function, without what it belongs to:
/// `App\Event\LoyaltyListener->onOrderTotalled`, `orders::order_total`, `main.orderTotal`,
/// `App.orderTotal` → the last part.
pub(crate) fn short_name(name: &str) -> String {
    // Without type arguments, whose paths have separators too, and a list of parameters.
    let mut bare = String::with_capacity(name.len());
    let mut depth = 0usize;
    let mut previous = ' ';
    for c in name.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 && previous != '-' => depth -= 1,
            _ if depth == 0 => bare.push(c),
            _ => {}
        }
        previous = c;
    }
    if let Some(open) = bare.strip_suffix(')').and_then(|rest| rest.rfind('(')) {
        bare.truncate(open);
    }
    let cut = [bare.rfind("->").map(|i| i + 2), bare.rfind("::").map(|i| i + 2), bare.rfind('.').map(|i| i + 1)];
    cut.into_iter().flatten().max().map_or(bare.clone(), |at| bare[at..].to_owned())
}

fn lines(span: Span) -> u32 {
    span.end.line.saturating_sub(span.start.line)
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::select::Why;
    use crate::tree::Stop;

    /// A project in a temporary directory, removed when dropped.
    struct Project(PathBuf);

    impl Project {
        fn new(name: &str, files: &[(&str, &str)]) -> Project {
            // Tests run at once: each its own directory.
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("tuitor-code-{name}-{}-{n}", std::process::id()));
            for (file, text) in files {
                let path = dir.join(file);
                std::fs::create_dir_all(path.parent().expect("dir")).expect("dir");
                std::fs::write(path, text).expect("write");
            }
            Project(dir)
        }

        fn path(&self, file: &str) -> PathBuf {
            self.0.join(file)
        }

        fn uri(&self, file: &str) -> String {
            lsp::uri(&self.path(file))
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    /// Answers as a server would, by method and the document, line and, if given, column
    /// asked about; asked anything else, it knows nothing. It counts what it is asked.
    struct Fake {
        call_hierarchy: bool,
        answers: Vec<(&'static str, String, u32, Option<u32>, Value)>,
        asked: usize,
    }

    impl Fake {
        fn new(call_hierarchy: bool) -> Fake {
            Fake { call_hierarchy, answers: Vec::new(), asked: 0 }
        }

        fn answer(&mut self, method: &'static str, uri: String, line: u32, answer: Value) {
            self.answers.push((method, uri, line, None, answer));
        }

        fn answer_at(&mut self, method: &'static str, uri: String, (line, column): (u32, u32), answer: Value) {
            self.answers.push((method, uri, line, Some(column), answer));
        }
    }

    impl Ask for Fake {
        fn call_hierarchy(&self) -> bool {
            self.call_hierarchy
        }

        fn open(&mut self, _: &Path, _: &Text) -> Result<()> {
            Ok(())
        }

        async fn ask(&mut self, method: &'static str, params: Value) -> Result<Value> {
            self.asked += 1;
            let (document, at) = match params.get("item") {
                Some(item) => (&item["uri"], &item["selectionRange"]["start"]),
                None => (&params["textDocument"]["uri"], &params["position"]),
            };
            let number = |key: &str| at[key].as_u64().and_then(|n| u32::try_from(n).ok());
            let (line, column) = (number("line"), number("character"));
            let answer = self.answers.iter().find(|(m, uri, l, c, _)| {
                *m == method
                    && Some(uri.as_str()) == document.as_str()
                    && (line.is_none() || line == Some(*l))
                    && (c.is_none() || *c == column)
            });
            Ok(answer.map_or(Value::Null, |(.., answer)| answer.clone()))
        }
    }

    fn span(line: u32, from: u32, to_line: u32, to: u32) -> Value {
        json!({ "start": { "line": line, "character": from }, "end": { "line": to_line, "character": to } })
    }

    /// A symbol from line `start` to `end`, named at `name` on line `at`.
    fn symbol(name: &str, kind: u64, (start, end): (u32, u32), (at, column): (u32, u32), children: Value) -> Value {
        let size = u32::try_from(name.len()).expect("short");
        json!({
            "name": name, "kind": kind,
            "range": span(start, 0, end, 1),
            "selectionRange": span(at, column, at, column + size),
            "children": children,
        })
    }

    /// Where `name` is on `line` of `file`, as a location.
    fn refer(project: &Project, file: &str, line: u32, name: &str) -> Value {
        let (column, size) = place(project, file, line, name);
        json!({ "uri": project.uri(file), "range": span(line, column, line, column + size) })
    }

    /// The column of `name` on `line` of `file`, and its length.
    fn place(project: &Project, file: &str, line: u32, name: &str) -> (u32, u32) {
        let text = std::fs::read_to_string(project.path(file)).expect("file");
        let column = text.lines().nth(line as usize).and_then(|l| l.find(name)).expect("on the line");
        (u32::try_from(column).expect("short"), u32::try_from(name.len()).expect("short"))
    }

    fn highlights(list: &[(u32, u32, u64)]) -> Value {
        Value::Array(
            list.iter()
                .map(|&(line, column, kind)| json!({ "range": span(line, column, line, column + 4), "kind": kind }))
                .collect(),
        )
    }

    async fn ask(
        fake: &mut Fake,
        project: &Project,
        file: &str,
        line: u32,
        observed: &[Observed],
        limits: Limits,
    ) -> Found {
        let question = Question { root: &project.0, file: &project.path(file), line, observed, limits };
        find(fake, Language::Php, question, |_| {}).await.expect("paths")
    }

    /// The trees as `depth name` lines, with `◆` for an anchor, `⇢` for a call a run showed
    /// and the cut, if any.
    fn tree(found: &Found) -> Vec<String> {
        let line = |(i, stage): (usize, &Stage)| {
            let mark = if stage.anchor {
                "◆ "
            } else if stage.observed {
                "⇢ "
            } else {
                ""
            };
            let cut = stage.cut.map(|cut| format!(" ({cut:?})")).unwrap_or_default();
            format!("{}{mark}{}{cut}", "  ".repeat(stage.depth), found.function(i).name)
        };
        found.stages.iter().enumerate().map(line).collect()
    }

    fn e2e(found: &Found) -> Vec<Vec<&str>> {
        let names = |leaf: usize| found.chain(leaf).iter().map(|s| found.function(*s).name.as_str()).collect();
        found.paths().into_iter().map(names).collect()
    }

    fn stage(found: &Found, name: &str, nth: usize) -> usize {
        let all = (0..found.stages.len()).filter(|s| found.function(*s).name == name);
        all.into_iter().nth(nth).unwrap_or_else(|| panic!("{name} #{nth}"))
    }

    fn stops(stops: &[Stop]) -> Vec<(String, u32, Why)> {
        let file = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        stops.iter().map(|stop| (file(&stop.path), stop.line, stop.why.clone())).collect()
    }

    const MONEY: &str = "<?php
final class Money
{
    public static function lineTotal(Line $line): int
    {
        $amount = $line->qty * $line->price;
        if ($line->qty >= 10) {
            $amount = intdiv($amount * 95, 100);
        }
        return $amount;
    }
    public static function format(int $cents): string
    {
        return sprintf('%d', $cents);
    }
}
";
    const SERVICE: &str = "<?php
final class OrderService
{
    public function totalFor(int $id): int
    {
        $order = $this->orders->find($id);
        $total = 0;
        foreach ($order->lines as $line) {
            $total += Money::lineTotal($line);
        }
        $this->events->dispatch($order);
        return $total;
    }
}
";
    const CONTROLLER: &str = "<?php
final class OrderController
{
    public function show(int $id): string
    {
        $total = $this->orders->totalFor($id);
        return Money::format($total);
    }
}
";
    const REPOSITORY: &str = "<?php
final class OrderRepository
{
    public function find(int $id): Order
    {
        return $this->load($id);
    }
    public function load(int $id): Order
    {
        return $this->find($id);
    }
}
";
    const KERNEL: &str = "<?php
$events->listen('order', [$money, 'lineTotal']);
";
    const TEST: &str = "<?php
final class MoneyTest
{
    public function testBulk(): void
    {
        assert(Money::lineTotal(new Line('MS-02', 12, 2450)) === 27930);
    }
}
";
    const LISTENER: &str = "<?php
final class Listener
{
    public function onEvent(object $event): void
    {
        $this->points += Money::lineTotal($event->line);
    }
}
";
    const DISPATCHER: &str = "<?php
final class Dispatcher
{
    public function dispatch(object $event): void
    {
        foreach ($this->listeners as $listener) {
            $listener($event);
        }
    }
}
";

    /// A server without call hierarchy, as intelephense: references up, definitions down,
    /// symbols and highlights.
    fn php_server(project: &Project) -> Fake {
        let mut fake = Fake::new(false);
        let method = |name: &str, (start, end): (u32, u32), at: u32| symbol(name, 6, (start, end), (at, 20), json!([]));
        let class = |name: &str, end: u32, methods: Vec<Value>| symbol(name, 5, (1, end), (1, 12), json!(methods));
        let files = [
            (
                "src/Money.php",
                class("Money", 15, vec![method("lineTotal", (3, 10), 3), method("format", (11, 14), 11)]),
            ),
            ("src/OrderService.php", class("OrderService", 13, vec![method("totalFor", (3, 12), 3)])),
            ("src/Controller/OrderController.php", class("OrderController", 8, vec![method("show", (3, 7), 3)])),
            (
                "src/OrderRepository.php",
                class("OrderRepository", 11, vec![method("find", (3, 6), 3), method("load", (7, 10), 7)]),
            ),
            ("tests/MoneyTest.php", class("MoneyTest", 7, vec![method("testBulk", (3, 6), 3)])),
            ("src/Listener.php", class("Listener", 7, vec![method("onEvent", (3, 6), 3)])),
            ("src/Dispatcher.php", class("Dispatcher", 9, vec![method("dispatch", (3, 8), 3)])),
        ];
        for (file, symbols) in files {
            fake.answer("textDocument/documentSymbol", project.uri(file), 0, json!([symbols]));
        }
        fake.answer("textDocument/documentSymbol", project.uri("src/Kernel.php"), 0, json!([]));
        let uri = |file: &str| project.uri(file);
        fake.answer(
            "textDocument/references",
            uri("src/Money.php"),
            3,
            json!([
                refer(project, "src/OrderService.php", 8, "lineTotal"),
                refer(project, "src/Kernel.php", 1, "lineTotal"),
                refer(project, "tests/MoneyTest.php", 5, "lineTotal"),
                refer(project, "src/Listener.php", 5, "lineTotal"),
            ]),
        );
        fake.answer(
            "textDocument/references",
            uri("src/OrderService.php"),
            3,
            json!([refer(project, "src/Controller/OrderController.php", 5, "totalFor")]),
        );
        fake.answer(
            "textDocument/references",
            uri("src/Dispatcher.php"),
            3,
            json!([refer(project, "src/OrderService.php", 10, "dispatch")]),
        );
        // Definitions of what the text calls: the declarations' names.
        let definitions = [
            ("src/Controller/OrderController.php", 5, "totalFor", "src/OrderService.php", 3),
            ("src/Controller/OrderController.php", 6, "format", "src/Money.php", 11),
            ("src/OrderService.php", 5, "find", "src/OrderRepository.php", 3),
            ("src/OrderService.php", 8, "lineTotal", "src/Money.php", 3),
            ("src/OrderService.php", 10, "dispatch", "src/Dispatcher.php", 3),
            ("src/OrderRepository.php", 5, "load", "src/OrderRepository.php", 7),
            ("src/OrderRepository.php", 9, "find", "src/OrderRepository.php", 3),
            ("src/Listener.php", 5, "lineTotal", "src/Money.php", 3),
            ("tests/MoneyTest.php", 5, "lineTotal", "src/Money.php", 3),
        ];
        for (file, line, name, to, at) in definitions {
            let (column, _) = place(project, file, line, name);
            let target = refer(project, to, at, name.trim_start_matches('$'));
            fake.answer_at("textDocument/definition", uri(file), (line, column), json!([target]));
        }
        // `$listener($event)`: the variable, no function.
        let (column, _) = place(project, "src/Dispatcher.php", 6, "$listener");
        let variable = refer(project, "src/Dispatcher.php", 5, "$listener");
        fake.answer_at("textDocument/definition", uri("src/Dispatcher.php"), (6, column), json!([variable]));
        fake.answer(
            "textDocument/documentHighlight",
            uri("src/Money.php"),
            5,
            highlights(&[(5, 8, 3), (7, 12, 3), (7, 29, 2), (9, 15, 2)]),
        );
        fake.answer(
            "textDocument/documentHighlight",
            uri("src/OrderService.php"),
            5,
            highlights(&[(5, 8, 3), (7, 17, 2), (10, 32, 2)]),
        );
        fake.answer(
            "textDocument/documentHighlight",
            uri("src/OrderService.php"),
            6,
            highlights(&[(6, 8, 3), (8, 12, 3), (11, 15, 2)]),
        );
        fake
    }

    fn php_project() -> Project {
        Project::new(
            "php",
            &[
                ("src/Money.php", MONEY),
                ("src/OrderService.php", SERVICE),
                ("src/Controller/OrderController.php", CONTROLLER),
                ("src/OrderRepository.php", REPOSITORY),
                ("src/Kernel.php", KERNEL),
                ("tests/MoneyTest.php", TEST),
                ("src/Listener.php", LISTENER),
                ("src/Dispatcher.php", DISPATCHER),
            ],
        )
    }

    /// The call the dispatcher makes that only a run shows.
    fn dispatched(project: &Project) -> Observed {
        Observed {
            path: project.path("src/Dispatcher.php"),
            line: 7,
            callee: "App\\Listener->onEvent".into(),
            callee_path: project.path("src/Listener.php"),
        }
    }

    #[tokio::test]
    async fn up_to_where_execution_starts_then_down_into_what_each_step_calls() {
        let project = php_project();
        let mut fake = php_server(&project);
        let found = ask(&mut fake, &project, "src/Money.php", 8, &[], Limits::default()).await;
        assert_eq!(found.target().name, "Money::lineTotal");
        // The controller first and the test last; the callback registered by name calls
        // nothing. Below the controller, the service and the helper's format; below the
        // service, the repository (whose two methods call each other), the anchor and the
        // dispatcher, which calls no function the server can name.
        assert_eq!(
            tree(&found),
            [
                "OrderController::show",
                "  OrderService::totalFor",
                "    OrderRepository::find",
                "      OrderRepository::load",
                "        OrderRepository::find (Recursion)",
                "    ◆ Money::lineTotal",
                "    Dispatcher::dispatch",
                "  Money::format",
                "Listener::onEvent",
                "  ◆ Money::lineTotal",
                "MoneyTest::testBulk",
                "  ◆ Money::lineTotal",
            ]
        );
        let roots: Vec<Option<Root>> = found.roots().map(|r| found.stages[r].root).collect();
        assert_eq!(roots, [Some(Root::Entry(Entry::Controller)), Some(Root::Uncalled), Some(Root::Entry(Entry::Test))]);
        assert_eq!(
            e2e(&found),
            [
                vec!["OrderController::show", "OrderService::totalFor", "Money::lineTotal"],
                vec!["Listener::onEvent", "Money::lineTotal"],
                vec!["MoneyTest::testBulk", "Money::lineTotal"],
            ]
        );
        // The calls of the service and the sites they are made at; the controller's.
        let total = stage(&found, "OrderService::totalFor", 0);
        let sites: Vec<(&str, Vec<u32>)> = found.stages[total]
            .children
            .iter()
            .map(|c| (found.function(*c).short.as_str(), found.stages[*c].sites.clone()))
            .collect();
        assert_eq!(sites, [("find", vec![6]), ("lineTotal", vec![9]), ("dispatch", vec![11])]);
        assert!(found.on_path(total) && !found.on_path(stage(&found, "Money::format", 0)));
        assert_eq!(found.forks(total), 1);
    }

    #[tokio::test]
    async fn a_call_a_run_showed_joins_both_ways_and_the_paths_part_where_it_leaves() {
        let project = php_project();
        let mut fake = php_server(&project);
        let found = ask(&mut fake, &project, "src/Money.php", 8, &[dispatched(&project)], Limits::default()).await;
        let show = &tree(&found)[..9];
        assert_eq!(
            show,
            [
                "OrderController::show",
                "  OrderService::totalFor",
                "    OrderRepository::find",
                "      OrderRepository::load",
                "        OrderRepository::find (Recursion)",
                "    ◆ Money::lineTotal",
                "    Dispatcher::dispatch",
                "      ⇢ Listener::onEvent",
                "        ◆ Money::lineTotal",
            ]
        );
        // The listener is called now: no tree of its own.
        assert_eq!(found.roots().count(), 2);
        let total = stage(&found, "OrderService::totalFor", 0);
        assert_eq!(found.forks(total), 2);
        assert!(e2e(&found).contains(&vec![
            "OrderController::show",
            "OrderService::totalFor",
            "Dispatcher::dispatch",
            "Listener::onEvent",
            "Money::lineTotal"
        ]));
        // The path through a stage goes on down the first call on a path.
        let listener = stage(&found, "Listener::onEvent", 0);
        let through: Vec<&str> = found
            .path_through(stage(&found, "Dispatcher::dispatch", 0))
            .iter()
            .map(|s| found.function(*s).short.as_str())
            .collect();
        assert_eq!(through, ["show", "totalFor", "dispatch", "onEvent", "lineTotal"]);
        assert!(found.stages[listener].observed);
    }

    #[tokio::test]
    async fn the_lines_of_a_stage_follow_the_calls_chosen_below_it() {
        let project = php_project();
        let mut fake = php_server(&project);
        let found = ask(&mut fake, &project, "src/Money.php", 8, &[dispatched(&project)], Limits::default()).await;
        let stop = |file: &str, line: u32, why: Why| (file.to_owned(), line, why);
        // The path straight to the helper: today's lines.
        let path: BTreeSet<usize> = found.path_through(stage(&found, "Money::lineTotal", 0)).into_iter().collect();
        assert_eq!(
            stops(&found.selection(&path)),
            [
                stop("OrderController.php", 6, Why::Calls("totalFor".into())),
                // `$order = $this->orders->find($id)` is the entry, and `find` the project's.
                stop("OrderService.php", 6, Why::Entry),
                stop("OrderService.php", 7, Why::Writes("$total".into())),
                stop("OrderService.php", 9, Why::Calls("lineTotal".into())),
                stop("Money.php", 6, Why::Entry),
                stop("Money.php", 8, Why::Target),
            ]
        );
        // With the dispatcher chosen too, the service's lines go on to its call, past the
        // last write of $total, and the dispatcher, a stage with nothing chosen below it,
        // keeps its entry.
        let mut more = path.clone();
        more.insert(stage(&found, "Dispatcher::dispatch", 0));
        let lines = stops(&found.selection(&more));
        assert!(lines.contains(&stop("OrderService.php", 11, Why::Calls("dispatch".into()))), "{lines:?}");
        assert!(lines.contains(&stop("OrderService.php", 9, Why::Calls("lineTotal".into()))), "{lines:?}");
        assert!(lines.contains(&stop("Dispatcher.php", 6, Why::Entry)), "{lines:?}");
        assert!(!lines.contains(&stop("OrderService.php", 7, Why::Writes("$total".into()))), "{lines:?}");
        // A stage alone, its calls left out: its entry, and the target in an anchor.
        let alone: BTreeSet<usize> = [stage(&found, "Money::lineTotal", 1)].into();
        assert_eq!(
            stops(&found.selection(&alone)),
            [stop("Money.php", 6, Why::Entry), stop("Money.php", 8, Why::Target)]
        );
        // A line two stages share is one stop.
        let both: BTreeSet<usize> = [stage(&found, "Money::lineTotal", 0), stage(&found, "Money::lineTotal", 1)].into();
        assert_eq!(found.selection(&both).len(), 2);
    }

    #[tokio::test]
    async fn the_limits_cut_the_trees_and_say_so() {
        let project = php_project();
        let mut fake = php_server(&project);
        // Up.
        let limits = Limits { depth: 1, ..Limits::default() };
        let found = ask(&mut fake, &project, "src/Money.php", 8, &[], limits).await;
        let cut: Vec<usize> = found.roots().filter(|r| found.stages[*r].root == Some(Root::Cut)).collect();
        let names: Vec<&str> = cut.iter().map(|r| found.function(*r).name.as_str()).collect();
        assert_eq!(names, ["OrderService::totalFor", "Listener::onEvent"]);
        let note = "2 functions at the limit of 1 call up: their callers were not asked for";
        assert!(found.notes.contains(&note.to_owned()), "{:?}", found.notes);
        // Down: one call below the paths, and what it calls not asked for.
        let limits = Limits { below: 1, ..Limits::default() };
        let found = ask(&mut fake, &project, "src/Money.php", 8, &[], limits).await;
        let find = stage(&found, "OrderRepository::find", 0);
        assert_eq!((found.stages[find].cut, found.stages[find].children.len()), (Some(Cut::Deep), 0));
        let note = "3 functions 1 call below the paths: what they call was not asked for (…)";
        assert!(found.notes.contains(&note.to_owned()), "{:?}", found.notes);
        // A call per function: the service's first, and the anchor, which the way up knows.
        let limits = Limits { callees: 1, ..Limits::default() };
        let found = ask(&mut fake, &project, "src/Money.php", 8, &[], limits).await;
        let total = stage(&found, "OrderService::totalFor", 0);
        let calls: Vec<&str> = found.stages[total].children.iter().map(|c| found.function(*c).short.as_str()).collect();
        assert_eq!(calls, ["find", "lineTotal"]);
        assert!(found.notes.iter().any(|n| n.starts_with("only the first 1 calls followed of")), "{:?}", found.notes);
        // Roots, and stages.
        let limits = Limits { roots: 1, stages: 4, ..Limits::default() };
        let found = ask(&mut fake, &project, "src/Money.php", 8, &[], limits).await;
        assert_eq!((found.roots().count(), found.stages.len()), (1, 4));
        assert!(found.notes.contains(&"2 more places execution starts not shown".to_owned()), "{:?}", found.notes);
        assert!(found.notes.contains(&"the trees were cut at 4 calls".to_owned()), "{:?}", found.notes);
        assert_eq!(found.stages[stage(&found, "OrderRepository::load", 0)].cut, Some(Cut::Spent));
    }

    #[tokio::test]
    async fn the_requests_are_bounded() {
        let project = php_project();
        let mut fake = php_server(&project);
        let all = ask(&mut fake, &project, "src/Money.php", 8, &[], Limits::default()).await;
        assert!(all.notes.iter().all(|n| !n.starts_with("stopped")), "{:?}", all.notes);
        let mut fake = php_server(&project);
        let limits = Limits { requests: 3, lines: 2, ..Limits::default() };
        let found = ask(&mut fake, &project, "src/Money.php", 8, &[], limits).await;
        // A function's question is asked whole: its references, and the symbols of the files
        // they are in.
        assert!(fake.asked < all.requests / 2, "{} requests of {}", fake.asked, all.requests);
        assert_eq!(found.requests, fake.asked);
        assert!(found.notes.iter().any(|n| n.starts_with("stopped asking for callers after 3")), "{:?}", found.notes);
        assert!(found.notes.iter().any(|n| n.starts_with("stopped asking what functions call")), "{:?}", found.notes);
    }

    const SCRIPT: &str = "function leaf(x) {
  let total = 0;
  total += x;
  return total;
}
function mid(x) {
  return leaf(x);
}
function top() {
  return mid(1);
}
function rec(n) {
  return n > 0 ? rec(n - 1) : mid(n);
}
top();
";

    /// A server with call hierarchy, as typescript-language-server: the script's top level
    /// is an item named after the file, and highlights tell no writes.
    #[tokio::test]
    async fn call_hierarchy_leads_up_through_recursion_to_the_script_and_down_again() {
        let project = Project::new("ts", &[("a.ts", SCRIPT)]);
        let uri = project.uri("a.ts");
        let mut fake = Fake::new(true);
        let function = |name: &str, (start, end): (u32, u32)| symbol(name, 12, (start, end), (start, 9), json!([]));
        let symbols = json!([
            function("leaf", (0, 4)),
            function("mid", (5, 7)),
            function("top", (8, 10)),
            function("rec", (11, 13))
        ]);
        fake.answer("textDocument/documentSymbol", uri.clone(), 0, symbols);
        let item = |name: &str, line: u32| json!({ "name": name, "kind": 12, "uri": uri, "range": span(line, 0, line, 1), "selectionRange": span(line, 9, line, 13) });
        let script = json!({ "name": "a.ts", "kind": 13, "uri": uri, "range": span(0, 0, 14, 0), "selectionRange": span(0, 0, 0, 0) });
        let from = |from: Value, line: u32| json!({ "from": from, "fromRanges": [span(line, 9, line, 12)] });
        let to = |to: Value, line: u32| json!({ "to": to, "fromRanges": [span(line, 9, line, 12)] });
        fake.answer("textDocument/prepareCallHierarchy", uri.clone(), 0, json!([item("leaf", 0)]));
        fake.answer("callHierarchy/incomingCalls", uri.clone(), 0, json!([from(item("mid", 5), 6)]));
        fake.answer(
            "callHierarchy/incomingCalls",
            uri.clone(),
            5,
            json!([from(item("top", 8), 9), from(item("rec", 11), 12)]),
        );
        fake.answer("callHierarchy/incomingCalls", uri.clone(), 8, json!([from(script.clone(), 14)]));
        fake.answer("callHierarchy/incomingCalls", uri.clone(), 11, json!([from(item("rec", 11), 12)]));
        fake.answer_at("callHierarchy/outgoingCalls", uri.clone(), (0, 0), json!([to(item("top", 8), 14)]));
        fake.answer("callHierarchy/outgoingCalls", uri.clone(), 8, json!([to(item("mid", 5), 9)]));
        fake.answer("callHierarchy/outgoingCalls", uri.clone(), 5, json!([to(item("leaf", 0), 6)]));
        fake.answer(
            "callHierarchy/outgoingCalls",
            uri.clone(),
            11,
            json!([to(item("rec", 11), 12), to(item("mid", 5), 12)]),
        );
        // Every occurrence a read: the text tells the writes.
        fake.answer("textDocument/documentHighlight", uri.clone(), 1, highlights(&[(1, 6, 2), (2, 2, 2), (3, 9, 2)]));
        let question = Question {
            root: &project.0,
            file: &project.path("a.ts"),
            line: 3,
            observed: &[],
            limits: Limits::default(),
        };
        let found = find(&mut fake, Language::TypeScript, question, |_| {}).await.expect("paths");
        assert_eq!(
            tree(&found),
            ["a.ts", "  top", "    mid", "      ◆ leaf", "rec", "  mid", "    ◆ leaf", "  rec (Recursion)"]
        );
        assert_eq!(found.stages[0].root, Some(Root::Entry(Entry::Script)));
        assert_eq!(found.stages[4].root, Some(Root::Uncalled));
        let path: BTreeSet<usize> = found.path_through(0).into_iter().collect();
        let lines: Vec<(u32, Why)> = found.selection(&path).into_iter().map(|s| (s.line, s.why)).collect();
        assert_eq!(
            lines,
            [
                (15, Why::Calls("top".into())),
                (10, Why::Calls("mid".into())),
                (7, Why::Calls("leaf".into())),
                (2, Why::Entry),
                (3, Why::Target)
            ]
        );
    }

    #[test]
    fn debugger_names_come_down_to_the_function() {
        assert_eq!(short_name("App\\Event\\LoyaltyListener->onOrderTotalled"), "onOrderTotalled");
        assert_eq!(short_name("App\\Helper\\Money::lineTotal"), "lineTotal");
        assert_eq!(short_name("orders::order_total"), "order_total");
        assert_eq!(short_name("main.(*Shop).total"), "total");
        assert_eq!(short_name("App.orderTotal"), "orderTotal");
        assert_eq!(short_name("lineTotal"), "lineTotal");
        assert_eq!(short_name("core::iter::Iterator::fold<core::slice::Iter<Line>, ()>"), "fold");
        assert_eq!(short_name("orders::total(order: &Order)"), "total");
        assert_eq!(owner("impl Display for Order"), "Order");
        assert_eq!(owner("impl<T> Basket<T>"), "Basket<T>");
    }
}
