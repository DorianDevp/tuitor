//! Paths to a line: the chains of calls from where execution starts (a controller, `main`, a
//! test, a script's top level, or a function nothing calls) down to the function the line is
//! in, found by asking a language server who calls each function in turn; and on each path
//! the lines worth a breakpoint.
//!
//! Callers come from `callHierarchy/incomingCalls`, or, from a server without it, from the
//! `textDocument/references` to a function that call it, each put in the function
//! `textDocument/documentSymbol` says it is in. A call in a closure counts as one from the
//! function the closure is written in. Calls the stacks of earlier runs showed join them,
//! marked as seen at run time: no server sees what a container, an event or an interface
//! calls.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::config::Language;
use crate::lsp::{self, Pos, Span, Symbol, WRITE};
use crate::select::{self, Facts, Why};
use crate::text::{self, Lexer, Text};

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
    /// Functions above the target on a path.
    pub depth: usize,
    /// Paths kept, those nearest an entry point first.
    pub paths: usize,
    /// Callers followed per function.
    pub callers: usize,
    /// Requests for callers; past them the paths found so far are the answer.
    pub requests: usize,
    /// Lines of state kept per function, besides its entry and calls.
    pub state: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { depth: 8, paths: 8, callers: 10, requests: 300, state: 3 }
    }
}

/// Requests for the lines of each function, besides those for callers.
const LINE_REQUESTS: usize = 16;
/// Partial chains kept while paths are put together, so that a dense graph cannot run away.
const CHAINS: usize = 4096;

/// A call an earlier run's stack showed: from `line` (1-based) of `path`, into the function
/// named `callee` (as the debugger names it) in `callee_path`.
#[derive(Debug, Clone, PartialEq)]
pub struct Observed {
    pub path: PathBuf,
    pub line: u32,
    pub callee: String,
    pub callee_path: PathBuf,
}

/// A function on a path.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// With its class or type: `OrderService::totalFor`.
    pub name: String,
    pub path: PathBuf,
    /// The line of its name, 1-based.
    pub line: u32,
    /// The call into it from the step before was seen at run time, not by the server.
    pub observed: bool,
}

/// A line on a path worth a breakpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Stop {
    pub path: PathBuf,
    /// 1-based.
    pub line: u32,
    pub why: Why,
    /// The index of the step it is in.
    pub step: usize,
}

/// Where execution starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// A script's top level.
    Script,
    Main,
    Controller,
    Test,
}

/// What a path starts at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Root {
    Entry(Entry),
    /// A function no code calls that the server knows of: a framework, a container or an
    /// event calls it, or nothing does.
    Uncalled,
    /// Its callers were not asked for: the path is longer than the limits.
    Cut,
}

