//! Debugging a container image's build. BuildKit speaks DAP (`docker buildx dap build`): a
//! breakpoint goes on a line of the Dockerfile, the build stops at it, and a step that fails
//! stops the build where it failed, with the step's arguments and its file system, and a
//! shell in the failed step's container on asking.
//!
//! A failed step's last line seldom says why: `cp: cannot stat 'modules/*'`, `make: ***
//! Error 1`, `E: Unable to locate package jq`. So when one fails tuitor says three things, each with
//! where it comes from:
//! - what of the step it was at: of the command's words, the one its output names last
//!   before the error;
//! - what the Dockerfile shows: a variable the step uses that only another RUN sets, which is
//!   empty here, as every RUN is a shell of its own;
//! - what commands run in the failed step's container show: whether a PHP extension is
//!   already in the image (`php -m`), where a missing header is, what a variable holds.

use std::collections::BTreeMap;
use std::time::Duration;

/// A failed step as the debugger shows it: its frame's name (`[build  3/9] RUN pecl
/// install imagick-3.8.0`), its line of the Dockerfile, and what it printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub step: String,
    pub line: u32,
    /// The command of a RUN, without `RUN`.
    pub command: String,
    /// What the step printed, without BuildKit's `#7 1.234 ` in front.
    pub output: Vec<String>,
}

impl Failure {
    /// `step` and `line` from the stopped frame, its output from the build's.
    pub fn new(step: &str, line: u32, build_output: &[&str]) -> Failure {
        let command = step.split_once("] ").map_or(step, |(_, rest)| rest);
        let command = command.strip_prefix("RUN ").unwrap_or(command).trim().to_owned();
        Failure { step: step.to_owned(), line, command, output: step_output(build_output, step) }
    }

    /// The first line near the step's end that says what went wrong: the cause comes before
    /// what it brings down (`fatal error: …` before `make: *** Error 1`).
    pub fn error(&self) -> Option<&str> {
        let wrong = |l: &&String| {
            let lower = l.to_lowercase();
            // apt's `E: …` starts its line; `configure: …` must not count.
            l.starts_with("E: ")
                || ["error:", "error 1", "fatal", "cannot ", "no such", "not found", "failed"]
                    .iter()
                    .any(|w| lower.contains(w))
        };
        let near_end = self.output.len().saturating_sub(40);
        self.output[near_end..].iter().find(wrong).map(String::as_str)
    }
}

/// The lines BuildKit printed for `step`: those of its vertex, `#7 …` after `#7 [build  3/9]
/// RUN …`, without that prefix and the time after it.
fn step_output(lines: &[&str], step: &str) -> Vec<String> {
    let mut vertex = None;
    for line in lines {
        if let Some((number, rest)) = line.strip_prefix('#').and_then(|l| l.split_once(' '))
            && rest.trim() == step.trim()
        {
            vertex = Some(number.to_owned());
        }
    }
    let Some(vertex) = vertex else { return Vec::new() };
    let prefix = format!("#{vertex} ");
    lines
        .iter()
        .filter_map(|line| line.strip_prefix(&prefix))
        .filter(|rest| !rest.starts_with('[') && !rest.starts_with("ERROR"))
        .map(|rest| {
            // `1.234 text`: the seconds since the step began.
            match rest.split_once(' ') {
                Some((secs, text)) if secs.parse::<f64>().is_ok() => text.to_owned(),
                _ => rest.to_owned(),
            }
        })
        .collect()
}

/// Words a command gives its tools to work on: what it installs, builds or fetches.
const VERBS: &[&str] =
    &["install", "add", "update", "upgrade", "remove", "enable", "configure", "build", "get", "run", "exec"];

