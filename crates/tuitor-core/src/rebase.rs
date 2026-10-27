//! Rebases as git's reflog of `HEAD` keeps them, and what was decided by hand in each. A
//! rebase picks a branch's commits one by one onto another; a pick that conflicts stops until
//! the conflict is resolved and the rebase continues, and the reflog says so: `rebase (start):
//! checkout development`, `rebase (pick): …`, `rebase (continue): …` for a pick that stopped,
//! `rebase (finish): returning to refs/heads/feature`, or `rebase (abort)`.
//!
//! For a pick that stopped, git's own merge of the commit onto its new parent is made again
//! (`git merge-file`, so that no worktree or newer git is needed): what the rebased commit has
//! in place of that merge's conflicts, or anywhere else it differs from it, was decided by
//! hand. Lines the branch added before the rebase and the branch no longer has after it were
//! lost on the way, in a resolution or a pick that was dropped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::follow::{self, Anchor};
use crate::tmpdir::TempDir;

/// One rebase, from its start to its finish or abort.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rebase {
    /// The branch it rewrote, which the finish names; none for an abort.
    pub branch: Option<String>,
    /// What it rebased onto, as the start names it (`development`, a commit), and its commit.
    pub onto_name: String,
    pub onto: String,
    /// `HEAD` before it started: the branch's tip as it was.
    pub before: Option<String>,
    /// `HEAD` when it finished: the branch's new tip.
    pub after: Option<String>,
    /// Seconds since the epoch.
    pub started: i64,
    pub ended: i64,
    pub aborted: bool,
    /// The commits it made, in order.
    pub picks: Vec<Pick>,
}

impl Rebase {
    /// The picks that stopped, for a conflict or an `edit`.
    pub fn stops(&self) -> impl Iterator<Item = &Pick> {
        self.picks.iter().filter(|p| p.stopped)
    }

    /// `feature onto main`
    pub fn title(&self) -> String {
        match &self.branch {
            Some(branch) => format!("{branch} onto {}", self.onto_name),
            None => format!("onto {}", self.onto_name),
        }
    }
}

/// A commit a rebase made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    pub commit: String,
    pub subject: String,
    /// It stopped before it was made: a conflict was resolved, or the commit edited.
    pub stopped: bool,
}

/// What was decided by hand in a pick that stopped, in one file: lines of the rebased commit
/// that are not what git's merge of the original commit made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// From the top of the repository.
    pub file: PathBuf,
    /// The lines in the rebased commit's file, 1-based; `first > last` when nothing was
    /// kept, which is then before line `first`.
    pub first: u32,
    pub last: u32,
    /// It resolves a conflict; otherwise it is an edit made while the rebase stopped.
    pub conflict: bool,
    /// What the conflict's sides read: the branch being rebased onto, then the commit picked.
    pub ours: Vec<String>,
    pub theirs: Vec<String>,
    /// What the rebased commit has there.
    pub resolved: Vec<String>,
    /// Its line in the rebased commit with the lines around it, to find it in the file as it
    /// is now; none past the file's end.
    pub anchor: Option<Anchor>,
}

impl Decision {
    /// The line a breakpoint for it goes on: its first, or the one after where it removed all.
    pub fn line(&self) -> u32 {
        self.first.max(1)
    }

    /// Which side of a conflict was kept, by the lines with text, whatever their indentation.
    pub fn kept(&self) -> Kept {
        let text = |lines: &[String]| -> Vec<String> {
            lines.iter().map(|l| l.trim().to_owned()).filter(|l| !l.is_empty()).collect()
        };
        let resolved = text(&self.resolved);
        if resolved.is_empty() {
            Kept::Neither
        } else if resolved == text(&self.ours) {
            Kept::Onto
        } else if resolved == text(&self.theirs) {
            Kept::Pick
        } else {
            Kept::Both
        }
    }
}

/// What a conflict's resolution kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    /// What the rebase went onto had there, the pick's side dropped.
    Onto,
    /// The pick's side, what it went onto had there dropped.
    Pick,
    /// Something of its own: both sides, or a part of each.
    Both,
    Neither,
}

