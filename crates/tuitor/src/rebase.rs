//! REBASE for a tab: the rebases git's reflog holds for the project's repository, newest
//! first, and for one opened, what was decided by hand in each pick that stopped and what
//! the branch lost on the way. Reading a rebase asks git a few times for every file of every
//! pick that stopped, so it is read on a thread of its own, once.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use tuitor_core::follow;
use tuitor_core::rebase::{self, Decision, Lost, Pick, Rebase};

use crate::app::{App, Outcome, Pane};
use crate::command::View;
use crate::sets::name;
use crate::tab::Cursor;

/// A rebase as read: each pick that stopped with what was decided in it, and what was lost.
pub struct Analysis {
    pub stops: Vec<Stop>,
    pub lost: Vec<Lost>,
    /// Picks the target already had, rewritten, each with the target's commit.
    pub upstream: HashMap<String, String>,
    /// The last of those as the branch had it before the rebase: `git rebase --onto <target>
    /// <this>` would have picked only the commits after it.
    pub replayed_from: Option<String>,
}

pub struct Stop {
    pub pick: Pick,
    /// The commit it picked, when the branch before the rebase still had one like it.
    pub original: Option<String>,
    pub decisions: Vec<Decision>,
}

impl Analysis {
    pub fn decisions(&self) -> usize {
        self.stops.iter().map(|s| s.decisions.len()).sum()
    }

    /// The decisions made in picks the target had already.
    pub fn decisions_again(&self) -> usize {
        self.stops.iter().filter(|s| self.upstream.contains_key(&s.pick.commit)).map(|s| s.decisions.len()).sum()
    }
}

struct Loading {
    index: usize,
    done: Arc<AtomicUsize>,
    total: usize,
    rx: Receiver<Result<Analysis, String>>,
}

/// What REBASE shows for a tab.
pub struct RebaseView {
    /// The repository's top directory.
    pub repo: PathBuf,
    pub rebases: Vec<Rebase>,
    pub open: HashSet<usize>,
    pub analyses: HashMap<usize, Analysis>,
    loading: Option<Loading>,
    pub cursor: Cursor,
}

/// A row of REBASE's tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Rebase(usize),
    /// A rebase being read: how far.
    Reading(usize),
    Stop(usize, usize),
    Decision(usize, usize, usize),
    LostHead(usize),
    Lost(usize, usize),
}

impl RebaseView {
    /// The rebases of the repository `dir` is in.
    pub fn read(dir: &Path) -> Result<RebaseView, String> {
        let repo = rebase::top(dir).map_err(|_| format!("{} is in no git repository", dir.display()))?;
        let rebases = rebase::rebases(&repo).map_err(|e| format!("reading the reflog: {e:#}"))?;
        Ok(RebaseView {
            repo,
            rebases,
            open: HashSet::new(),
            analyses: HashMap::new(),
            loading: None,
            cursor: Cursor::default(),
        })
    }

    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (i, _) in self.rebases.iter().enumerate() {
            rows.push(Row::Rebase(i));
            if !self.open.contains(&i) {
                continue;
            }
            let Some(analysis) = self.analyses.get(&i) else {
                rows.push(Row::Reading(i));
                continue;
            };
            for (s, stop) in analysis.stops.iter().enumerate() {
                rows.push(Row::Stop(i, s));
                rows.extend((0..stop.decisions.len()).map(|d| Row::Decision(i, s, d)));
            }
            if !analysis.lost.is_empty() {
                rows.push(Row::LostHead(i));
                rows.extend((0..analysis.lost.len()).map(|l| Row::Lost(i, l)));
            }
        }
        rows
    }

    pub fn selected(&self) -> Option<Row> {
        self.rows().get(self.cursor.pos).copied()
    }

    /// The rebase being read, and how many of its stops are done of how many.
    pub fn progress(&self) -> Option<(usize, usize, usize)> {
        self.loading.as_ref().map(|l| (l.index, l.done.load(Ordering::Relaxed), l.total))
    }

    /// Opens rebase `index`, reading it the first time; closes it when open.
    pub fn toggle(&mut self, index: usize) {
        if !self.open.insert(index) {
            self.open.remove(&index);
            return;
        }
        if self.analyses.contains_key(&index) || self.loading.as_ref().is_some_and(|l| l.index == index) {
            return;
        }
        let rebase = self.rebases[index].clone();
        let repo = self.repo.clone();
        let done = Arc::new(AtomicUsize::new(0));
        let total = rebase.stops().count();
        let (tx, rx) = channel();
        let counter = done.clone();
        std::thread::spawn(move || {
            let _ = tx.send(analyse(&repo, &rebase, &counter).map_err(|e| format!("{e:#}")));
        });
        self.loading = Some(Loading { index, done, total, rx });
    }

    /// Takes in a reading that finished; says how it went.
    pub fn poll(&mut self) -> Option<Result<String, String>> {
        let loading = self.loading.as_ref()?;
        let result = match loading.rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("reading the rebase stopped".into()),
        };
        let index = loading.index;
        self.loading = None;
        Some(result.map(|analysis| {
            let said = summary(&self.rebases[index], &analysis);
            self.analyses.insert(index, analysis);
            said
        }))
    }
}