/// What of the step it was at when it failed: of the command's words, the one its output
/// names last before the error, with its place among them and how many there are. A word
/// with a version (`imagick-3.8.0`) is named by its name too.
pub fn failed_item(failure: &Failure) -> Option<(String, usize, usize)> {
    let words = items(&failure.command);
    if words.is_empty() {
        return None;
    }
    let mut best: Option<(usize, usize)> = None;
    for (place, word) in words.iter().enumerate() {
        let names = [word.as_str(), name_of(word)];
        let last = failure.output.iter().rposition(|line| names.iter().any(|n| has_word(line, n)));
        if let Some(at) = last
            && best.is_none_or(|(b, _)| at >= b)
        {
            best = Some((at, place));
        }
    }
    let (_, place) = best?;
    Some((words[place].clone(), place + 1, words.len()))
}

/// The arguments of a command's tools, the tools themselves, flags, variables, paths and verbs
/// left out.
fn items(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut first = true;
    for raw in command.split_whitespace() {
        let word = raw.trim_end_matches([';', '\\', ')', '"', '\'']);
        let ends = raw.ends_with(';') || ["&&", "||", "|", ";"].contains(&raw);
        let plain = !word.is_empty()
            && word.chars().all(|c| c.is_alphanumeric() || "_.+-:@".contains(c))
            && word.chars().next().is_some_and(char::is_alphanumeric)
            && word.len() >= 2;
        if !first && plain && !VERBS.contains(&word) {
            words.push(word.to_owned());
        }
        first = ends || ["&&", "||", "|", ";"].contains(&raw);
    }
    words
}

/// `imagick-3.8.0` is named `imagick`; `pkg@1.2` and `pkg==1.2` likewise.
fn name_of(word: &str) -> &str {
    for sep in ["==", "@", ":", "-"] {
        if let Some((name, version)) = word.split_once(sep)
            && version.chars().next().is_some_and(|c| c.is_ascii_digit())
        {
            return name;
        }
    }
    word
}

/// `word` in `line` as a word: no letter, digit or `_` right before or after it.
fn has_word(line: &str, word: &str) -> bool {
    let inner = |c: char| c.is_alphanumeric() || c == '_';
    line.match_indices(word).any(|(at, _)| {
        let before = line[..at].chars().next_back().is_none_or(|c| !inner(c));
        let after = line[at + word.len()..].chars().next().is_none_or(|c| !inner(c));
        before && after
    })
}

/// Shell variables the step uses that the Dockerfile does not give it as an `ARG` or `ENV`
/// before its line, and that an earlier `RUN` sets, with that RUN's line: every RUN is a
/// shell of its own, so the variable is empty here.
pub fn foreign_variables(dockerfile: &str, failure: &Failure) -> Vec<(String, u32)> {
    let instructions = instructions(dockerfile);
    let before = |line: u32| line < failure.line;
    let declared: Vec<String> = instructions
        .iter()
        .filter(|(line, _)| before(*line))
        .filter_map(|(_, text)| {
            let (keyword, rest) = text.split_once(char::is_whitespace)?;
            matches!(keyword.to_uppercase().as_str(), "ARG" | "ENV").then(|| rest.to_owned())
        })
        .flat_map(|rest| {
            rest.split_whitespace().map(|w| w.split('=').next().unwrap_or(w).to_owned()).collect::<Vec<_>>()
        })
        .collect();
    let mut found = BTreeMap::new();
    for name in variables(&failure.command) {
        if declared.contains(&name) {
            continue;
        }
        let set_by = instructions.iter().filter(|(line, _)| before(*line)).find(|(_, text)| {
            text.split_once(char::is_whitespace).is_some_and(|(k, _)| k.eq_ignore_ascii_case("RUN"))
                && assigns(text, &name)
        });
        if let Some((line, _)) = set_by {
            found.insert(name, *line);
        }
    }
    found.into_iter().collect()
}