/// Lines the branch had added before a rebase that it does not have after it, together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lost {
    pub file: PathBuf,
    /// Where they were in the branch before the rebase, 1-based.
    pub line: u32,
    pub lines: Vec<LostLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LostLine {
    pub text: String,
    /// The line of the file after the rebase most like it, when one shares most of its words:
    /// it was changed, as a resolution changes a line. Without one, it is gone.
    pub now: Option<String>,
    /// It is a comment, which matters less than code.
    pub comment: bool,
}

/// `branch@sha` of `HEAD` in the repository `dir` is in, or the commit alone when detached;
/// none outside a repository.
pub fn head(dir: &Path) -> Option<String> {
    let sha = git(dir, &["rev-parse", "--short", "HEAD"]).ok()?;
    let branch = git(dir, &["symbolic-ref", "--short", "-q", "HEAD"]).ok();
    Some(match branch.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        Some(branch) => format!("{branch}@{}", sha.trim()),
        None => sha.trim().to_owned(),
    })
}

/// The repository's top directory, for a directory in it.
pub fn top(dir: &Path) -> Result<PathBuf> {
    Ok(PathBuf::from(git(dir, &["rev-parse", "--show-toplevel"])?.trim()))
}

/// The rebases the reflog of `HEAD` in `repo` holds, newest first.
pub fn rebases(repo: &Path) -> Result<Vec<Rebase>> {
    let log = git(repo, &["reflog", "show", "--date=unix", "--format=%H%x00%gd%x00%gs", "HEAD"])?;
    Ok(parse_reflog(&log))
}

/// The reflog's lines, newest first, as rebases, newest first.
fn parse_reflog(log: &str) -> Vec<Rebase> {
    let entries: Vec<(&str, i64, &str)> = log
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\0');
            let sha = parts.next()?;
            let time = parts.next()?;
            let subject = parts.next()?;
            let time = time.rsplit('{').next()?.trim_end_matches('}').parse().ok()?;
            Some((sha, time, subject))
        })
        .collect();
    let mut rebases = Vec::new();
    let mut current: Option<Rebase> = None;
    // Oldest first: the entry before a start holds the branch's tip before it.
    for (i, (sha, time, subject)) in entries.iter().enumerate().rev() {
        let Some((what, rest)) = subject.split_once(": ") else { continue };
        // `pull --rebase` is a rebase too, and says `pull (start)`.
        let kind = what
            .strip_prefix("rebase -i ")
            .or_else(|| what.strip_prefix("rebase "))
            .or_else(|| what.strip_prefix("pull "));
        let Some(kind) = kind else { continue };
        if kind.contains("(start)") {
            if let Some(unfinished) = current.take() {
                rebases.push(unfinished);
            }
            current = Some(Rebase {
                branch: None,
                onto_name: rest.strip_prefix("checkout ").unwrap_or(rest).to_owned(),
                onto: (*sha).to_owned(),
                before: entries.get(i + 1).map(|(sha, ..)| (*sha).to_owned()),
                after: None,
                started: *time,
                ended: *time,
                aborted: false,
                picks: Vec::new(),
            });
            continue;
        }
        let Some(rebase) = current.as_mut() else { continue };
        rebase.ended = *time;
        if kind.contains("(finish)") {
            rebase.branch = rest.strip_prefix("returning to refs/heads/").map(str::to_owned);
            rebase.after = Some((*sha).to_owned());
            rebases.extend(current.take());
        } else if kind.contains("(abort)") {
            rebase.aborted = true;
            rebases.extend(current.take());
        } else if kind == "(continue)" {
            rebase.picks.push(Pick { commit: (*sha).to_owned(), subject: (*rest).to_owned(), stopped: true });
        } else if kind.contains("(pick)")
            || kind.contains("(reword)")
            || kind.contains("(fixup)")
            || kind.contains("(squash)")
        {
            rebase.picks.push(Pick { commit: (*sha).to_owned(), subject: (*rest).to_owned(), stopped: false });
        }
    }
    rebases.extend(current);
    rebases.reverse();
    rebases
}