/// `feature onto main: 3 picks stopped, 12 decisions by hand, 2 lines of code lost`
fn summary(rebase: &Rebase, analysis: &Analysis) -> String {
    let lost = analysis.lost.iter().flat_map(|l| &l.lines).filter(|l| l.now.is_none() && !l.comment).count();
    let again = match (analysis.upstream.len(), &analysis.replayed_from) {
        (0, _) | (_, None) => String::new(),
        (n, Some(from)) => format!(
            "; {n} picks, with {} of the decisions, were in {} already: --onto {} {} would have picked the rest",
            analysis.decisions_again(),
            rebase.onto_name,
            rebase.onto_name,
            &from[..from.len().min(9)]
        ),
    };
    format!(
        "{}: {} picks stopped, {} decisions by hand, {lost} lines of code lost{again}",
        rebase.title(),
        analysis.stops.len(),
        analysis.decisions()
    )
}

fn analyse(repo: &Path, rebase: &Rebase, done: &AtomicUsize) -> anyhow::Result<Analysis> {
    let originals = rebase::originals(repo, rebase)?;
    let upstream = rebase::upstream(repo, rebase)?;
    let replayed_from = rebase
        .picks
        .iter()
        .rev()
        .find(|p| upstream.contains_key(&p.commit))
        .and_then(|p| originals.get(&p.commit))
        .cloned();
    let mut stops = Vec::new();
    for pick in rebase.stops() {
        let original = originals.get(&pick.commit).cloned();
        let decisions = match &original {
            Some(original) => rebase::decisions(repo, original, &pick.commit)?,
            None => Vec::new(),
        };
        stops.push(Stop { pick: pick.clone(), original, decisions });
        done.fetch_add(1, Ordering::Relaxed);
    }
    Ok(Analysis { stops, lost: rebase::lost(repo, rebase)?, upstream, replayed_from })
}

/// Where a decision is in the file as it is now: found by its anchor, else at its line.
pub fn place(repo: &Path, decision: &Decision) -> Option<(PathBuf, u32)> {
    let path = repo.join(&decision.file);
    let text = std::fs::read_to_string(&path).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    let line = match &decision.anchor {
        Some(anchor) => follow::find(&lines, anchor, decision.line())?,
        None => decision.line().min(u32::try_from(lines.len()).unwrap_or(u32::MAX)),
    };
    Some((path, line.max(1)))
}

