//! A language server tuitor runs: its process, the connection, and what it says it is busy
//! with, so that questions wait until it has read the project.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, watch};

use crate::config::{self, Language};
use crate::lsp;
use crate::rpc::{Incoming, Rpc};

/// How long a server gets to answer `initialize`: some read the project first.
const INITIALIZE: Duration = Duration::from_secs(120);
/// How long a question gets.
const ANSWER: Duration = Duration::from_secs(30);
/// How long a server must have said nothing new about its work before it counts as done,
/// once it has finished some: work can come in parts.
const QUIET: Duration = Duration::from_millis(500);
/// How long a server gets to report the work it does on starting (reading the project, or
/// intelephense first checking its licence) before it is taken to report none.
const FIRST_REPORT: Duration = Duration::from_secs(10);
/// How long a server gets to end when asked to.
const GOODBYE: Duration = Duration::from_secs(1);
/// Lines of its error output kept, to tell why it failed.
const STDERR_LINES: usize = 6;

pub struct Server {
    /// The program, without its directory.
    pub name: String,
    pub call_hierarchy: bool,
    rpc: Rpc,
    child: Mutex<Option<Child>>,
    status: watch::Receiver<Status>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    /// What each document was last sent as: its version and a hash of its text.
    documents: Mutex<HashMap<PathBuf, (i64, u64)>>,
}

/// What the server says it is doing.
#[derive(Debug, Clone)]
struct Status {
    /// Work begun and not ended, by its progress token: its title, and what it said last.
    busy: BTreeMap<String, (String, String)>,
    /// rust-analyzer's own report: false while it loads and indexes the project.
    quiescent: Option<bool>,
    /// It finished some work it reported.
    reported: bool,
    started: Instant,
    changed: Instant,
    /// It is gone.
    exited: bool,
}

impl Server {
    /// Runs the server for the project in `root` and initializes it.
    pub async fn start(language: Language, command: &config::Command, root: &Path) -> Result<Server> {
        let name = Path::new(&command.program)
            .file_name()
            .map_or_else(|| command.program.clone(), |n| n.to_string_lossy().into_owned());
        let mut child = spawn(language, command, root)?;
        let (Some(stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            bail!("{name} has no standard input and output to talk over");
        };
        let (rpc, incoming) = Rpc::start(stdout, stdin);
        let said = Arc::new(Mutex::new(VecDeque::new()));
        tokio::spawn(keep_stderr(stderr, said.clone(), name.clone()));
        let (status, watched) = watch::channel(Status::new());
        let folders = json!([{ "uri": lsp::uri(root), "name": root.file_name().map(|n| n.to_string_lossy()) }]);
        tokio::spawn(answer(incoming, rpc.clone(), status, folders.clone()));

        let params = json!({
            "processId": std::process::id(),
            "clientInfo": { "name": "tuitor", "version": env!("CARGO_PKG_VERSION") },
            "rootUri": lsp::uri(root),
            "rootPath": root,
            "workspaceFolders": folders,
            "capabilities": capabilities(),
            "initializationOptions": command.initialization_options,
        });
        let initialized = match tokio::time::timeout(INITIALIZE, rpc.request("initialize", params)).await {
            Ok(Ok(result)) => result,
            Ok(Err(err)) => bail!("{name}: {err}{}", tail(&said)),
            Err(_) => bail!("{name} did not answer `initialize` in {} s{}", INITIALIZE.as_secs(), tail(&said)),
        };
        rpc.notify("initialized", json!({}))?;
        let call_hierarchy = initialized["capabilities"]["callHierarchyProvider"]
            .as_bool()
            .unwrap_or_else(|| initialized["capabilities"]["callHierarchyProvider"].is_object());
        Ok(Server {
            name,
            call_hierarchy,
            rpc,
            child: Mutex::new(Some(child)),
            status: watched,
            stderr: said,
            documents: Mutex::default(),
        })
    }

    pub fn alive(&self) -> bool {
        !self.status.borrow().exited
    }

    /// Waits until the server has done the work it reported, telling `said` what that is.
    pub async fn ready(&self, mut said: impl FnMut(String)) -> Result<()> {
        let mut status = self.status.clone();
        loop {
            let wait = {
                let now = status.borrow_and_update();
                if now.exited {
                    bail!("{} exited{}", self.name, tail(&self.stderr));
                }
                if let Some(doing) = now.doing() {
                    said(doing);
                }
                now.wait()
            };
            let changed = match wait {
                Some(Duration::ZERO) => return Ok(()),
                Some(rest) => tokio::select! {
                    changed = status.changed() => changed,
                    () = tokio::time::sleep(rest) => Ok(()),
                },
                None => status.changed().await,
            };
            if changed.is_err() {
                bail!("{} stopped answering{}", self.name, tail(&self.stderr));
            }
        }
    }

    /// Asks the server something, and waits a while for the answer.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        match tokio::time::timeout(ANSWER, self.rpc.request(method, params)).await {
            Ok(answer) => answer,
            Err(_) => bail!("{} did not answer {method} in {} s", self.name, ANSWER.as_secs()),
        }
    }

    /// Gives the server `text` as `path`'s content: opened the first time, changed when it
    /// differs from what it was last sent as. Documents stay open, so that a question given up
    /// on leaves nothing in a state to be cleaned up.
    pub fn sync(&self, path: &Path, text: &str) -> Result<()> {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let hash = hasher.finish();
        let mut documents = self.documents.lock().unwrap_or_else(PoisonError::into_inner);
        let uri = lsp::uri(path);
        match documents.get_mut(path) {
            Some((_, known)) if *known == hash => Ok(()),
            Some((version, known)) => {
                *version += 1;
                *known = hash;
                let document = json!({ "uri": uri, "version": *version });
                self.rpc.notify(
                    "textDocument/didChange",
                    json!({ "textDocument": document, "contentChanges": [{ "text": text }] }),
                )
            }
            None => {
                documents.insert(path.to_path_buf(), (1, hash));
                let language = Language::document(path);
                let document = json!({ "uri": uri, "languageId": language, "version": 1, "text": text });
                self.rpc.notify("textDocument/didOpen", json!({ "textDocument": document }))
            }
        }
    }

    /// Asks the server to end, as LSP says, and makes sure it does.
    pub async fn shutdown(&self) {
        let _ = tokio::time::timeout(GOODBYE, self.rpc.request("shutdown", Value::Null)).await;
        let _ = self.rpc.notify("exit", Value::Null);
        let child = self.child.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(mut child) = child
            && tokio::time::timeout(GOODBYE, child.wait()).await.is_err()
        {
            let _ = child.kill().await;
        }
    }
}

