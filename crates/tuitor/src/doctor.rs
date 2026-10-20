//! The report of [`tuitor_core::doctor`]: printed by `tuitor doctor`, and shown by DOCTOR
//! (`/doctor`), where the checks run on a thread of their own, since gdb and PHP take a
//! moment to answer.

use std::fmt::Write as _;
use std::sync::mpsc::{Receiver, TryRecvError};

use tuitor_core::config::Adapters;
use tuitor_core::doctor::{self, Finding, Fix, Options, ProjectInput, Report, Section, Status};

pub fn glyph(status: Status) -> &'static str {
    match status {
        Status::Ok => "✓",
        Status::Note => "!",
        Status::Problem => "✗",
    }
}

/// How a fix reads: a tuitor command is typed after `/` in tuitor.
pub fn fix_text(fix: &Fix, in_tuitor: bool) -> String {
    match fix {
        Fix::Shell(command) | Fix::Text(command) => command.clone(),
        Fix::Command(line) if in_tuitor => format!("/{line}"),
        Fix::Command(line) => format!("in tuitor's CONFIG: /{line}"),
        Fix::Install(package) if in_tuitor => format!("/install {package}"),
        Fix::Install(package) => format!("tuitor install {package}"),
    }
}

/// The report as `tuitor doctor` prints it: a line per finding, under its section, and what
/// fixes it below.
pub fn print(report: &Report) -> String {
    let width = report.sections.iter().flat_map(|s| &s.findings).map(|f| f.subject.chars().count()).max();
    let width = width.unwrap_or(0).min(28);
    let mut out = String::new();
    for section in &report.sections {
        let _ = writeln!(out, "{}", section.title);
        let mut last = None;
        for finding in &section.findings {
            // A subject's findings after the first leave it out.
            let subject = if last == Some(&finding.subject) { "" } else { finding.subject.as_str() };
            last = Some(&finding.subject);
            let _ = writeln!(out, "  {} {subject:<width$}  {}", glyph(finding.status), finding.text);
            if let Some(fix) = &finding.fix {
                let _ = writeln!(out, "    {:<width$}  → {}", "", fix_text(fix, false));
            }
        }
        out.push('\n');
    }
    if report.sections.iter().any(|s| s.needed) {
        match report.blocking() {
            0 => out.push_str("Nothing stands in the way.\n"),
            1 => out.push_str("1 problem stands in the way.\n"),
            n => {
                let _ = writeln!(out, "{n} problems stand in the way.");
            }
        }
    }
    out
}

/// DOCTOR: the report once it is there, and the finding under the cursor.
pub struct Doctor {
    pub report: Option<Report>,
    pending: Option<Receiver<Report>>,
    /// Index among all findings, in order.
    pub selected: usize,
}

impl Doctor {
    /// Starts the checks.
    pub fn start(adapters: Adapters, projects: Vec<ProjectInput>, options: Options) -> Doctor {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(doctor::check(&adapters, &projects, &options));
        });
        Doctor { report: None, pending: Some(rx), selected: 0 }
    }

    /// Takes the report once the checks are done.
    pub fn poll(&mut self) {
        let Some(rx) = &self.pending else { return };
        match rx.try_recv() {
            Ok(report) => {
                // What to look at first: the first problem in the way, else the first at all.
                let findings = report.sections.iter().flat_map(|s| s.findings.iter().map(move |f| (s.needed, f)));
                let problem =
                    |needed: bool| findings.clone().position(|(n, f)| f.status == Status::Problem && (n || !needed));
                self.selected = problem(true).or_else(|| problem(false)).unwrap_or(0);
                self.report = Some(report);
                self.pending = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.pending = None,
        }
    }

    pub fn select(&mut self, delta: isize) {
        let len = self.report.as_ref().map_or(0, |r| r.sections.iter().map(|s| s.findings.len()).sum());
        if len > 0 {
            self.selected = self.selected.saturating_add_signed(delta).min(len - 1);
        }
    }

    /// The finding under the cursor, and its section.
    pub fn selection(&self) -> Option<(&Section, &Finding)> {
        let report = self.report.as_ref()?;
        let mut all = report.sections.iter().flat_map(|s| s.findings.iter().map(move |f| (s, f)));
        all.nth(self.selected)
    }
}
