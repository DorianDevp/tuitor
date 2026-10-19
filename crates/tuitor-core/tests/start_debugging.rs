//! Child sessions against a fake adapter that behaves like js-debug's server: the session
//! started from a configuration asks for a child with `startDebugging`, and the child
//! connects to the same address. Runs without Node, unlike `tests/js.rs`.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tuitor_core::config::{Adapters, LaunchConfig, RequestKind};
use tuitor_core::session::SessionState;
use tuitor_core::{Engine, EngineMsg, EngineOptions, Level};

/// What the fake adapter received, by connection: 0 is the parent, 1 the child.
type Received = (usize, Value);

async fn read_message(reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.ok()? == 0 {
            return None;
        }
        match line.trim_end().split_once(':') {
            Some((name, value)) if name.eq_ignore_ascii_case("content-length") => length = value.trim().parse().ok(),
            _ if line.trim_end().is_empty() => break,
            _ => {}
        }
    }
    let mut body = vec![0; length?];
    reader.read_exact(&mut body).await.ok()?;
    serde_json::from_slice(&body).ok()
}

async fn write_message(writer: &mut tokio::net::tcp::OwnedWriteHalf, message: Value) {
    let body = message.to_string();
    let frame = format!("Content-Length: {}\r\n\r\n{body}", body.len());
    writer.write_all(frame.as_bytes()).await.expect("write to tuitor");
}

/// Answers every request with success. Once launched, the parent asks for a child, or for
/// a terminal when the configuration wants the program in one, and ends when told to
/// disconnect, like js-debug.
async fn serve(conn: usize, stream: TcpStream, received: UnboundedSender<Received>) {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let mut seq = 1;
    let mut next = || {
        seq += 1;
        seq
    };
    while let Some(message) = read_message(&mut reader).await {
        let _ = received.send((conn, message.clone()));
        if message["type"] != "request" {
            continue;
        }
        let command = message["command"].as_str().unwrap_or_default().to_owned();
        let body = match command.as_str() {
            "initialize" => json!({ "supportsConfigurationDoneRequest": true }),
            "setBreakpoints" => {
                let count = message["arguments"]["breakpoints"].as_array().map_or(0, Vec::len);
                json!({ "breakpoints": vec![json!({ "verified": true }); count] })
            }
            _ => json!({}),
        };
        let response = json!({
            "seq": next(), "type": "response", "request_seq": message["seq"],
            "success": true, "command": command, "body": body,
        });
        write_message(&mut write, response).await;
        match (conn, command.as_str()) {
            (_, "initialize") => {
                write_message(&mut write, json!({ "seq": next(), "type": "event", "event": "initialized" })).await;
            }
            (0, "launch") => {
                let (command, arguments) = if message["arguments"]["console"] == "integratedTerminal" {
                    ("runInTerminal", json!({ "kind": "integrated", "cwd": "/", "args": ["node", "orders.js"] }))
                } else {
                    let configuration =
                        json!({ "type": "pwa-node", "name": "orders.js [42]", "__pendingTargetId": "t1" });
                    ("startDebugging", json!({ "request": "launch", "configuration": configuration }))
                };
                let request = json!({ "seq": next(), "type": "request", "command": command, "arguments": arguments });
                write_message(&mut write, request).await;
            }
            (_, "disconnect") => {
                write_message(&mut write, json!({ "seq": next(), "type": "event", "event": "terminated" })).await;
                return;
            }
            _ => {}
        }
    }
}

async fn fake_adapter() -> (String, UnboundedReceiver<Received>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listen");
    let address = listener.local_addr().expect("address").to_string();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        for conn in 0.. {
            let Ok((stream, _)) = listener.accept().await else { return };
            tokio::spawn(serve(conn, stream, tx.clone()));
        }
    });
    (address, rx)
}

/// Feeds the engine until `done` holds, keeping what the adapter received.
async fn pump(
    engine: &mut Engine,
    rx: &mut UnboundedReceiver<EngineMsg>,
    adapter: &mut UnboundedReceiver<Received>,
    log: &mut Vec<Received>,
    mut done: impl FnMut(&Engine, &[Received]) -> bool,
) {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        while !done(engine, log) {
            tokio::select! {
                Some(msg) = rx.recv() => engine.handle(msg),
                Some(received) = adapter.recv() => log.push(received),
            }
        }
    })
    .await;
    assert!(result.is_ok(), "timed out; the adapter received {log:#?}");
}

fn requests<'a>(log: &'a [Received], conn: usize, command: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
    log.iter().filter(move |(c, m)| *c == conn && m["type"] == "request" && m["command"] == command).map(|(_, m)| m)
}

fn response<'a>(log: &'a [Received], command: &str) -> Option<&'a Value> {
    log.iter().map(|(_, m)| m).find(|m| m["type"] == "response" && m["command"] == command)
}