fn spawn(language: Language, command: &config::Command, root: &Path) -> Result<Child> {
    let mut process = Command::new(&command.program);
    process
        .args(&command.args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // A process group of its own, as for adapters: ctrl-c at the terminal reaches tuitor
    // alone, which ends the server in order.
    #[cfg(unix)]
    process.process_group(0);
    process.spawn().map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => anyhow!(
            "{} is not on PATH; `command` in the [lsp.{}] table of config.toml says where it is",
            command.program,
            language.key()
        ),
        _ => anyhow!("cannot run {}: {err}", command.program),
    })
}

/// What tuitor asks of a server, and how it wants answers.
fn capabilities() -> Value {
    json!({
        "general": { "positionEncodings": ["utf-16"] },
        "window": { "workDoneProgress": true },
        "workspace": { "configuration": true, "workspaceFolders": true },
        "textDocument": {
            "synchronization": { "didSave": false },
            "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
            "callHierarchy": {},
            "references": {},
            "documentHighlight": {},
            "definition": { "linkSupport": true },
        },
        // rust-analyzer's report of when it has loaded the project.
        "experimental": { "serverStatusNotification": true },
    })
}

/// Answers what the server asks and keeps what it says it is doing, until it is gone.
async fn answer(
    mut incoming: mpsc::UnboundedReceiver<Incoming>,
    rpc: Rpc,
    status: watch::Sender<Status>,
    folders: Value,
) {
    while let Some(message) = incoming.recv().await {
        match message {
            Incoming::Request { id, method, params } => {
                let reply = match method.as_str() {
                    // Defaults for every setting asked for.
                    "workspace/configuration" => {
                        Ok(Value::Array(vec![Value::Null; params["items"].as_array().map_or(0, Vec::len)]))
                    }
                    "workspace/workspaceFolders" => Ok(folders.clone()),
                    "workspace/applyEdit" => Ok(json!({ "applied": false })),
                    "window/workDoneProgress/create"
                    | "window/showMessageRequest"
                    | "client/registerCapability"
                    | "client/unregisterCapability" => Ok(Value::Null),
                    refresh if refresh.ends_with("/refresh") => Ok(Value::Null),
                    other => Err((-32601, format!("tuitor does not handle {other}"))),
                };
                let _ = rpc.respond(id, reply);
            }
            Incoming::Notification { method, params } => match method.as_str() {
                "$/progress" => status.send_modify(|status| status.progress(&params)),
                "experimental/serverStatus" => status.send_modify(|status| {
                    status.quiescent = params["quiescent"].as_bool();
                    status.reported |= status.quiescent == Some(true);
                    status.changed = Instant::now();
                }),
                "window/showMessage" | "window/logMessage" => {
                    tracing::debug!(target: "lsp", "{}", params["message"].as_str().unwrap_or_default());
                }
                _ => {}
            },
        }
    }
    status.send_modify(|status| status.exited = true);
}