/// The original of each of a rebase's picks, by author, author date and subject, which a pick
/// keeps: the commits of the branch before the rebase that were not in what it went onto.
pub fn originals(repo: &Path, rebase: &Rebase) -> Result<HashMap<String, String>> {
    let Some(before) = &rebase.before else { return Ok(HashMap::new()) };
    let format = "--format=%H%x00%at%x00%ae%x00%s";
    let not_onto = format!("^{}", rebase.onto);
    let branch = git(repo, &["log", format, before, &not_onto])?;
    let key = |line: &str| -> Option<(String, String)> {
        let (sha, rest) = line.split_once('\0')?;
        Some((rest.to_owned(), sha.to_owned()))
    };
    let by_key: HashMap<String, String> = branch.lines().filter_map(key).collect();
    let mut args = vec!["log", "--no-walk=unsorted", format];
    args.extend(rebase.picks.iter().map(|p| p.commit.as_str()));
    let picks = git(repo, &args)?;
    Ok(picks.lines().filter_map(key).filter_map(|(k, pick)| Some((pick, by_key.get(&k)?.clone()))).collect())
}

/// The picks that the rebase's target already had, rewritten (another rebase of the same
/// commits): by author, author date and subject, each pick with the target's commit. Picking
/// them again conflicts with themselves; `git rebase --onto` would have left them out.
pub fn upstream(repo: &Path, rebase: &Rebase) -> Result<HashMap<String, String>> {
    let Some(before) = &rebase.before else { return Ok(HashMap::new()) };
    let format = "--format=%H%x00%at%x00%ae%x00%s";
    let not_before = format!("^{before}");
    let onto = git(repo, &["log", format, &rebase.onto, &not_before])?;
    let key = |line: &str| -> Option<(String, String)> {
        let (sha, rest) = line.split_once('\0')?;
        Some((rest.to_owned(), sha.to_owned()))
    };
    let by_key: HashMap<String, String> = onto.lines().filter_map(key).collect();
    let mut args = vec!["log", "--no-walk=unsorted", format];
    args.extend(rebase.picks.iter().map(|p| p.commit.as_str()));
    let picks = git(repo, &args)?;
    Ok(picks.lines().filter_map(key).filter_map(|(k, pick)| Some((pick, by_key.get(&k)?.clone()))).collect())
}

/// What was decided by hand when `original` was picked as `rebased`.
pub fn decisions(repo: &Path, original: &str, rebased: &str) -> Result<Vec<Decision>> {
    let files = git(repo, &["diff", "--name-only", "--no-renames", &format!("{original}^"), original])?;
    let tmp = TempDir::create().context("no temporary directory")?;
    let mut decisions = Vec::new();
    for file in files.lines().filter(|f| !f.is_empty()) {
        let show = |commit: &str| show(repo, commit, file);
        let (Some(base), Some(theirs), Some(ours), Some(resolved)) =
            (show(&format!("{original}^")), show(original), show(&format!("{rebased}^")), show(rebased))
        else {
            continue;
        };
        let merged = merge_file(repo, tmp.path(), &ours, &base, &theirs)?;
        if merged == resolved {
            continue;
        }
        decisions.extend(compare(repo, tmp.path(), Path::new(file), &merged, &resolved)?);
    }
    Ok(decisions)
}

