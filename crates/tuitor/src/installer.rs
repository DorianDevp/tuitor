//! INSTALL (`/install`): what installing an adapter downloads, from where and into which
//! directory, or runs; done on a thread of its own once `y` says so, then checked the way
//! starting a session checks, with `locate`.

use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError};

use tuitor_core::config::Missing;
use tuitor_core::config::{Adapters, home_relative};
use tuitor_core::doctor::{self, Status};
use tuitor_core::install::{self, Found, Output, Package, Plan};

pub enum State {
    /// Shown, until `y`.
    Asking,
    Running(Receiver<Progress>),
    /// What `locate` says of each adapter, or why installing failed.
    Done(Result<Vec<String>, String>),
}

pub enum Progress {
    Step(String),
    Done(Result<(), String>),
}

pub struct Installer {
    pub plan: Plan,
    pub state: State,
    /// The steps taken so far.
    pub steps: Vec<String>,
}

impl Installer {
    pub fn new(plan: Plan) -> Installer {
        Installer { plan, state: State::Asking, steps: Vec::new() }
    }

    pub fn package(&self) -> Package {
        match &self.plan {
            Plan::Release(release) => release.package,
            Plan::Go(_) => Package::Delve,
            Plan::Distro { package, .. } => *package,
        }
    }

    /// Whether `y` installs: tuitor installs no package of the distribution, and needs its tools.
    pub fn can_start(&self) -> bool {
        match &self.plan {
            Plan::Release(release) => release.missing_tools().is_empty(),
            Plan::Go(_) => true,
            Plan::Distro { .. } => false,
        }
    }

    /// Starts installing on a thread of its own.
    pub fn start(&mut self) {
        if !self.can_start() || !matches!(self.state, State::Asking) {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let plan = self.plan.clone();
        std::thread::spawn(move || {
            let progress = tx.clone();
            let mut step = move |text: String| {
                let _ = progress.send(Progress::Step(text));
            };
            let result = match &plan {
                Plan::Release(release) => release.install(Output::Quiet, &mut step),
                Plan::Go(go) => go.install(Output::Quiet, &mut step).map(drop),
                Plan::Distro { .. } => Ok(()),
            };
            let _ = tx.send(Progress::Done(result.map_err(|err| format!("{err:#}"))));
        });
        self.state = State::Running(rx);
    }

    /// Takes in the steps taken, and once done, checks what the adapters now find.
    pub fn poll(&mut self, adapters: &Adapters, cwd: &Path) {
        let State::Running(rx) = &self.state else { return };
        loop {
            match rx.try_recv() {
                Ok(Progress::Step(step)) => self.steps.push(step),
                Ok(Progress::Done(Ok(()))) => {
                    self.state = State::Done(Ok(found(self.package(), adapters, cwd)));
                    return;
                }
                Ok(Progress::Done(Err(err))) => {
                    self.state = State::Done(Err(err));
                    return;
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.state = State::Done(Err("the installation stopped without a word".into()));
                    return;
                }
            }
        }
    }

    pub fn running(&self) -> bool {
        matches!(self.state, State::Running(_))
    }

    /// How it went, in a line, once done.
    pub fn outcome(&self) -> Option<(String, bool)> {
        let package = self.package();
        match &self.state {
            State::Done(Ok(found)) => {
                let ok = found.iter().all(|line| line.starts_with('✓'));
                let how = if ok { "installed" } else { "installed, but" };
                Some((format!("{package} {how}: {}", found.join("; ")), ok))
            }
            State::Done(Err(err)) => Some((format!("installing {package} failed: {err}"), false)),
            State::Asking | State::Running(_) => None,
        }
    }
}

/// A line per adapter of `package`, or per adapters that run the same: `✓` and what they
/// run, or `✗` and what is still missing.
pub fn found(package: Package, adapters: &Adapters, cwd: &Path) -> Vec<String> {
    // No adapter runs Xdebug; the PHP here says whether it has it.
    if package == Package::Xdebug {
        let finding = doctor::php_xdebug("php");
        let mark = if finding.status == Status::Problem { "✗" } else { "✓" };
        return vec![format!("{mark} {}", finding.text)];
    }
    let mut lines: Vec<(Vec<String>, String)> = Vec::new();
    for (name, found) in install::verify(package, adapters, cwd) {
        let line = describe(adapters, &name, found);
        match lines.iter_mut().find(|(_, other)| *other == line) {
            Some((names, _)) => names.push(name),
            None => lines.push((vec![name], line)),
        }
    }
    let line = |(names, line): (Vec<String>, String)| match line.strip_prefix("✗ ") {
        Some(missing) => format!("✗ {}: {missing}", names.join(", ")),
        None => format!("✓ {}: {line}", names.join(", ")),
    };
    lines.into_iter().map(line).collect()
}

/// What an adapter runs, or `✗` and what is missing.
fn describe(adapters: &Adapters, name: &str, found: Found) -> String {
    match found {
        Found::Runs(program) => runs(adapters, name, &program),
        Found::Missing(Missing::OffPath { program, dir }) => {
            let export = format!("export PATH=\"$PATH:{}\"", dir.display());
            let missing = Missing::OffPath { program, dir };
            format!("✗ {missing}; add it, in ~/.bashrc or ~/.zshrc: {export}")
        }
        Found::Missing(missing) => format!("✗ {missing}"),
        Found::Elsewhere => "✗ adapters.toml has it run something else".to_owned(),
    }
}

/// The program and the arguments that are paths, as the adapter is started.
fn runs(adapters: &Adapters, name: &str, program: &Path) -> String {
    let script = adapters.get(name).and_then(|spec| spec.args.iter().find(|arg| arg.starts_with('/')).cloned());
    let program = home_relative(program);
    match script {
        Some(script) => format!("{program} {}", home_relative(Path::new(&script))),
        None => program,
    }
}
