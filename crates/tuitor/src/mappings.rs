//! `/config mappings-from-compose` on a thread of its own: docker compose takes a moment to
//! answer, and telling whether a directory holds code takes reading it.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use tuitor_core::compose::{self, Compose, Mapped, Reader, Service};
use tuitor_core::config::{AdapterSpec, home_relative};

use crate::app::Refusal;
use crate::tab::TabId;

/// What to read, and the configuration of which tab the mappings are for.
#[derive(Debug, Clone)]
pub struct Job {
    pub tab: TabId,
    /// The project's directory.
    pub dir: PathBuf,
    pub config: String,
    pub file: PathBuf,
    pub service: Option<String>,
    /// The configuration is for PHP, whose service is the one to map unless named.
    pub php: bool,
    /// Its adapter, which tells code files by their extension.
    pub adapter: Option<AdapterSpec>,
}

/// The mappings found, and what to tell of them.
pub struct Found {
    pub summary: String,
    pub mappings: Vec<(String, PathBuf)>,
}

pub struct Running {
    job: Job,
    rx: Receiver<Result<Found, Refusal>>,
}

impl Job {
    pub fn file_name(&self) -> String {
        self.file.file_name().map_or_else(|| self.file.display().to_string(), |n| n.to_string_lossy().into_owned())
    }

    pub fn start(self) -> Running {
        let (tx, rx) = std::sync::mpsc::channel();
        let job = self.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job.find());
        });
        Running { job: self, rx }
    }

    fn find(&self) -> Result<Found, Refusal> {
        let compose = compose::load(&self.file, true).map_err(|e| format!("{e:#}"))?;
        let service = compose::pick(&compose, self.service.as_deref(), self.php)
            .map_err(|why| Refusal::warn(format!("{why}; /config mappings-from-compose ‹service› names one")))?;
        let is_code = |path: &Path| self.adapter.as_ref().is_some_and(|spec| spec.handles(path));
        let mapped = compose::mappings(service, &is_code);
        if mapped.mappings.is_empty() {
            return Err(format!("{} mounts no directory from here", service.name).into());
        }
        Ok(Found { summary: summary(&self.dir, &compose, service, &mapped), mappings: mapped.mappings })
    }
}

impl Running {
    /// The job and what it found, once it is done.
    pub fn poll(&self) -> Option<(Job, Result<Found, Refusal>)> {
        match self.rx.try_recv() {
            Ok(result) => Some((self.job.clone(), result)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some((self.job.clone(), Err("reading the compose file failed".into()))),
        }
    }

    pub fn file_name(&self) -> String {
        self.job.file_name()
    }
}

/// What `/config mappings-from-compose` mapped, from where, and what it left out.
fn summary(dir: &Path, compose: &Compose, service: &Service, mapped: &Mapped) -> String {
    // Relative to the project, up to a few directories above it.
    let local = |path: &Path| {
        let above =
            dir.ancestors().take(4).enumerate().find_map(|(up, root)| Some((up, path.strip_prefix(root).ok()?)));
        match above {
            Some((0, rest)) if rest.as_os_str().is_empty() => ".".to_owned(),
            Some((up, rest)) => format!("{}{}", "../".repeat(up), rest.display()),
            None => home_relative(path),
        }
    };
    let mappings: Vec<String> = mapped
        .mappings
        .iter()
        .map(|(remote, here)| match mapped.collapsed.iter().find(|(root, _)| root == remote) {
            Some((_, mounts)) => format!("{remote} → {} ({mounts} mounts)", local(here)),
            None => format!("{remote} → {}", local(here)),
        })
        .collect();
    let file = local(&compose.file);
    let reader = match (compose.read_by, &compose.docker_error) {
        (Reader::Docker, _) => "docker compose config".to_owned(),
        (Reader::Tuitor, Some(why)) => format!("read by tuitor, as {why}"),
        (Reader::Tuitor, None) => "read by tuitor".to_owned(),
    };
    let count = mappings.len();
    let s = if count == 1 { "" } else { "s" };
    let mut text =
        format!("{count} mapping{s} from {file}, service {} ({reader}): {}", service.name, mappings.join(" · "));
    if !mapped.skipped.is_empty() {
        text.push_str(&format!("; left out {}", mapped.skipped.join(", ")));
    }
    text
}
