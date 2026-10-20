//! Code integration through language servers. From a line of code, the call paths that lead
//! to it, from where execution starts, and on each the lines worth a breakpoint.
//!
//! The rest of tuitor speaks DAP; this crate alone speaks LSP, to the servers of
//! [`config::Servers`], started the first time a project asks and kept for its next questions.
//! It knows nothing about tuitor.

pub mod config;
mod lsp;
mod paths;
mod rpc;
mod select;
mod server;
mod text;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;
use tokio::sync::{OnceCell, mpsc};
use tokio::task::JoinHandle;

pub use config::{Language, Servers};
pub use paths::{CallPath, Entry, Found, Observed, Root, Step, Stop};
pub use select::Why;

use crate::paths::{Ask, Limits, Question};
use crate::server::Server;
use crate::text::Text;

/// One project's language servers, a server per language, started when first needed.
pub struct Code {
    root: PathBuf,
    servers: Servers,
    running: HashMap<Language, Slot>,
}

/// A server, once started; queries that need it at once wait for the one that starts it.
type Slot = Arc<OnceCell<Arc<Server>>>;

/// What a query says as it goes, and its answer.
#[derive(Debug)]
pub enum Update {
    Progress(String),
    Done(Result<Found, String>),
}

/// A query running; dropped, it stops, while its server goes on.
pub struct Query {
    updates: mpsc::UnboundedReceiver<Update>,
    task: JoinHandle<()>,
}

impl Query {
    /// What it said since last asked, oldest first.
    pub fn updates(&mut self) -> Vec<Update> {
        let mut updates = Vec::new();
        while let Ok(update) = self.updates.try_recv() {
            updates.push(update);
        }
        updates
    }
}

impl Drop for Query {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Code {
    /// The servers of the project in `root`; none runs yet.
    pub fn new(root: PathBuf, servers: Servers) -> Code {
        Code { root, servers, running: HashMap::new() }
    }

    /// Starts looking for the paths to `line` (1-based) of `file`, with the calls earlier
    /// runs showed. Fails at once for a file no server tuitor knows reads.
    pub fn paths(&mut self, file: PathBuf, line: u32, observed: Vec<Observed>) -> Result<Query, String> {
        let language = Language::of(&file).ok_or_else(|| {
            let name = file.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            format!("tuitor knows no language server for {name}; it asks those of PHP, TypeScript, Rust and Go")
        })?;
        // A server that exited is started again.
        let slot = self.running.entry(language).or_default();
        if slot.get().is_some_and(|server| !server.alive()) {
            *slot = Slot::default();
        }
        let slot = slot.clone();
        let command = self.servers.command(language);
        let root = self.root.clone();
        let (tx, updates) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let said = tx.clone();
            let progress = move |text: String| drop(said.send(Update::Progress(text)));
            let question = Question { root: &root, file: &file, line, observed: &observed, limits: Limits::default() };
            let found = query(slot, language, command, question, progress).await;
            let _ = tx.send(Update::Done(found.map_err(|err| format!("{err:#}"))));
        });
        Ok(Query { updates, task })
    }

    /// Ends the servers that run, each as LSP says.
    pub async fn shutdown(&mut self) {
        let running: Vec<Arc<Server>> = self.running.drain().filter_map(|(_, slot)| slot.get().cloned()).collect();
        futures::future::join_all(running.iter().map(|server| server.shutdown())).await;
    }
}

async fn query(
    slot: Slot,
    language: Language,
    command: config::Command,
    question: Question<'_>,
    mut progress: impl FnMut(String) + Send,
) -> Result<Found> {
    let server = slot
        .get_or_try_init(|| {
            let program = Path::new(&command.program).file_name().map(|name| name.to_string_lossy());
            progress(format!("starting {}…", program.unwrap_or_default()));
            async { Server::start(language, &command, question.root).await.map(Arc::new) }
        })
        .await?
        .clone();
    let name = server.name.clone();
    server.sync(question.file, &Text::read(question.file)?.source)?;
    server.ready(|doing| progress(format!("{name}: {doing}"))).await?;
    progress(format!("asking {name}…"));
    let mut asking = Asking { server: server.clone() };
    let mut found =
        paths::find(&mut asking, language, question, |doing| progress(format!("asking {name}: {doing}"))).await?;
    if !server.call_hierarchy {
        found.notes.insert(0, format!("{name} has no call hierarchy: callers are the references that call"));
    }
    Ok(found)
}

/// A server, asked by the path finder.
struct Asking {
    server: Arc<Server>,
}

impl Ask for Asking {
    fn call_hierarchy(&self) -> bool {
        self.server.call_hierarchy
    }

    fn open(&mut self, path: &Path, text: &Text) -> Result<()> {
        self.server.sync(path, &text.source)
    }

    async fn ask(&mut self, method: &'static str, params: Value) -> Result<Value> {
        self.server.request(method, params).await
    }
}