/// A directory with an empty `orders.js`, and adapters with `fake` for it at `address`.
fn workspace(name: &str, address: &str) -> (PathBuf, PathBuf, Adapters) {
    let dir = std::env::temp_dir().join(format!("tuitor-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let file = dir.join("adapters.toml");
    std::fs::write(&file, format!("[fake]\ntransport = \"tcp\"\naddress = {address:?}\nextensions = [\"js\"]\n"))
        .expect("adapters.toml");
    let adapters = Adapters::load(Some(&file)).expect("adapters");
    let script = dir.join("orders.js");
    std::fs::write(&script, "").expect("script");
    let script = std::fs::canonicalize(script).expect("script path");
    (dir, script, adapters)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn child_sessions_connect_to_the_same_adapter() {
    let (address, mut adapter) = fake_adapter().await;
    let (dir, script, adapters) = workspace("start-debugging", &address);
    let (mut engine, mut rx) = Engine::new(EngineOptions { adapters, workspace: dir.clone(), state_file: None });
    engine.toggle_breakpoint(&script, 3);
    let config = LaunchConfig::adhoc("orders", "fake", RequestKind::Launch, Default::default());
    let parent = engine.start(config).expect("start");
    let mut log = Vec::new();

    // The child sends back the configuration the adapter made up and is configured on its own.
    let child_configured = |log: &[Received]| requests(log, 1, "configurationDone").next().is_some();
    pump(&mut engine, &mut rx, &mut adapter, &mut log, |_, log| child_configured(log)).await;
    let started = response(&log, "startDebugging").expect("startDebugging answered");
    assert_eq!(started["success"], true);
    let launch = requests(&log, 1, "launch").next().expect("child launch");
    assert_eq!(launch["arguments"]["__pendingTargetId"], "t1");
    let child = engine.sessions().iter().find(|s| s.parent == Some(parent)).expect("child session");
    let cid = child.id;
    assert_eq!(child.name, format!("{} › orders.js [42]", engine.session(parent).expect("parent").name));
    let child_breakpoints = requests(&log, 1, "setBreakpoints").next().expect("child breakpoints");
    assert_eq!(child_breakpoints["arguments"]["breakpoints"], json!([{ "line": 3 }]));

    // From now on breakpoints go to the child, where the code runs.
    let parent_sets = requests(&log, 0, "setBreakpoints").count();
    engine.toggle_breakpoint(&script, 7);
    let sent_to_child = |log: &[Received]| requests(log, 1, "setBreakpoints").count() == 2;
    let bound = |e: &Engine| e.breakpoints().get(2).is_some_and(|b| !b.bindings.is_empty());
    pump(&mut engine, &mut rx, &mut adapter, &mut log, |e, log| sent_to_child(log) && bound(e)).await;
    assert_eq!(requests(&log, 0, "setBreakpoints").count(), parent_sets);
    assert_eq!(engine.breakpoints().get(2).map(|b| b.bindings.keys().copied().collect()), Some(vec![cid]));

    // Stopping the child stops its parent, which ends the child too.
    engine.stop(cid).expect("stop");
    let all_ended = |e: &Engine| e.sessions().iter().all(|s| s.state == SessionState::Exited);
    pump(&mut engine, &mut rx, &mut adapter, &mut log, |e, _| all_ended(e)).await;
    assert!(requests(&log, 0, "disconnect").next().is_some(), "the parent was not stopped");
    assert!(requests(&log, 1, "disconnect").next().is_some(), "the child was not ended");

    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_for_a_terminal_ends_the_session_and_says_what_to_change() {
    let (address, mut adapter) = fake_adapter().await;
    let (dir, _, adapters) = workspace("run-in-terminal", &address);
    let (mut engine, mut rx) = Engine::new(EngineOptions { adapters, workspace: dir.clone(), state_file: None });
    let Value::Object(args) = json!({ "console": "integratedTerminal" }) else { unreachable!() };
    let sid = engine.start(LaunchConfig::adhoc("orders", "fake", RequestKind::Launch, args)).expect("start");
    let mut log = Vec::new();

    // The adapter answered `launch` with success, and would wait for the terminal for good.
    let closed = |e: &Engine| e.events().iter().any(|event| event.name == "closed");
    pump(&mut engine, &mut rx, &mut adapter, &mut log, |e, log| response(log, "runInTerminal").is_some() && closed(e))
        .await;
    let refused = response(&log, "runInTerminal").expect("runInTerminal answered");
    assert_eq!(refused["success"], false);
    assert!(refused["message"].as_str().is_some_and(|m| m.contains("internalConsole")), "{refused}");
    assert_eq!(engine.session(sid).map(|s| s.state), Some(SessionState::Exited));

    // What to change is the last thing said, not buried under the connection closing.
    let notices = engine.take_notices();
    let last = notices.last().expect("a notice");
    assert_eq!(last.level, Level::Error);
    assert!(last.text.contains("\"console\": \"internalConsole\""), "{notices:?}");

    std::fs::remove_dir_all(dir).ok();
}