impl Root {
    /// Paths that start where execution starts come first, tests and cut paths last.
    fn rank(self) -> u8 {
        match self {
            Root::Entry(Entry::Script | Entry::Main | Entry::Controller) => 0,
            Root::Uncalled => 1,
            Root::Entry(Entry::Test) => 2,
            Root::Cut => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallPath {
    /// From the root to the function the target line is in.
    pub steps: Vec<Step>,
    pub root: Root,
    /// In the order execution passes them.
    pub stops: Vec<Stop>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// The function the target line is in.
    pub target: Step,
    pub paths: Vec<CallPath>,
    /// What the limits left out, and what the server could not answer.
    pub notes: Vec<String>,
    pub requests: usize,
}

type Key = (PathBuf, Pos);

#[derive(Debug, Clone)]
struct Function {
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

impl Function {
    fn key(&self) -> Key {
        (self.path.clone(), self.at)
    }
}

struct Node {
    function: Function,
    callers: Vec<Edge>,
    explored: bool,
}

struct Edge {
    from: usize,
    /// Lines (0-based) of the caller that make the call.
    sites: Vec<u32>,
    /// Only a run's stack showed it.
    observed: bool,
}

/// What is asked: the paths to `line` (1-based) of `file`, in the project in `root`, with the
/// calls earlier runs showed.
pub(crate) struct Question<'a> {
    pub root: &'a Path,
    pub file: &'a Path,
    pub line: u32,
    pub observed: &'a [Observed],
    pub limits: Limits,
}

/// Finds the paths, telling `progress` what it asks about as it goes.
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
        facts: HashMap::new(),
        requests: 0,
        notes: Vec::new(),
        progress,
    };
    let line = line.saturating_sub(1);
    let target = finder.function_at(file, line).await?;
    let target = finder.add(target);
    finder.explore(target).await;
    let mut chains = finder.chains(target);
    chains.sort_by_key(|(chain, root)| (root.rank(), chain.len()));
    if chains.len() > limits.paths {
        let more = chains.len() - limits.paths;
        finder.notes.push(format!("{more} more path{} not shown", plural(more)));
        chains.truncate(limits.paths);
    }
    let mut paths = Vec::new();
    for (chain, root) in chains {
        paths.push(finder.path(&chain, root, line).await);
    }
    let target = finder.step(target, false);
    Ok(Found { target, paths, notes: finder.notes, requests: finder.requests })
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
    facts: HashMap<usize, Facts>,
    requests: usize,
    notes: Vec<String>,
    progress: P,
}

impl<A: Ask, P: FnMut(String) + Send> Finder<'_, A, P> {
    async fn request(&mut self, method: &'static str, params: Value) -> Result<Value> {
        self.requests += 1;
        self.ask.ask(method, params).await
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
    async fn function_at(&mut self, path: &Path, line: u32) -> Result<Function> {
        let symbols = self.symbols(path).await?;
        match self.function(path, &symbols, |s| s.range.has_line(line)) {
            Some(function) => Ok(function),
            None => bail!("line {} of {} is in no function the server knows of", line + 1, file_name(path)),
        }
    }

    /// The function `pos` of `path` is in, or the file's top level.
    async fn enclosing(&mut self, path: &Path, pos: Pos) -> Result<Function> {
        let symbols = self.symbols(path).await?;
        match self.function(path, &symbols, |s| s.range.contains(pos)) {
            Some(function) => Ok(function),
            None => self.script(path),
        }
    }

    /// The top level of `path`, as a function that calls what is called outside functions.
    fn script(&mut self, path: &Path) -> Result<Function> {
        let last = self.text(path)?.last_line();
        let name = file_name(path);
        Ok(Function {
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
    fn function(&self, path: &Path, symbols: &[Symbol], inside: impl Fn(&Symbol) -> bool) -> Option<Function> {
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
        Some(Function {
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

    fn add(&mut self, function: Function) -> usize {
        if let Some(&n) = self.index.get(&function.key()) {
            if self.nodes[n].function.item.is_none() {
                self.nodes[n].function.item = function.item;
            }
            return n;
        }
        self.index.insert(function.key(), self.nodes.len());
        self.nodes.push(Node { function, callers: Vec::new(), explored: false });
        self.nodes.len() - 1
    }

    /// Asks for the callers of the target, then theirs, nearest first, up to an entry point,
    /// a function nothing calls or the limits.
    async fn explore(&mut self, target: usize) {
        let mut queue = VecDeque::from([(target, 0)]);
        let (mut cut, mut crowded) = (HashSet::new(), Vec::new());
        while let Some((n, depth)) = queue.pop_front() {
            if self.nodes[n].explored || self.entry(n).is_some() {
                continue;
            }
            if depth >= self.limits.depth {
                cut.insert(n);
                continue;
            }
            if self.requests >= self.limits.requests {
                self.notes.push(format!("stopped after {} requests: the paths found so far", self.requests));
                break;
            }
            (self.progress)(format!("callers of {}", self.nodes[n].function.name));
            let callers = match self.callers(n).await {
                Ok(callers) => callers,
                Err(err) => {
                    self.notes.push(format!("callers of {}: {err:#}", self.nodes[n].function.name));
                    Vec::new()
                }
            };
            self.nodes[n].explored = true;
            if callers.len() > self.limits.callers {
                crowded.push(format!("{} ({})", self.nodes[n].function.name, callers.len()));
            }
            for (function, sites, observed) in callers.into_iter().take(self.limits.callers) {
                let from = self.add(function);
                self.nodes[n].callers.push(Edge { from, sites, observed });
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
    async fn callers(&mut self, n: usize) -> Result<Vec<(Function, Vec<u32>, bool)>> {
        let function = self.nodes[n].function.clone();
        let mut found: Vec<(Function, Vec<u32>, bool)> = Vec::new();
        let called =
            if self.ask.call_hierarchy() { self.incoming(n).await? } else { self.referrers(&function).await? };
        for (caller, site) in called {
            match found.iter_mut().find(|(known, ..)| known.key() == caller.key()) {
                Some((_, sites, _)) => sites.push(site),
                None => found.push((caller, vec![site], false)),
            }
        }
        for (caller, site) in self.seen(&function).await? {
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
    async fn incoming(&mut self, n: usize) -> Result<Vec<(Function, u32)>> {
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
        let function = &self.nodes[n].function;
        if let Some(item) = &function.item {
            return Ok(Some(item.clone()));
        }
        if function.script {
            return Ok(None);
        }
        let (path, at) = (function.path.clone(), function.at);
        let params = json!({ "textDocument": lsp::document(&path), "position": at.json() });
        let answer = self.request("textDocument/prepareCallHierarchy", params).await?;
        let items = answer.as_array().cloned().unwrap_or_default();
        let starts_at = |item: &&Value| item.get("selectionRange").and_then(Span::read).is_some_and(|s| s.start == at);
        let item = items.iter().find(starts_at).or(items.first()).cloned();
        self.nodes[n].function.item = item.clone();
        Ok(item)
    }

    /// Callers from `textDocument/references`: those that call it, each in the function it
    /// is in.
    async fn referrers(&mut self, function: &Function) -> Result<Vec<(Function, u32)>> {
        let params = json!({
            "textDocument": lsp::document(&function.path),
            "position": function.at.json(),
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

    /// Callers the stacks of earlier runs showed calling `function`, from the project's code:
    /// a framework's own functions would lead up through its internals.
    async fn seen(&mut self, function: &Function) -> Result<Vec<(Function, u32)>> {
        let calls: Vec<(PathBuf, u32)> = self
            .observed
            .iter()
            .filter(|call| call.callee_path == function.path && short_name(&call.callee) == function.short)
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
        let function = &self.nodes[n].function;
        let relative = function.path.strip_prefix(&self.root).unwrap_or(&function.path);
        let file = file_name(&function.path);
        let container = function.container.as_deref().unwrap_or_default();
        let in_dir =
            |names: &[&str]| relative.parent().is_some_and(|dir| dir.iter().any(|d| names.iter().any(|n| d == *n)));
        if function.script {
            Some(Entry::Script)
        } else if function.short == "main" {
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
                node.callers.iter().map(|edge| edge.from).filter(|from| !chain.contains(from)).collect();
            let root = match self.entry(top) {
                Some(entry) => Some(Root::Entry(entry)),
                None if !node.explored => Some(Root::Cut),
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

    /// A path from a chain (target first), with its lines.
    async fn path(&mut self, chain: &[usize], root: Root, target: u32) -> CallPath {
        let order: Vec<usize> = chain.iter().rev().copied().collect();
        let mut steps = Vec::new();
        let mut stops = Vec::new();
        for (k, &n) in order.iter().enumerate() {
            let into = k.checked_sub(1).and_then(|before| self.edge(order[before], n));
            steps.push(self.step(n, into.is_some_and(|edge| edge.observed)));
            let next = order.get(k + 1).copied();
            let calls: Vec<(u32, String)> = match next.and_then(|next| self.edge(n, next).map(|edge| (next, edge))) {
                Some((next, edge)) => {
                    let name = self.nodes[next].function.short.clone();
                    edge.sites.iter().map(|line| (*line, name.clone())).collect()
                }
                None => Vec::new(),
            };
            let facts = self.facts(n).await;
            let target = next.is_none().then_some(target);
            let path = &self.nodes[n].function.path;
            stops.extend(
                select::choose(&facts, &calls, target, self.limits.state).into_iter().map(|(line, why)| Stop {
                    path: path.clone(),
                    line: line + 1,
                    why,
                    step: k,
                }),
            );
        }
        CallPath { steps, root, stops }
    }

    /// The call from node `from` into node `to`.
    fn edge(&self, from: usize, to: usize) -> Option<&Edge> {
        self.nodes[to].callers.iter().find(|edge| edge.from == from)
    }

    fn step(&self, n: usize, observed: bool) -> Step {
        let function = &self.nodes[n].function;
        let line = if function.script { 1 } else { function.at.line + 1 };
        Step { name: function.name.clone(), path: function.path.clone(), line, observed }
    }

    /// What node `n`'s lines do, read once.
    async fn facts(&mut self, n: usize) -> Facts {
        if let Some(facts) = self.facts.get(&n) {
            return facts.clone();
        }
        let function = self.nodes[n].function.clone();
        (self.progress)(format!("lines of {}", function.name));
        let facts = match self.read_facts(n, &function).await {
            Ok(facts) => facts,
            Err(err) => {
                self.notes.push(format!("lines of {}: {err:#}", function.name));
                Facts::default()
            }
        };
        self.facts.insert(n, facts.clone());
        facts
    }

    async fn read_facts(&mut self, n: usize, function: &Function) -> Result<Facts> {
        let text = self.text(&function.path)?;
        let entry = (!function.script).then(|| match self.language {
            // Delve takes only a line with a statement, and the first line of a body can be
            // part of one that goes on over lines; on the declaration it stops as the
            // function starts.
            Language::Go => function.at.line,
            _ => {
                let at = (function.at.line, lsp::byte(text.line(function.at.line), function.at.character));
                text::entry(&text, self.language, at, function.range.end.line)
            }
        });
        // A script's own lines are those outside its functions.
        let symbols = self.symbols(&function.path).await?;
        let mut functions = Vec::new();
        if function.script {
            collect_functions(&symbols, &mut functions);
        }
        let own = |line: u32| function.range.has_line(line) && !functions.iter().any(|span: &Span| span.has_line(line));
        let writes = self.writes(function, &text, &own).await;
        let uses = if function.script { Vec::new() } else { self.uses(n, function, &text, &own).await };
        Ok(Facts { entry, writes, uses })
    }

    /// The lines that write a variable or a field, with its name. The text says which names a
    /// line assigns to; the server, asked for the highlights of each, which occurrences are
    /// the same variable and, if it tells writes from reads, which of them write.
    async fn writes(&mut self, function: &Function, text: &Text, own: &impl Fn(u32) -> bool) -> Vec<(u32, String)> {
        let mut lexer = Lexer::new(self.language);
        let mut candidates: Vec<(u32, usize, String)> = Vec::new();
        for line in function.range.start.line..=function.range.end.line {
            let toks = lexer.line(text.line(line));
            // Delve takes no line in the middle of a statement, nor the first of one that
            // goes on over lines.
            if own(line) && !(self.language == Language::Go && text::continues(&toks)) {
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
            if asked >= LINE_REQUESTS {
                found.push((*line, name.clone()));
                continue;
            }
            asked += 1;
            let position = Pos::new(*line, lsp::utf16(text.line(*line), *at));
            let params = json!({ "textDocument": lsp::document(&function.path), "position": position.json() });
            let highlights = match self.request("textDocument/documentHighlight", params).await {
                Ok(answer) => lsp::highlights(&answer),
                // Without an answer, the text is all there is.
                Err(_) => {
                    found.push((*line, name.clone()));
                    continue;
                }
            };
            let by_kind = highlights.iter().any(|h| h.kind == WRITE);
            for highlight in highlights.iter().filter(|h| own(h.span.start.line)) {
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

    /// The lines that call another function of the project, with its name: from
    /// `callHierarchy/outgoingCalls`, or the names the text calls whose definition the
    /// server places in the project.
    async fn uses(
        &mut self,
        n: usize,
        function: &Function,
        text: &Text,
        own: &impl Fn(u32) -> bool,
    ) -> Vec<(u32, String)> {
        let mut found = Vec::new();
        if self.ask.call_hierarchy() {
            let Ok(Some(item)) = self.item(n).await else { return found };
            let Ok(answer) = self.request("callHierarchy/outgoingCalls", json!({ "item": item })).await else {
                return found;
            };
            for call in answer.as_array().into_iter().flatten() {
                let to = &call["to"];
                let (Some(path), Some(name)) = (to["uri"].as_str().and_then(lsp::path), to["name"].as_str()) else {
                    continue;
                };
                if !self.in_project(&path) {
                    continue;
                }
                let sites = call["fromRanges"].as_array().into_iter().flatten().filter_map(Span::read);
                found.extend(sites.filter(|s| own(s.start.line)).map(|s| (s.start.line, name.to_owned())));
            }
        } else {
            let mut lexer = Lexer::new(self.language);
            let mut calls: Vec<(u32, usize, String)> = Vec::new();
            for line in function.range.start.line..=function.range.end.line {
                let toks = lexer.line(text.line(line));
                if own(line) && line != function.at.line {
                    calls.extend(text::calls(&toks).into_iter().map(|(name, at)| (line, at, name)));
                }
            }
            let mut decided: HashMap<String, bool> = HashMap::new();
            for (line, at, name) in calls {
                if !decided.contains_key(&name) && decided.len() < LINE_REQUESTS {
                    let position = Pos::new(line, lsp::utf16(text.line(line), at));
                    let params = json!({ "textDocument": lsp::document(&function.path), "position": position.json() });
                    let answer = self.request("textDocument/definition", params).await.unwrap_or_default();
                    let ours = lsp::locations(&answer).iter().any(|l| self.in_project(&l.path));
                    decided.insert(name.clone(), ours);
                }
                if decided.get(&name) == Some(&true) {
                    found.push((line, name));
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
    use super::*;

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

    /// Answers as a server would, by method and the document and line asked about; asked
    /// anything else, it knows nothing.
    struct Fake {
        call_hierarchy: bool,
        answers: Vec<(&'static str, String, u32, Value)>,
    }

    impl Fake {
        fn answer(&mut self, method: &'static str, uri: String, line: u32, answer: Value) {
            self.answers.push((method, uri, line, answer));
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
            let (document, at) = match params.get("item") {
                Some(item) => (&item["uri"], &item["selectionRange"]["start"]),
                None => (&params["textDocument"]["uri"], &params["position"]),
            };
            let line = at["line"].as_u64().and_then(|l| u32::try_from(l).ok());
            let answer = self.answers.iter().find(|(m, uri, l, _)| {
                *m == method && Some(uri.as_str()) == document.as_str() && (line.is_none() || line == Some(*l))
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
        let text = std::fs::read_to_string(project.path(file)).expect("file");
        let column = text.lines().nth(line as usize).and_then(|l| l.find(name)).expect("on the line");
        let (column, size) = (u32::try_from(column).expect("short"), u32::try_from(name.len()).expect("short"));
        json!({ "uri": project.uri(file), "range": span(line, column, line, column + size) })
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

    fn names(path: &CallPath) -> Vec<&str> {
        path.steps.iter().map(|step| step.name.as_str()).collect()
    }

    fn stops(path: &CallPath) -> Vec<(String, u32, Why)> {
        let file = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        path.stops.iter().map(|stop| (file(&stop.path), stop.line, stop.why.clone())).collect()
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
        return $total;
    }
}
";
    const CONTROLLER: &str = "<?php
final class OrderController
{
    public function show(int $id): string
    {
        return (string) $this->orders->totalFor($id);
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

    /// A server without call hierarchy, as intelephense: references, symbols, highlights and
    /// definitions.
    fn php_server(project: &Project) -> Fake {
        let mut fake = Fake { call_hierarchy: false, answers: Vec::new() };
        let class = |name: &str, end: u32, method: Value| symbol(name, 5, (1, end), (1, 12), json!([method]));
        let files = [
            ("src/Money.php", class("Money", 11, symbol("lineTotal", 6, (3, 10), (3, 27), json!([])))),
            ("src/OrderService.php", class("OrderService", 12, symbol("totalFor", 6, (3, 11), (3, 20), json!([])))),
            (
                "src/Controller/OrderController.php",
                class("OrderController", 7, symbol("show", 6, (3, 6), (3, 20), json!([]))),
            ),
            ("tests/MoneyTest.php", class("MoneyTest", 7, symbol("testBulk", 6, (3, 6), (3, 20), json!([])))),
            ("src/Listener.php", class("Listener", 7, symbol("onEvent", 6, (3, 6), (3, 20), json!([])))),
            ("src/Dispatcher.php", class("Dispatcher", 9, symbol("dispatch", 6, (3, 8), (3, 20), json!([])))),
            ("src/Kernel.php", json!(null)),
        ];
        for (file, symbols) in files {
            let symbols = if symbols.is_null() { json!([]) } else { json!([symbols]) };
            fake.answer("textDocument/documentSymbol", project.uri(file), 0, symbols);
        }
        fake.answer(
            "textDocument/references",
            project.uri("src/Money.php"),
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
            project.uri("src/OrderService.php"),
            3,
            json!([refer(project, "src/Controller/OrderController.php", 5, "totalFor")]),
        );
        fake.answer(
            "textDocument/documentHighlight",
            project.uri("src/Money.php"),
            5,
            highlights(&[(5, 8, 3), (7, 12, 3), (7, 29, 2), (9, 15, 2)]),
        );
        fake.answer(
            "textDocument/documentHighlight",
            project.uri("src/OrderService.php"),
            5,
            highlights(&[(5, 8, 3), (7, 17, 2)]),
        );
        fake.answer(
            "textDocument/documentHighlight",
            project.uri("src/OrderService.php"),
            6,
            highlights(&[(6, 8, 3), (8, 12, 3), (10, 15, 2)]),
        );
        fake.answer(
            "textDocument/definition",
            project.uri("src/OrderService.php"),
            5,
            json!([{ "uri": project.uri("src/OrderRepository.php"), "range": span(3, 20, 3, 24) }]),
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
                ("src/Kernel.php", KERNEL),
                ("tests/MoneyTest.php", TEST),
                ("src/Listener.php", LISTENER),
                ("src/Dispatcher.php", DISPATCHER),
            ],
        )
    }

    #[tokio::test]
    async fn references_that_call_lead_up_to_entry_points() {
        let project = php_project();
        let mut fake = php_server(&project);
        let found = ask(&mut fake, &project, "src/Money.php", 8, &[], Limits::default()).await;
        assert_eq!(found.target.name, "Money::lineTotal");
        let paths: Vec<(Vec<&str>, Root)> = found.paths.iter().map(|path| (names(path), path.root)).collect();
        // The callback registered by name calls nothing; the controller comes first, the test last.
        assert_eq!(
            paths,
            [
                (
                    vec!["OrderController::show", "OrderService::totalFor", "Money::lineTotal"],
                    Root::Entry(Entry::Controller)
                ),
                (vec!["Listener::onEvent", "Money::lineTotal"], Root::Uncalled),
                (vec!["MoneyTest::testBulk", "Money::lineTotal"], Root::Entry(Entry::Test)),
            ]
        );
        let stop = |file: &str, line: u32, why: Why| (file.to_owned(), line, why);
        assert_eq!(
            stops(&found.paths[0]),
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
    }

    #[tokio::test]
    async fn a_call_a_run_showed_joins_what_the_server_knows() {
        let project = php_project();
        let mut fake = php_server(&project);
        let observed = [Observed {
            path: project.path("src/Dispatcher.php"),
            line: 7,
            callee: "App\\Listener->onEvent".into(),
            callee_path: project.path("src/Listener.php"),
        }];
        let found = ask(&mut fake, &project, "src/Money.php", 8, &observed, Limits::default()).await;
        let listener = found.paths.iter().find(|path| path.steps.len() == 3 && path.root == Root::Uncalled);
        let listener = listener.expect("a path through the dispatcher");
        assert_eq!(names(listener), ["Dispatcher::dispatch", "Listener::onEvent", "Money::lineTotal"]);
        let seen: Vec<bool> = listener.steps.iter().map(|step| step.observed).collect();
        assert_eq!(seen, [false, true, false]);
        assert!(stops(listener).contains(&("Dispatcher.php".into(), 7, Why::Calls("onEvent".into()))));
    }

    #[tokio::test]
    async fn the_limits_cut_paths_short_and_say_so() {
        let project = php_project();
        let mut fake = php_server(&project);
        let limits = Limits { depth: 1, ..Limits::default() };
        let found = ask(&mut fake, &project, "src/Money.php", 8, &[], limits).await;
        let cut: Vec<Vec<&str>> = found.paths.iter().filter(|p| p.root == Root::Cut).map(names).collect();
        assert_eq!(
            cut,
            [vec!["OrderService::totalFor", "Money::lineTotal"], vec!["Listener::onEvent", "Money::lineTotal"]]
        );
        let note = "2 functions at the limit of 1 call up: their callers were not asked for";
        assert!(found.notes.contains(&note.to_owned()), "{:?}", found.notes);
        let limits = Limits { paths: 1, ..Limits::default() };
        let found = ask(&mut fake, &project, "src/Money.php", 8, &[], limits).await;
        assert_eq!(found.paths.len(), 1);
        assert!(found.notes.contains(&"2 more paths not shown".to_owned()), "{:?}", found.notes);
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
    async fn call_hierarchy_leads_up_through_recursion_to_the_script() {
        let project = Project::new("ts", &[("a.ts", SCRIPT)]);
        let uri = project.uri("a.ts");
        let mut fake = Fake { call_hierarchy: true, answers: Vec::new() };
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
        let calls = |from: Value, line: u32| json!({ "from": from, "fromRanges": [span(line, 9, line, 12)] });
        fake.answer("textDocument/prepareCallHierarchy", uri.clone(), 0, json!([item("leaf", 0)]));
        fake.answer("callHierarchy/incomingCalls", uri.clone(), 0, json!([calls(item("mid", 5), 6)]));
        fake.answer(
            "callHierarchy/incomingCalls",
            uri.clone(),
            5,
            json!([calls(item("top", 8), 9), calls(item("rec", 11), 12)]),
        );
        fake.answer("callHierarchy/incomingCalls", uri.clone(), 8, json!([calls(script, 14)]));
        fake.answer("callHierarchy/incomingCalls", uri.clone(), 11, json!([calls(item("rec", 11), 12)]));
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
        let paths: Vec<(Vec<&str>, Root)> = found.paths.iter().map(|path| (names(path), path.root)).collect();
        assert_eq!(
            paths,
            [
                (vec!["a.ts", "top", "mid", "leaf"], Root::Entry(Entry::Script)),
                (vec!["rec", "mid", "leaf"], Root::Uncalled)
            ]
        );
        let leaf: Vec<(u32, Why)> =
            found.paths[0].stops.iter().filter(|s| s.step == 3).map(|s| (s.line, s.why.clone())).collect();
        assert_eq!(leaf, [(2, Why::Entry), (3, Why::Target)]);
        assert_eq!(
            found.paths[0].stops[0],
            Stop { path: project.path("a.ts"), line: 15, why: Why::Calls("top".into()), step: 0 }
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