impl Status {
    fn new() -> Status {
        let now = Instant::now();
        Status { busy: BTreeMap::new(), quiescent: None, reported: false, started: now, changed: now, exited: false }
    }

    /// How long to wait before the server may be ready: zero when it is, none while it works.
    fn wait(&self) -> Option<Duration> {
        if self.doing().is_some() {
            return None;
        }
        let quiet = self.changed.elapsed();
        let unreported = FIRST_REPORT.saturating_sub(self.started.elapsed());
        if !self.reported && !unreported.is_zero() {
            return Some(unreported);
        }
        Some(QUIET.saturating_sub(quiet))
    }

    /// Takes in a `$/progress` notification of work done.
    fn progress(&mut self, params: &Value) {
        let token = match &params["token"] {
            Value::String(token) => token.clone(),
            other => other.to_string(),
        };
        let value = &params["value"];
        let text = |title: &str| {
            let message = value["message"].as_str().unwrap_or_default();
            let percent = value["percentage"].as_u64().map(|p| format!(" {p}%")).unwrap_or_default();
            [title, message].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(": ") + &percent
        };
        match value["kind"].as_str() {
            Some("begin") => {
                let title = value["title"].as_str().unwrap_or("working").to_owned();
                let said = text(&title);
                self.busy.insert(token, (title, said));
            }
            Some("report") => {
                if let Some((title, said)) = self.busy.get_mut(&token) {
                    *said = text(title);
                }
            }
            Some("end") => {
                self.busy.remove(&token);
                self.reported = true;
            }
            _ => return,
        }
        self.changed = Instant::now();
    }

    /// What it is busy with, if anything.
    fn doing(&self) -> Option<String> {
        if !self.busy.is_empty() {
            return Some(self.busy.values().map(|(_, said)| said.as_str()).collect::<Vec<_>>().join(" · "));
        }
        (self.quiescent == Some(false)).then(|| "loading the project".to_owned())
    }
}

async fn keep_stderr(stderr: tokio::process::ChildStderr, said: Arc<Mutex<VecDeque<String>>>, name: String) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::debug!(target: "lsp", "{name}: {line}");
        let mut said = said.lock().unwrap_or_else(PoisonError::into_inner);
        if said.len() == STDERR_LINES {
            said.pop_front();
        }
        said.push_back(line);
    }
}

/// The last lines the server wrote to its error output, to add to a message.
fn tail(said: &Mutex<VecDeque<String>>) -> String {
    let said = said.lock().unwrap_or_else(PoisonError::into_inner);
    let lines: Vec<&str> = said.iter().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    if lines.is_empty() { String::new() } else { format!(" ({})", lines.join(" / ")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_work_begun_and_not_ended() {
        let mut s = Status::new();
        // Nothing reported yet: it is given time to start reporting.
        assert!(s.wait().is_some_and(|wait| wait > QUIET));
        s.progress(
            &json!({ "token": "i1", "value": { "kind": "begin", "title": "Intelephense indexing", "percentage": 0 } }),
        );
        s.progress(&json!({ "token": 7, "value": { "kind": "begin", "title": "Setting up workspace" } }));
        s.progress(
            &json!({ "token": "i1", "value": { "kind": "report", "message": "63 of 151 files", "percentage": 41 } }),
        );
        assert_eq!(s.doing().as_deref(), Some("Setting up workspace · Intelephense indexing: 63 of 151 files 41%"));
        s.progress(&json!({ "token": 7, "value": { "kind": "end" } }));
        s.progress(&json!({ "token": "i1", "value": { "kind": "end" } }));
        assert_eq!(s.doing(), None);
        assert!(s.wait().is_some_and(|wait| wait <= QUIET));
        s.changed -= QUIET;
        assert_eq!(s.wait(), Some(Duration::ZERO));
        s.quiescent = Some(false);
        assert_eq!(s.wait(), None);
        assert_eq!(s.doing().as_deref(), Some("loading the project"));
    }

    #[tokio::test]
    async fn a_missing_server_says_where_to_name_it() {
        let command = config::Command {
            program: "tuitor-no-such-server".into(),
            args: Vec::new(),
            initialization_options: Value::Null,
        };
        let err = Server::start(Language::Go, &command, Path::new("/")).await.err().expect("not started");
        assert_eq!(
            err.to_string(),
            "tuitor-no-such-server is not on PATH; `command` in the [lsp.go] table of config.toml says where it is"
        );
    }
}