impl App {
    /// `/rebase [n]`: REBASE, read anew from the reflog; with a number, that rebase opened.
    pub(crate) fn rebase(&mut self, arg: Option<String>) -> Outcome {
        let dir = self.tab().engine.workspace().to_path_buf();
        let mut view = RebaseView::read(&dir)?;
        if view.rebases.is_empty() {
            return Err("the reflog holds no rebase".into());
        }
        if let Some(n) = arg.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
            let index: usize = n.parse().map_err(|_| format!("expected the number of a rebase: {n}"))?;
            let index = index.checked_sub(1).filter(|i| *i < view.rebases.len()).ok_or("no such rebase")?;
            view.toggle(index);
            view.cursor.pos = index;
        }
        let count = view.rebases.len();
        let tab = self.tab_mut();
        tab.rebase = Some(view);
        tab.ui.view = View::Rebase;
        tab.ui.zoom = false;
        tab.set_focus(Pane::Rebase);
        Ok(format!("{count} rebases in the reflog; ⏎ reads one"))
    }

    /// Takes in a rebase that finished reading.
    pub(crate) fn poll_rebase(&mut self) {
        let Some(view) = self.tab_mut().rebase.as_mut() else { return };
        let Some(result) = view.poll() else { return };
        match result {
            Ok(message) => self.say(&message),
            Err(message) => self.execute_error(&message),
        }
    }

    pub(crate) fn rebase_move(&mut self, delta: isize) {
        let Some(view) = self.tab_mut().rebase.as_mut() else { return };
        let len = view.rows().len();
        view.cursor.move_by(delta, len);
    }

    /// `h` and `l`: closes or opens the rebase the cursor is in.
    pub(crate) fn rebase_fold(&mut self, open: bool) {
        let Some(view) = self.tab_mut().rebase.as_mut() else { return };
        let index = match view.selected() {
            Some(Row::Rebase(i)) => i,
            Some(Row::Reading(i) | Row::Stop(i, _) | Row::Decision(i, ..) | Row::LostHead(i) | Row::Lost(i, _)) => i,
            None => return,
        };
        if view.open.contains(&index) != open {
            view.toggle(index);
        }
        if !open {
            view.cursor.pos = view.rows().iter().position(|r| *r == Row::Rebase(index)).unwrap_or(0);
        }
    }

    /// `enter` in REBASE: a rebase opens or closes; a decision opens in CODE where its line is
    /// now.
    pub(crate) fn rebase_open(&mut self) -> Outcome {
        let view = self.tab().rebase.as_ref().ok_or("no rebase read")?;
        match view.selected().ok_or("nothing selected")? {
            Row::Rebase(i) => {
                if let Some(view) = self.tab_mut().rebase.as_mut() {
                    view.toggle(i);
                }
                Ok(String::new())
            }
            Row::Decision(i, s, d) => {
                let decision = &view.analyses[&i].stops[s].decisions[d];
                let (path, line) = place(&view.repo, decision)
                    .ok_or_else(|| format!("{} no longer has that line", decision.file.display()))?;
                let tab = self.tab_mut();
                tab.ui.view = View::Code;
                tab.browse(path, line);
                Ok(String::new())
            }
            Row::Stop(..) | Row::Reading(_) | Row::LostHead(_) | Row::Lost(..) => Ok(String::new()),
        }
    }

    /// `/rebase set`: a new breakpoint set with a breakpoint at every decision of the
    /// selected rebase, pick or decision, where its line is in the files now. The set is not
    /// the active one, so its breakpoints record and the program goes on.
    pub(crate) fn rebase_set(&mut self) -> Outcome {
        let view = self.tab().rebase.as_ref().ok_or("no rebase read; /rebase reads them")?;
        let row = view.selected().ok_or("nothing selected")?;
        let decisions: Vec<&Decision> = match row {
            Row::Rebase(i) | Row::Reading(i) | Row::LostHead(i) | Row::Lost(i, _) => {
                let analysis = view.analyses.get(&i).ok_or("the rebase is not read yet")?;
                analysis.stops.iter().flat_map(|s| &s.decisions).collect()
            }
            Row::Stop(i, s) => view.analyses[&i].stops[s].decisions.iter().collect(),
            Row::Decision(i, s, d) => vec![&view.analyses[&i].stops[s].decisions[d]],
        };
        if decisions.is_empty() {
            return Err("nothing was decided by hand here".into());
        }
        let places: Vec<Option<(PathBuf, u32)>> = decisions.iter().map(|d| place(&view.repo, d)).collect();
        let missing = places.iter().filter(|p| p.is_none()).count();
        let tab = self.tab_mut();
        let set = tab.engine.new_set();
        let mut made = 0;
        for (path, line) in places.into_iter().flatten() {
            if tab.engine.put_in_set(&path, line, set).is_ok() {
                made += 1;
            }
        }
        let set_name = name(tab.engine.breakpoints(), set);
        let gone = if missing > 0 { format!("; {missing} no longer in the files") } else { String::new() };
        let breakpoints = if made == 1 { "a breakpoint" } else { "breakpoints" };
        let count = if made == 1 { String::new() } else { format!("{made} ") };
        Ok(format!("{set_name}: {count}{breakpoints} at what was decided by hand, recording{gone}"))
    }
}