/// The first line of the instruction `line` is in: BuildKit gives a RUN over many lines by its
/// last, `docker build` by its first.
pub fn instruction_start(dockerfile: &str, line: u32) -> u32 {
    let starts: Vec<u32> = instructions(dockerfile).iter().map(|(start, _)| *start).collect();
    starts.into_iter().take_while(|start| *start <= line).last().unwrap_or(line)
}

/// The instructions of a Dockerfile, each with its first line, continuation lines joined.
fn instructions(dockerfile: &str) -> Vec<(u32, String)> {
    let mut found: Vec<(u32, String)> = Vec::new();
    let mut open = false;
    for (i, line) in dockerfile.lines().enumerate() {
        let number = u32::try_from(i + 1).unwrap_or(u32::MAX);
        let text = line.trim_end();
        let continues = text.ends_with('\\');
        let text = text.trim_end_matches('\\');
        match found.last_mut() {
            Some((_, joined)) if open => {
                joined.push(' ');
                joined.push_str(text.trim());
            }
            _ if text.trim().is_empty() || text.trim_start().starts_with('#') => {}
            _ => found.push((number, text.trim().to_owned())),
        }
        open = continues || (open && text.trim().is_empty());
    }
    found
}

/// The names of the `$NAME` and `${NAME}` in `command`.
fn variables(command: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = command;
    while let Some(at) = rest.find('$') {
        rest = &rest[at + 1..];
        let braced = rest.strip_prefix('{');
        let text = braced.unwrap_or(rest);
        let name: String = text.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if !name.is_empty() && !name.chars().next().is_some_and(|c| c.is_ascii_digit()) && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// `NAME=` in `text`, an assignment and not a use.
fn assigns(text: &str, name: &str) -> bool {
    let pattern = format!("{name}=");
    text.match_indices(&pattern).any(|(at, _)| {
        text[..at].chars().next_back().is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '$' || c == '{'))
    })
}

/// A header the compiler could not find: `fatal error: ext/standard/php_smart_string.h: No
/// such file or directory`.
pub fn missing_header(failure: &Failure) -> Option<String> {
    failure.output.iter().find_map(|line| {
        let rest = line.split_once("fatal error: ")?.1;
        let (header, why) = rest.split_once(": ")?;
        why.contains("No such file").then(|| header.trim().to_owned())
    })
}

/// A `pecl install name-version` of the step: the package and the version it asks for.
pub fn pecl_package(failure: &Failure) -> Option<(String, String)> {
    let mut words = failure.command.split_whitespace().map(|w| w.trim_end_matches([';', '\\']));
    while let Some(word) = words.next() {
        if word == "pecl" && words.next() == Some("install") {
            let package = words.find(|w| !w.starts_with('-'))?;
            let (name, version) = package.split_once('-')?;
            return Some((name.to_owned(), version.to_owned()));
        }
    }
    None
}

/// Shell commands, each with what it is asked for, to run in the failed step's container.
pub fn probes(failure: &Failure, item: Option<&str>, foreign: &[(String, u32)]) -> Vec<(Probe, String)> {
    let mut probes = Vec::new();
    let php = ["docker-php-ext-install", "docker-php-ext-enable", "pecl "].iter().any(|t| failure.command.contains(t));
    if php {
        probes.push((Probe::PhpVersion, "php -v | head -1".to_owned()));
        probes.push((Probe::PhpModules, "php -m".to_owned()));
    }
    if failure.command.contains("docker-php-ext-install") {
        // The extension it was building: the directory of the PHP sources touched last.
        probes.push((Probe::NewestExtension, "ls -1t /usr/src/php/ext | head -1".to_owned()));
    }
    if let Some(header) = missing_header(failure) {
        let name = header.rsplit('/').next().unwrap_or(&header).to_owned();
        // `php_smart_string.h` → `smart_string`: the name without the prefix of who it was from.
        let bare = name.trim_end_matches(".h");
        let stem = bare.split_once('_').map_or(bare, |(_, rest)| rest).to_owned();
        probes.push((Probe::Header(name.clone()), format!("find / -xdev -name '{name}' 2>/dev/null | head -3")));
        probes.push((Probe::LikeHeader(name), format!("find / -xdev -name '*{stem}*.h' 2>/dev/null | head -5")));
    }
    if let Some(item) =
        item.filter(|_| failure.command.contains("apt-get install") || failure.command.contains("apt install"))
    {
        probes.push((Probe::AptPolicy(item.to_owned()), format!("apt-cache policy {item} | head -3")));
    }
    for (name, _) in foreign {
        probes.push((Probe::Variable(name.clone()), format!("echo \"[${name}]\"")));
    }
    probes
}