/// The decisions in `resolved` against git's `merged`, conflicts with their markers included.
fn compare(repo: &Path, tmp: &Path, file: &Path, merged: &str, resolved: &str) -> Result<Vec<Decision>> {
    let regions = conflicts(merged);
    let hunks = diff_hunks(repo, tmp, merged, resolved)?;
    let resolved_lines: Vec<&str> = resolved.lines().collect();
    let anchor = |first: u32| follow::anchor(&resolved_lines, first.max(1));
    let take = |first: u32, last: u32| -> Vec<String> {
        (first..=last).filter_map(|l| resolved_lines.get(l as usize - 1)).map(|s| (*s).to_owned()).collect()
    };
    let mut decisions: Vec<Decision> = Vec::new();
    let mut by_region: HashMap<usize, (u32, u32)> = HashMap::new();
    for hunk in &hunks {
        let (lo, hi) = hunk.new_range();
        let region = regions.iter().position(|r| hunk.old_start <= r.end && hunk.old_end() >= r.start);
        match region {
            Some(r) => {
                let span = by_region.entry(r).or_insert((lo, hi));
                *span = (span.0.min(lo), span.1.max(hi));
            }
            None => decisions.push(Decision {
                file: file.to_path_buf(),
                first: lo,
                last: hi,
                conflict: false,
                ours: Vec::new(),
                theirs: Vec::new(),
                resolved: take(lo, hi),
                anchor: anchor(lo),
            }),
        }
    }
    for (r, (lo, hi)) in by_region {
        let region = &regions[r];
        decisions.push(Decision {
            file: file.to_path_buf(),
            first: lo,
            last: hi,
            conflict: true,
            ours: region.ours.clone(),
            theirs: region.theirs.clone(),
            resolved: take(lo, hi),
            anchor: anchor(lo),
        });
    }
    decisions.sort_by_key(|d| d.first);
    Ok(decisions)
}

/// A conflict in a merged file: its lines from `<<<<<<<` to `>>>>>>>`, 1-based, and each side.
#[derive(Debug, PartialEq, Eq)]
struct Conflict {
    start: u32,
    end: u32,
    ours: Vec<String>,
    theirs: Vec<String>,
}

fn conflicts(merged: &str) -> Vec<Conflict> {
    let mut found = Vec::new();
    let mut open: Option<Conflict> = None;
    let mut in_theirs = false;
    for (i, line) in merged.lines().enumerate() {
        let number = u32::try_from(i + 1).unwrap_or(u32::MAX);
        if line.starts_with("<<<<<<< ") {
            open = Some(Conflict { start: number, end: number, ours: Vec::new(), theirs: Vec::new() });
            in_theirs = false;
        } else if let Some(conflict) = open.as_mut() {
            if line.starts_with("=======") {
                in_theirs = true;
            } else if line.starts_with(">>>>>>> ") {
                conflict.end = number;
                found.extend(open.take());
            } else if in_theirs {
                conflict.theirs.push(line.to_owned());
            } else if !line.starts_with("||||||| ") {
                conflict.ours.push(line.to_owned());
            }
        }
    }
    found
}

/// A hunk of `diff -U0`: lines `old_start..` of the old text became `new_start..`.
#[derive(Debug, PartialEq, Eq)]
struct Hunk {
    old_start: u32,
    old_len: u32,
    new_start: u32,
    new_len: u32,
}

impl Hunk {
    fn old_end(&self) -> u32 {
        (self.old_start + self.old_len).saturating_sub(1).max(self.old_start)
    }

    /// Its lines in the new text; for one that only removes, the line after, as `(n + 1, n)`.
    fn new_range(&self) -> (u32, u32) {
        if self.new_len == 0 {
            (self.new_start + 1, self.new_start)
        } else {
            (self.new_start, self.new_start + self.new_len - 1)
        }
    }
}

fn diff_hunks(repo: &Path, tmp: &Path, old: &str, new: &str) -> Result<Vec<Hunk>> {
    let (a, b) = (tmp.join("old"), tmp.join("new"));
    std::fs::write(&a, old)?;
    std::fs::write(&b, new)?;
    let a = a.to_string_lossy();
    let b = b.to_string_lossy();
    let (_, out) = git_status(repo, &["diff", "--no-index", "--no-color", "-U0", "--", &a, &b])?;
    Ok(out.lines().filter_map(parse_hunk).collect())
}

/// `@@ -12,3 +12,0 @@`; a count left out is 1.
fn parse_hunk(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let new = rest.split_once(" @@")?.0;
    let range = |text: &str| -> Option<(u32, u32)> {
        match text.split_once(',') {
            Some((start, len)) => Some((start.parse().ok()?, len.parse().ok()?)),
            None => Some((text.parse().ok()?, 1)),
        }
    };
    let (old_start, old_len) = range(old)?;
    let (new_start, new_len) = range(new)?;
    Some(Hunk { old_start, old_len, new_start, new_len })
}

