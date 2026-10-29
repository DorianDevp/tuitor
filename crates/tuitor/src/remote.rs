//! Command lines from other programs, an editor's key above all: `tuitor send bp toggle
//! src/main.rs:12` runs `/bp toggle src/main.rs:12` in the tuitor whose project it was sent
//! from. Each tuitor listens on a socket of its own, `<pid>.sock` in tuitor's runtime
//! directory; a request is a line of JSON, and so is its answer.

use std::io::{BufRead, BufReader, ErrorKind, Write as _};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::{mpsc, oneshot};

/// How long `send` waits for a tuitor to answer: a command runs between two frames.
const ANSWER_WAIT: Duration = Duration::from_secs(10);

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    /// The directory the sender ran in, which tells the project.
    pub cwd: PathBuf,
    /// The command line, as typed after `/`.
    pub line: String,
    /// Run it in the active tab when no tab's project holds `cwd`.
    pub any: bool,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    /// It ran, with what the status line says.
    Done(String),
    /// It was refused, with why.
    Refused(String),
    /// No tab's project holds the sender's directory.
    Elsewhere,
}

/// A request, and where its answer goes.
pub struct Asked {
    pub request: Request,
    pub answer: oneshot::Sender<Reply>,
}

/// The directory of the sockets: `$XDG_RUNTIME_DIR/tuitor`, else one of the user's own in the
/// temporary directory.
fn dir() -> PathBuf {
    dirs::runtime_dir().map(|dir| dir.join("tuitor")).unwrap_or_else(|| {
        let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
        std::env::temp_dir().join(format!("tuitor-{user}"))
    })
}

/// The socket this tuitor listens on, removed when dropped.
pub struct Listener {
    path: PathBuf,
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Listens for requests, which come out of the receiver until the runtime ends.
pub fn listen() -> Result<(Listener, mpsc::UnboundedReceiver<Asked>)> {
    let dir = dir();
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .with_context(|| format!("{}", dir.display()))?;
    let path = dir.join(format!("{}.sock", std::process::id()));
    // A socket's path has to fit in `sockaddr_un`: 108 bytes on Linux, 104 on macOS.
    if path.as_os_str().len() >= 104 {
        bail!("{} is too long for a socket's path; set $XDG_RUNTIME_DIR to a shorter directory", path.display());
    }
    // A tuitor that had this pid before and ended without removing it.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).with_context(|| format!("listening on {}", path.display()))?;
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(serve(stream, tx.clone()));
        }
    });
    Ok((Listener { path }, rx))
}

/// Answers the one request a connection makes.
async fn serve(stream: tokio::net::UnixStream, tx: mpsc::UnboundedSender<Asked>) {
    let (read, mut write) = stream.into_split();
    let Ok(Some(line)) = tokio::io::BufReader::new(read).lines().next_line().await else { return };
    let reply = match serde_json::from_str::<Request>(&line) {
        Ok(request) => {
            let (answer, answered) = oneshot::channel();
            if tx.send(Asked { request, answer }).is_err() {
                return;
            }
            answered.await.unwrap_or_else(|_| Reply::Refused("tuitor is ending".into()))
        }
        Err(err) => Reply::Refused(format!("not a request: {err}")),
    };
    let Ok(mut text) = serde_json::to_string(&reply) else { return };
    text.push('\n');
    let _ = write.write_all(text.as_bytes()).await;
}

/// `tuitor send <command line>`: runs it in the tuitor whose project holds the current
/// directory, else in the one started last; `$TUITOR_SOCKET` names a tuitor's socket instead.
/// Prints what tuitor said, and exits with 1 when it refused or none is running.
pub fn send(words: &[String]) -> Result<ExitCode> {
    let cwd = std::env::current_dir().context("the current directory")?;
    let cwd = std::fs::canonicalize(&cwd).unwrap_or(cwd);
    let line = words.iter().map(|word| absolute(word, &cwd)).collect::<Vec<_>>().join(" ");
    let line = line.as_str();
    let request = |any| Request { cwd: cwd.clone(), line: line.to_owned(), any };
    if let Some(socket) = std::env::var_os("TUITOR_SOCKET").filter(|s| !s.is_empty()) {
        let socket = Path::new(&socket);
        let reply = ask(socket, &request(true)).with_context(|| format!("$TUITOR_SOCKET {}", socket.display()))?;
        return Ok(finish(reply));
    }
    let dir = dir();
    let mut live = Vec::new();
    for socket in sockets(&dir) {
        match ask(&socket, &request(false)) {
            Ok(Reply::Elsewhere) => live.push(socket),
            Ok(reply) => return Ok(finish(reply)),
            // Left by a tuitor that ended without removing it.
            Err(err) if is_gone(&err) => {
                let _ = std::fs::remove_file(&socket);
            }
            Err(err) => eprintln!("tuitor: {}: {err:#}", socket.display()),
        }
    }
    let Some(newest) = live.first() else { bail!("no tuitor is running (no socket in {})", dir.display()) };
    Ok(finish(ask(newest, &request(true))?))
}