/// Each probe's output lines.
pub type Answers = Vec<(Probe, Vec<String>)>;

/// A failed step being looked into, until the probes' answers come.
#[derive(Debug, Clone)]
pub struct Probing {
    pub failure: Failure,
    pub item: Option<(String, usize, usize)>,
    pub foreign: Vec<(String, u32)>,
    pub probes: Vec<(Probe, String)>,
    /// The terminal the probes run in has been given.
    pub started: bool,
    /// The probes' answers and the newest release, once in; the step's log may come later.
    pub answered: Option<(Answers, Option<String>)>,
    /// What was said already, not to say it twice.
    pub said: Vec<String>,
}

/// Runs `args`, the command the adapter gives for a terminal into the failed step's container,
/// on a pseudo-terminal of tuitor's own: types `script` once the shell is there, and returns
/// what it printed when it exits, or after `limit`. Blocks: run it on a thread of its own.
#[cfg(unix)]
pub fn run_probes(args: &[String], script: &str, limit: Duration) -> String {
    use std::io::{Read, Write};
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let Some((program, rest)) = args.split_first() else { return String::new() };
    let Ok(pty) = nix::pty::openpty(None, None) else { return String::new() };
    let stdio = || pty.slave.try_clone().map(Stdio::from);
    let (Ok(stdin), Ok(stdout), Ok(stderr)) = (stdio(), stdio(), stdio()) else { return String::new() };
    let Ok(mut child) =
        Command::new(program).args(rest).stdin(stdin).stdout(stdout).stderr(stderr).process_group(0).spawn()
    else {
        return String::new();
    };
    drop(pty.slave);
    let mut master = std::fs::File::from(pty.master);
    let Ok(mut reader) = master.try_clone() else { return String::new() };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        // The pseudo-terminal reads as an error once the shell has exited.
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let started = std::time::Instant::now();
    let mut printed = Vec::new();
    // When the script was typed, and how many times: the attaching client sets the terminal up
    // after it says it is running the shell, and that throws away what was typed before.
    let mut typed: Option<std::time::Instant> = None;
    let mut tries = 0;
    while started.elapsed() < limit {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(bytes) => printed.extend(bytes),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        let text = String::from_utf8_lossy(&printed);
        // The shell ends a line with `\r\r\n` on the pseudo-terminal.
        if text.contains("@@tuitor end\r") || child.try_wait().ok().flatten().is_some() {
            break;
        }
        // The shell is there once its prompt is, after the client said it runs it.
        let prompt = text.split_once("Running").is_some_and(|(_, after)| after.contains("# ") || after.contains("$ "));
        let answered = text.contains("@@tuitor 0\r");
        let again = typed.is_some_and(|at| at.elapsed() > Duration::from_secs(2)) && !answered;
        if prompt && tries < 3 && (typed.is_none() || again) {
            tries += 1;
            typed = master.write_all(script.as_bytes()).is_ok().then(std::time::Instant::now);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    String::from_utf8_lossy(&printed).into_owned()
}

#[cfg(not(unix))]
pub fn run_probes(_args: &[String], _script: &str, _limit: Duration) -> String {
    String::new()
}

/// The newest release of a PECL package, from PECL's REST interface, if it answers soon.
pub fn pecl_newest(package: &str) -> Option<String> {
    let url = format!("https://pecl.php.net/rest/r/{package}/latest.txt");
    let curl = std::path::Path::new("curl");
    let args = ["-fsS", "--max-time", "8", url.as_str()];
    let mut ask = crate::process::Ask::new(curl, &args);
    ask.timeout = Duration::from_secs(10);
    let answer = crate::process::ask(&ask)?;
    let version = answer.stdout.trim();
    (answer.success() && !version.is_empty() && version.len() < 32).then(|| version.to_owned())
}

/// What a probe asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    PhpVersion,
    PhpModules,
    /// The PHP extension's source directory touched last.
    NewestExtension,
    /// Where a header the compiler could not find is.
    Header(String),
    /// Headers with a name like it.
    LikeHeader(String),
    AptPolicy(String),
    Variable(String),
}

/// The shell script that runs `probes` and exits, each one's output after a marker line.
pub fn script(probes: &[(Probe, String)]) -> String {
    let mut script = String::new();
    for (i, (_, command)) in probes.iter().enumerate() {
        script.push_str(&format!("echo '@@tuitor {i}'; {command}; "));
    }
    script.push_str("echo '@@tuitor end'; exit\n");
    script
}

/// Each probe's output, from what the shell printed: the lines between its marker and the next.
pub fn answers(probes: &[(Probe, String)], printed: &str) -> Vec<(Probe, Vec<String>)> {
    let mut answers: Vec<(Probe, Vec<String>)> = Vec::new();
    let mut current: Option<usize> = None;
    for line in printed.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(marker) = line.strip_prefix("@@tuitor ") {
            current = marker.trim().parse().ok();
            // A probe that prints nothing has an answer too: nothing.
            if let Some((probe, _)) = current.and_then(|i| probes.get(i))
                && !answers.iter().any(|(p, _)| p == probe)
            {
                answers.push((probe.clone(), Vec::new()));
            }
            continue;
        }
        // The shell echoes the script as it was typed; that line holds every marker.
        if line.contains("echo '@@tuitor") {
            continue;
        }
        if let Some(i) = current
            && let Some((probe, _)) = probes.get(i)
        {
            match answers.iter_mut().find(|(p, _)| p == probe) {
                Some((_, lines)) => lines.push(line.to_owned()),
                None => answers.push((probe.clone(), vec![line.to_owned()])),
            }
        }
    }
    answers
}