/// git's three-way merge of `ours` and `theirs` from `base`, with conflict markers.
fn merge_file(repo: &Path, tmp: &Path, ours: &str, base: &str, theirs: &str) -> Result<String> {
    let paths = [tmp.join("ours"), tmp.join("base"), tmp.join("theirs")];
    for (path, text) in paths.iter().zip([ours, base, theirs]) {
        std::fs::write(path, text)?;
    }
    let names: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let mut args = vec!["merge-file", "-p", "-L", "ours", "-L", "base", "-L", "theirs"];
    args.extend(names.iter().map(String::as_str));
    let (code, merged) = git_status(repo, &args)?;
    // The number of conflicts, or negative when it could not merge.
    if !(0..128).contains(&code) {
        bail!("git merge-file failed ({code})");
    }
    Ok(merged)
}

/// What the branch added before the rebase (from where it left what it went onto to its tip
/// then) and has no longer after it: every line with a letter or a digit in it that the file
/// after the rebase does not have, those next to each other together, each with the line most
/// like it there.
pub fn lost(repo: &Path, rebase: &Rebase) -> Result<Vec<Lost>> {
    let (Some(before), Some(after)) = (&rebase.before, &rebase.after) else { return Ok(Vec::new()) };
    let base = git(repo, &["merge-base", before, &rebase.onto])?;
    let diff = git(repo, &["diff", "--no-color", "--no-renames", "-U0", base.trim(), before])?;
    let mut lost: Vec<Lost> = Vec::new();
    let mut file: Option<PathBuf> = None;
    let mut now: Vec<String> = Vec::new();
    let mut line = 0u32;
    for text in diff.lines() {
        if let Some(path) = text.strip_prefix("+++ ") {
            file = path.strip_prefix("b/").filter(|_| path != "/dev/null").map(PathBuf::from);
            now = file
                .as_ref()
                .and_then(|p| show(repo, after, &p.to_string_lossy()))
                .map(|t| t.lines().map(|l| l.trim().to_owned()).collect())
                .unwrap_or_default();
        } else if let Some(hunk) = parse_hunk(text) {
            line = hunk.new_start;
        } else if let (Some(added), Some(file)) = (text.strip_prefix('+'), &file) {
            let trimmed = added.trim();
            if trimmed.chars().any(char::is_alphanumeric) && !now.iter().any(|l| l == trimmed) {
                let lost_line =
                    LostLine { text: added.to_owned(), now: most_like(trimmed, &now), comment: comment(trimmed) };
                match lost.last_mut() {
                    Some(last) if last.file == *file && last.line + last.lines.len() as u32 == line => {
                        last.lines.push(lost_line);
                    }
                    _ => lost.push(Lost { file: file.clone(), line, lines: vec![lost_line] }),
                }
            }
            line += 1;
        }
    }
    Ok(lost)
}

/// A line that is all comment, in the languages and files a project has: `//`, `#` (but not
/// PHP's `#[` attributes), `;`, `/*`, `*`, `--`, `<!--`.
fn comment(trimmed: &str) -> bool {
    ["//", "/*", "*", ";", "--", "<!--"].iter().any(|p| trimmed.starts_with(p))
        || (trimmed.starts_with('#') && !trimmed.starts_with("#["))
}

/// The line of `lines` that has the most of `text`'s words, if it has at least two thirds.
fn most_like(text: &str, lines: &[String]) -> Option<String> {
    let words = |t: &str| -> Vec<String> {
        t.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).map(str::to_owned).collect()
    };
    let mine = words(text);
    if mine.is_empty() {
        return None;
    }
    let share = |line: &String| {
        let theirs = words(line);
        mine.iter().filter(|w| theirs.contains(w)).count() * 3
    };
    lines
        .iter()
        .map(|l| (share(l), l))
        .filter(|(s, _)| *s >= mine.len() * 2)
        .max_by_key(|(s, _)| *s)
        .map(|(_, l)| l.clone())
}