/// `word`, or, when it is the `file:line` of a file relative to `cwd`, that place with the
/// file's whole path: an editor names files from its own directory, not tuitor's project.
fn absolute(word: &str, cwd: &Path) -> String {
    let Some((file, line)) = word.rsplit_once(':') else { return word.to_owned() };
    let path = cwd.join(file);
    if file.is_empty() || Path::new(file).is_absolute() || line.parse::<u32>().is_err() || !path.is_file() {
        return word.to_owned();
    }
    format!("{}:{line}", path.display())
}

/// The sockets in `dir`, the newest first.
fn sockets(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut sockets: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sock"))
        .map(|path| (std::fs::metadata(&path).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), path))
        .collect();
    sockets.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    sockets.into_iter().map(|(_, path)| path).collect()
}

fn is_gone(err: &anyhow::Error) -> bool {
    err.downcast_ref::<std::io::Error>()
        .is_some_and(|err| matches!(err.kind(), ErrorKind::ConnectionRefused | ErrorKind::NotFound))
}

fn ask(socket: &Path, request: &Request) -> Result<Reply> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(ANSWER_WAIT))?;
    let mut text = serde_json::to_string(request)?;
    text.push('\n');
    stream.write_all(text.as_bytes())?;
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer).context("waiting for tuitor's answer")?;
    serde_json::from_str(&answer).with_context(|| format!("tuitor answered `{}`", answer.trim()))
}

fn finish(reply: Reply) -> ExitCode {
    match reply {
        Reply::Done(message) => {
            if !message.is_empty() {
                println!("{message}");
            }
            ExitCode::SUCCESS
        }
        Reply::Refused(message) => {
            eprintln!("tuitor: {message}");
            ExitCode::FAILURE
        }
        Reply::Elsewhere => {
            eprintln!("tuitor: no tab has this project");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_place_relative_to_the_sender_gets_its_whole_path() {
        let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
        let whole = format!("{}/src/main.rs:12", cwd.display());
        assert_eq!(absolute("src/main.rs:12", cwd), whole);
        assert_eq!(absolute(&whole, cwd), whole);
        for word in ["toggle", "src/nothing.rs:12", "src/main.rs:x", ":12", "12"] {
            assert_eq!(absolute(word, cwd), word);
        }
    }

    #[tokio::test]
    async fn a_request_is_answered_on_its_connection() {
        let dir = std::env::temp_dir().join(format!("tuitor-remote-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind");
        let (tx, mut rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve(stream, tx.clone()));
            }
        });
        tokio::spawn(async move {
            while let Some(asked) = rx.recv().await {
                let reply = match asked.request.any {
                    false => Reply::Elsewhere,
                    true => Reply::Done(format!("ran {}", asked.request.line)),
                };
                let _ = asked.answer.send(reply);
            }
        });
        let client = path.clone();
        let replies = tokio::task::spawn_blocking(move || {
            let request = |any| Request { cwd: "/w".into(), line: "bp toggle a.rs:3".into(), any };
            (ask(&client, &request(false)).expect("ask"), ask(&client, &request(true)).expect("ask"))
        })
        .await
        .expect("client");
        assert_eq!(replies, (Reply::Elsewhere, Reply::Done("ran bp toggle a.rs:3".into())));
        assert_eq!(sockets(&dir), std::slice::from_ref(&path));
        std::fs::remove_dir_all(&dir).expect("clean up");
        let gone = ask(&path, &Request { cwd: "/w".into(), line: String::new(), any: true }).expect_err("gone");
        assert!(is_gone(&gone), "{gone:#}");
    }
}