/// What the failure is, in lines to read: the step, what of it failed, and what the
/// Dockerfile and the probes show. `newest` is the newest release of a `pecl install`'s
/// package, when known.
pub fn explain(
    failure: &Failure,
    item: Option<&(String, usize, usize)>,
    foreign: &[(String, u32)],
    answers: &[(Probe, Vec<String>)],
    newest: Option<&str>,
) -> Vec<String> {
    let mut said = vec![format!("Dockerfile:{} {} failed", failure.line, failure.step)];
    if let Some(error) = failure.error() {
        said.push(format!("  its error: {}", error.trim()));
    }
    let answer = |probe: &Probe| answers.iter().find(|(p, _)| p == probe).map(|(_, lines)| lines.as_slice());
    // The extension directory touched last says what it was at more surely than the log.
    let words = items(&failure.command);
    let touched = answer(&Probe::NewestExtension)
        .and_then(|lines| lines.iter().map(|l| l.trim()).find(|l| !l.is_empty()))
        .and_then(|dir| words.iter().position(|w| w == dir).map(|at| (words[at].clone(), at + 1, words.len())));
    let item_owned = touched.clone().or_else(|| item.cloned());
    let item = item_owned.as_ref();
    match (&touched, item) {
        (Some((word, place, of)), _) if *of > 1 => said.push(format!(
            "  at `{word}`, {place} of the {of} it names: its directory of PHP's sources is the one the step touched last"
        )),
        (None, Some((word, place, of))) if *of > 1 => said.push(format!(
            "  at `{word}`, {place} of the {of} it names: the last of them its output names before the error"
        )),
        _ => {}
    }
    let php = answer(&Probe::PhpVersion).and_then(|l| l.first()).map(|v| v.split(" (").next().unwrap_or(v).to_owned());
    if let (Some((word, ..)), Some(modules)) = (item, answer(&Probe::PhpModules)) {
        let name = name_of(word).to_lowercase();
        if let Some(module) = modules.iter().find(|m| m.to_lowercase().contains(&name)) {
            let php = php.clone().unwrap_or_else(|| "this PHP".into());
            said.push(format!(
                "  `php -m` in the failed step lists `{}`: {php} has {name} in it already, so there is nothing to build or install for it; leave it out",
                module.trim()
            ));
        }
    }
    for (probe, lines) in answers {
        match probe {
            Probe::Header(name) if lines.iter().all(|l| l.trim().is_empty()) => {
                let like: Vec<&str> = answer(&Probe::LikeHeader(name.clone()))
                    .unwrap_or_default()
                    .iter()
                    .map(|l| l.trim())
                    .filter(|l| !l.is_empty())
                    .collect();
                let php = php.clone().unwrap_or_else(|| "this image".into());
                let mut line = format!(
                    "  no `{name}` anywhere in the failed step's container: what it builds expects a header {php} does not have, so it was written for another version"
                );
                if !like.is_empty() {
                    line.push_str(&format!("; headers like it here: {}", like.join(", ")));
                }
                said.push(line);
            }
            Probe::AptPolicy(package)
                if lines.iter().any(|l| l.contains("(none)")) || lines.iter().all(|l| l.trim().is_empty()) =>
            {
                said.push(format!(
                    "  `apt-cache policy {package}` finds no such package in the image's package sources"
                ));
            }
            Probe::Variable(name) => {
                let set_at = foreign.iter().find(|(n, _)| n == name).map(|(_, l)| *l).unwrap_or_default();
                let value = lines.iter().find(|l| l.starts_with('[')).map(|l| l.trim().to_owned()).unwrap_or_default();
                said.push(format!(
                    "  `${name}` is set by the RUN at Dockerfile:{set_at}, a shell of its own: in this RUN it is {}; set it in the RUN that uses it, or make it an ARG or ENV",
                    if value == "[]" { "empty".to_owned() } else { value }
                ));
            }
            _ => {}
        }
    }
    if let (Some((package, asked)), Some(newest)) = (pecl_package(failure), newest)
        && newest != asked
    {
        said.push(format!("  PECL's newest {package} is {newest}; the step asks for {asked}"));
    }
    said
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(step: &str, line: u32, output: &[&str]) -> Failure {
        let mut lines = vec![format!("#9 {step}")];
        lines.extend(output.iter().enumerate().map(|(i, l)| format!("#9 {}.{:03} {l}", i / 10, i % 10)));
        lines.push(format!(
            "#9 ERROR: process \"/bin/sh -c {}\" did not complete successfully: exit code: 2",
            &step[step.find("RUN ").unwrap_or(0) + 4..]
        ));
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        Failure::new(step, line, &refs)
    }

    #[test]
    fn an_extension_php_has_built_in_is_found_and_said_to_be_there() {
        let f = failure(
            "[build  4/9] RUN docker-php-ext-install     bcmath     gd     intl     opcache     zip;",
            20,
            &[
                "+ strip --strip-all modules/intl.so",
                "Installing shared extensions:     /usr/local/lib/php/extensions/no-debug-non-zts-20250925/",
                "rm -f ext/opcache/jit/ir/minilua",
                "checking whether to support opcache JIT disassembly through Capstone... no",
                "Installing shared extensions:     /usr/local/lib/php/extensions/no-debug-non-zts-20250925/",
                "cp: cannot stat 'modules/*': No such file or directory",
                "make: *** [Makefile:89: install-modules] Error 1",
            ],
        );
        assert_eq!(f.error(), Some("cp: cannot stat 'modules/*': No such file or directory"));
        let item = failed_item(&f).expect("item");
        assert_eq!(item, ("opcache".to_owned(), 4, 5));
        let probes = probes(&f, Some(&item.0), &[]);
        let printed = "# echo '@@tuitor 0'; php -v | head -1; ...\n@@tuitor 0\nPHP 8.5.0 (cli) (built: Oct 28 2026)\n@@tuitor 1\n[PHP Modules]\nCore\nZend OPcache\n\n[Zend Modules]\nZend OPcache\n@@tuitor end\n";
        let said = explain(&f, Some(&item), &[], &answers(&probes, printed), None).join("\n");
        assert!(said.contains("at `opcache`, 4 of the 5"), "{said}");
        assert!(
            said.contains("`php -m` in the failed step lists `Zend OPcache`: PHP 8.5.0 has opcache in it already"),
            "{said}"
        );
    }

    #[test]
    fn a_missing_header_is_looked_for_and_a_newer_release_named() {
        let f = failure(
            "[build  3/9] RUN pecl install imagick-3.8.0",
            17,
            &[
                "downloading imagick-3.8.0.tgz ...",
                "/tmp/pear/temp/imagick/imagick.c:28:10: fatal error: ext/standard/php_smart_string.h: No such file or directory",
                "make: *** [Makefile:226: imagick.lo] Error 1",
                "ERROR: `make' failed",
            ],
        );
        assert_eq!(missing_header(&f).as_deref(), Some("ext/standard/php_smart_string.h"));
        assert_eq!(pecl_package(&f), Some(("imagick".into(), "3.8.0".into())));
        let probes = probes(&f, None, &[]);
        let printed = "@@tuitor 0\nPHP 8.5.0 (cli)\n@@tuitor 1\nCore\n@@tuitor 2\n@@tuitor 3\n/usr/local/include/php/Zend/zend_smart_string.h\n@@tuitor end\n";
        let said = explain(&f, None, &[], &answers(&probes, printed), Some("3.8.1")).join("\n");
        assert!(said.contains("no `php_smart_string.h` anywhere"), "{said}");
        assert!(said.contains("headers like it here: /usr/local/include/php/Zend/zend_smart_string.h"), "{said}");
        assert!(said.contains("PECL's newest imagick is 3.8.1; the step asks for 3.8.0"), "{said}");
    }

    #[test]
    fn a_run_over_many_lines_is_given_by_its_first() {
        let dockerfile = "FROM php\nRUN docker-php-ext-install \\\n    bcmath \\\n    zip;\nRUN true\n";
        assert_eq!(instruction_start(dockerfile, 4), 2);
        assert_eq!(instruction_start(dockerfile, 5), 5);
    }

    #[test]
    fn a_variable_another_run_sets_is_said_to_be_empty_here() {
        let dockerfile = "FROM debian AS build\nARG EXTRA=jq\nRUN set -eux; \\\n    packages=\"$(cat /packages.txt)\"; \\\n    apt-get update;\nRUN apt-get install -y curl\nRUN apt-get install -y $packages $EXTRA\n";
        let f = failure("[build  6/9] RUN apt-get install -y $packages $EXTRA", 7, &["E: Unable to locate package jq"]);
        let foreign = foreign_variables(dockerfile, &f);
        assert_eq!(foreign, [("packages".to_owned(), 3)]);
        let probes = probes(&f, None, &foreign);
        let printed = "@@tuitor 0\n[]\n@@tuitor end\n";
        let said = explain(&f, None, &foreign, &answers(&probes, printed), None).join("\n");
        assert!(
            said.contains(
                "`$packages` is set by the RUN at Dockerfile:3, a shell of its own: in this RUN it is empty"
            ),
            "{said}"
        );
    }
}