/// `commit:file`, or none when the commit has no such file.
fn show(repo: &Path, commit: &str, file: &str) -> Option<String> {
    git(repo, &["show", &format!("{commit}:{file}")]).ok()
}

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let (code, out) = git_status(repo, args)?;
    if code != 0 {
        bail!("git {} failed ({code})", args.first().unwrap_or(&""));
    }
    Ok(out)
}

/// Runs git in `repo`; its exit status and what it printed.
fn git_status(repo: &Path, args: &[&str]) -> Result<(i32, String)> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["-c", "core.quotepath=off"])
        .args(args)
        .output()
        .context("running git")?;
    Ok((output.status.code().unwrap_or(-1), String::from_utf8_lossy(&output.stdout).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reflog_reads_as_rebases_with_their_stops() {
        let log = "\
f9\0HEAD@{60}\0checkout: moving from feature to main
f8\0HEAD@{50}\0rebase (finish): returning to refs/heads/feature
f8\0HEAD@{50}\0rebase (continue) (pick): third
f7\0HEAD@{49}\0rebase (continue): second
f6\0HEAD@{40}\0rebase (pick): first
m1\0HEAD@{40}\0rebase (start): checkout main
f3\0HEAD@{30}\0checkout: moving from main to feature
a2\0HEAD@{20}\0rebase (abort): updating HEAD
m1\0HEAD@{10}\0rebase -i (start): checkout main
f3\0HEAD@{5}\0commit: third
";
        let rebases = parse_reflog(log);
        assert_eq!(rebases.len(), 2);
        let done = &rebases[0];
        assert_eq!(done.title(), "feature onto main");
        assert_eq!((done.onto.as_str(), done.before.as_deref(), done.after.as_deref()), ("m1", Some("f3"), Some("f8")));
        assert_eq!((done.started, done.ended), (40, 50));
        let picks: Vec<(&str, bool)> = done.picks.iter().map(|p| (p.subject.as_str(), p.stopped)).collect();
        assert_eq!(picks, [("first", false), ("second", true), ("third", false)]);
        assert!(rebases[1].aborted && rebases[1].after.is_none());
        assert_eq!(rebases[1].before.as_deref(), Some("f3"));
    }

    #[test]
    fn hunks_and_conflicts_read_as_git_writes_them() {
        assert_eq!(
            parse_hunk("@@ -12,3 +12,0 @@ fn x"),
            Some(Hunk { old_start: 12, old_len: 3, new_start: 12, new_len: 0 })
        );
        assert_eq!(parse_hunk("@@ -4 +5 @@"), Some(Hunk { old_start: 4, old_len: 1, new_start: 5, new_len: 1 }));
        let merged = "a\n<<<<<<< ours\nmine\n=======\nyours\n>>>>>>> theirs\nb\n";
        let found = conflicts(merged);
        assert_eq!(found, [Conflict { start: 2, end: 6, ours: vec!["mine".into()], theirs: vec!["yours".into()] }]);
    }

    /// A repository where `feature` changes a line `main` changes too, and adds a line to a
    /// file `main` rewrites; rebased onto main with the conflict resolved by hand, the added
    /// line dropped on the way.
    fn rebased_repo() -> tempdir::Repo {
        let repo = tempdir::Repo::new();
        repo.write("total.php", "<?php\nfunction total($o) {\n    $sum = 0;\n    return $sum;\n}\n");
        repo.write("Dockerfile", "FROM alpine\nRUN apk add curl\nCOPY . /app\n");
        repo.commit("start");
        repo.git(&["checkout", "-q", "-b", "feature"]);
        repo.write("total.php", "<?php\nfunction total($o) {\n    $sum = $o->base;\n    return $sum;\n}\n");
        repo.write(
            "Dockerfile",
            "FROM alpine\nRUN apk add curl nginx\nCOPY ./site.conf /etc/site.conf\nCOPY . /app\n",
        );
        repo.commit("feature: start from the base");
        repo.git(&["checkout", "-q", "main"]);
        repo.write("total.php", "<?php\nfunction total($o) {\n    $sum = $o->opening;\n    return $sum;\n}\n");
        repo.write("Dockerfile", "FROM alpine\nRUN apk add curl git\nCOPY . /app\n");
        repo.commit("main: start from the opening");
        repo.git(&["checkout", "-q", "feature"]);
        repo.git_may_fail(&["rebase", "main"]);
        // Both sides of the total; main's Dockerfile.
        repo.write(
            "total.php",
            "<?php\nfunction total($o) {\n    $sum = $o->opening + $o->base;\n    return $sum;\n}\n",
        );
        repo.write("Dockerfile", "FROM alpine\nRUN apk add curl git\nCOPY . /app\n");
        repo.git(&["add", "-A"]);
        repo.git(&["-c", "core.editor=true", "rebase", "--continue"]);
        repo
    }

    #[test]
    fn a_rebase_shows_what_was_decided_by_hand_and_what_was_lost() {
        let repo = rebased_repo();
        let rebases = rebases(repo.path()).expect("rebases");
        assert_eq!(rebases.len(), 1);
        let rebase = &rebases[0];
        assert_eq!(rebase.title(), "feature onto main");
        let stop = rebase.stops().next().expect("a stop");
        assert_eq!(stop.subject, "feature: start from the base");

        let originals = originals(repo.path(), rebase).expect("originals");
        let original = originals.get(&stop.commit).expect("the original of the stop");
        let decisions = decisions(repo.path(), original, &stop.commit).expect("decisions");
        let total = decisions.iter().find(|d| d.file == Path::new("total.php")).expect("total.php");
        assert!(total.conflict);
        assert_eq!((total.first, total.last), (3, 3));
        assert_eq!(total.ours, ["    $sum = $o->opening;"]);
        assert_eq!(total.theirs, ["    $sum = $o->base;"]);
        assert_eq!(total.resolved, ["    $sum = $o->opening + $o->base;"]);

        let lost = lost(repo.path(), rebase).expect("lost");
        let line = |text: &str| lost.iter().flat_map(|l| &l.lines).find(|l| l.text.contains(text)).cloned();
        let copy = line("site.conf").expect("the COPY of site.conf");
        assert_eq!(copy.now, None, "gone: nothing after the rebase is like it");
        let install = line("nginx").expect("the install of nginx");
        assert_eq!(install.now.as_deref(), Some("RUN apk add curl git"), "changed by the resolution");
        let total = line("$o->base").expect("the branch's total");
        assert_eq!(total.now.as_deref(), Some("$sum = $o->opening + $o->base;"));
    }

    mod tempdir {
        use std::path::{Path, PathBuf};
        use std::process::Command;

        /// A git repository in a temporary directory, removed when dropped.
        pub struct Repo {
            dir: PathBuf,
        }

        impl Repo {
            pub fn new() -> Repo {
                let dir = std::env::temp_dir().join(format!(
                    "tuitor-rebase-{}-{:?}",
                    std::process::id(),
                    std::thread::current().id()
                ));
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(&dir).expect("dir");
                let repo = Repo { dir };
                repo.git(&["init", "-q", "-b", "main"]);
                repo
            }

            pub fn path(&self) -> &Path {
                &self.dir
            }

            pub fn write(&self, file: &str, text: &str) {
                std::fs::write(self.dir.join(file), text).expect("write");
            }

            pub fn commit(&self, message: &str) {
                self.git(&["add", "-A"]);
                self.git(&["commit", "-q", "-m", message]);
            }

            pub fn git(&self, args: &[&str]) {
                assert!(self.run(args), "git {args:?}");
            }

            pub fn git_may_fail(&self, args: &[&str]) {
                self.run(args);
            }

            fn run(&self, args: &[&str]) -> bool {
                Command::new("git")
                    .arg("-C")
                    .arg(&self.dir)
                    .args(args)
                    .env("GIT_AUTHOR_NAME", "t")
                    .env("GIT_AUTHOR_EMAIL", "t@example.com")
                    .env("GIT_COMMITTER_NAME", "t")
                    .env("GIT_COMMITTER_EMAIL", "t@example.com")
                    .output()
                    .expect("git")
                    .status
                    .success()
            }
        }

        impl Drop for Repo {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.dir);
            }
        }
    }
}
