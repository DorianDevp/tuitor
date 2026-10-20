//! The call trees PATHS shows: from each place execution starts, what it calls, down through
//! the function the line asked about is in and on into what each of those calls; and the
//! lines worth a breakpoint in each stage of a tree, given which stages below it are chosen
//! with it.
//!
//! A stage is one call: a function, called from its parent stage. A chain from a root down to
//! a stage with nothing below it that passes the function asked about, the anchor, is an
//! end-to-end path. Stages are kept depth first, so a stage's branch, the stage and every
//! stage below it, is the run `stage..end` of them.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::select::{self, Facts, Why};

/// A function in the trees.
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    /// With its class or type: `OrderService::total`.
    pub name: String,
    /// As declared: `total`.
    pub short: String,
    pub path: PathBuf,
    /// The line of its name, 1-based; 1 for a script's top level.
    pub line: u32,
    /// What its lines do, as far as they were read.
    pub facts: Facts,
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

/// What a tree starts at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Root {
    Entry(Entry),
    /// A function no code calls that the server knows of: a framework, a container or an
    /// event calls it, or nothing does.
    Uncalled,
    /// Its callers were not asked for: the way up is longer than the limits.
    Cut,
}

impl Root {
    /// Trees that start where execution starts come first, tests and cut ones last.
    pub(crate) fn rank(self) -> u8 {
        match self {
            Root::Entry(Entry::Script | Entry::Main | Entry::Controller) => 0,
            Root::Uncalled => 1,
            Root::Entry(Entry::Test) => 2,
            Root::Cut => 3,
        }
    }
}

/// Why a stage has nothing below it although its function may call more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cut {
    /// It calls a function above it in its chain: recursion, followed once.
    Recursion,
    /// Its calls were not asked for: it is as many calls below the paths as the limit goes.
    Deep,
    /// Its calls were not asked for, or not followed: the requests or the stages ran out.
    Spent,
}

/// A call in a tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Stage {
    /// Its function, in [`Found::functions`].
    pub function: usize,
    pub parent: Option<usize>,
    /// The calls it makes, in the order of their lines.
    pub children: Vec<usize>,
    /// One past the last stage below it.
    pub end: usize,
    /// Calls from its root.
    pub depth: usize,
    /// The lines of the parent, 1-based, that make the call; none for a root.
    pub sites: Vec<u32>,
    /// Only a run's stack showed the call from the parent.
    pub observed: bool,
    /// Its function is the one the line asked about is in.
    pub anchor: bool,
    /// An anchor is at it or below it.
    pub leads: bool,
    pub cut: Option<Cut>,
    /// For a root: where execution starts.
    pub root: Option<Root>,
}

/// A line worth a breakpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Stop {
    pub path: PathBuf,
    /// 1-based.
    pub line: u32,
    pub why: Why,
    /// The stage it is in.
    pub stage: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub functions: Vec<Function>,
    /// The function the line asked about is in.
    pub target: usize,
    /// The line asked about, 1-based.
    pub line: u32,
    /// The trees, depth first, one after the other.
    pub stages: Vec<Stage>,
    /// What the limits left out, and what the server could not answer.
    pub notes: Vec<String>,
    /// The calls no server sees in this language.
    pub unseen: String,
    pub requests: usize,
    /// Lines of state kept per stage, besides its entry and calls.
    pub state: usize,
}

impl Found {
    pub fn target(&self) -> &Function {
        &self.functions[self.target]
    }

    pub fn function(&self, stage: usize) -> &Function {
        &self.functions[self.stages[stage].function]
    }

    pub fn roots(&self) -> impl Iterator<Item = usize> + '_ {
        self.stages.iter().enumerate().filter(|(_, stage)| stage.parent.is_none()).map(|(i, _)| i)
    }

    /// The stages from its root down to `stage`.
    pub fn chain(&self, stage: usize) -> Vec<usize> {
        let mut chain = vec![stage];
        while let Some(parent) = self.stages[chain[chain.len() - 1]].parent {
            chain.push(parent);
        }
        chain.reverse();
        chain
    }

    /// Whether `stage` is on an end-to-end path: an anchor is at it, below it or above it.
    pub fn on_path(&self, stage: usize) -> bool {
        self.stages[stage].leads || self.chain(stage).iter().any(|s| self.stages[*s].anchor)
    }

    /// The end-to-end paths, each as the stage it ends at: a stage with nothing below it
    /// whose chain passes an anchor.
    pub fn paths(&self) -> Vec<usize> {
        (0..self.stages.len()).filter(|s| self.stages[*s].children.is_empty() && self.on_path(*s)).collect()
    }

    /// The end-to-end path through `stage`: its chain, and below it the first call on an
    /// end-to-end path, else the first call, down to a stage with nothing below it.
    pub fn path_through(&self, stage: usize) -> Vec<usize> {
        let mut path = self.chain(stage);
        let mut at = stage;
        while let Some(&first) = self.stages[at].children.first() {
            at = self.stages[at].children.iter().copied().find(|c| self.on_path(*c)).unwrap_or(first);
            path.push(at);
        }
        path
    }

    /// How many of the calls of `stage` are on end-to-end paths: where two or more are, the
    /// paths part there.
    pub fn forks(&self, stage: usize) -> usize {
        self.stages[stage].children.iter().filter(|c| self.on_path(**c)).count()
    }

    /// `stage` and every stage below it.
    pub fn branch(&self, stage: usize) -> std::ops::Range<usize> {
        stage..self.stages[stage].end
    }

    /// The lines worth a breakpoint in `stage` when the stages `chosen` says are chosen with
    /// it: its entry, its calls into those of its calls chosen, in an anchor the line asked
    /// about, and before the last of those the last writes and calls into the project that
    /// make the state they get.
    pub fn lines(&self, stage: usize, chosen: impl Fn(usize) -> bool) -> Vec<Stop> {
        let at = &self.stages[stage];
        let function = &self.functions[at.function];
        let calls: Vec<(u32, String)> = at
            .children
            .iter()
            .filter(|child| chosen(**child))
            .flat_map(|child| {
                let callee = &self.functions[self.stages[*child].function].short;
                self.stages[*child].sites.iter().map(|line| (line.saturating_sub(1), callee.clone()))
            })
            .collect();
        let target = at.anchor.then(|| self.line.saturating_sub(1));
        let lines = select::choose(&function.facts, &calls, target, self.state);
        lines.into_iter().map(|(line, why)| Stop { path: function.path.clone(), line: line + 1, why, stage }).collect()
    }

    /// The lines of the stages chosen, each line once, stage by stage depth first.
    pub fn selection(&self, chosen: &BTreeSet<usize>) -> Vec<Stop> {
        let mut stops: Vec<Stop> = Vec::new();
        for &stage in chosen.iter().filter(|s| **s < self.stages.len()) {
            for stop in self.lines(stage, |child| chosen.contains(&child)) {
                if !stops.iter().any(|known| known.path == stop.path && known.line == stop.line) {
                    stops.push(stop);
                }
            }
        }
        stops
    }
}
