//! End-to-end with vscode-php-debug listening for Xdebug, the way PHP in a
//! container connects back to the host. Needs two environment variables, so it
//! is ignored by default (the adapter path is where README's install steps put it):
//!
//! ```sh
//! TUITOR_PHP=/path/to/php-with-xdebug \
//! TUITOR_PHP_ADAPTER=~/.local/share/tuitor/adapters/vscode-php-debug/extension/out/phpDebug.js \
//! cargo test -p tuitor-core --test php -- --ignored
//! ```

mod common;

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;
use tuitor_core::config::{Adapters, LaunchConfig, RequestKind};
use tuitor_core::session::{SessionId, SessionState};
use tuitor_core::snapshot::{Change, diff};
use tuitor_core::{Engine, EngineMsg, EngineOptions, Step};

use common::TestDir;

const BREAK_LINE: u32 = 12; // `$total += $item['price'] * $item['qty'];`

async fn pump(engine: &mut Engine, rx: &mut UnboundedReceiver<EngineMsg>, mut done: impl FnMut(&Engine) -> bool) {
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        while !done(engine) {
            engine.handle(rx.recv().await.expect("engine channel open"));
            for notice in engine.take_notices() {
                eprintln!("notice: {:?} {}", notice.level, notice.text);
            }
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "timed out; events: {:#?}",
        engine.events().iter().map(|e| (&e.name, &e.detail)).collect::<Vec<_>>()
    );
}

fn stops(engine: &Engine, sid: SessionId) -> usize {
    engine.session(sid).map_or(0, |s| s.history.len())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs TUITOR_PHP and TUITOR_PHP_ADAPTER"]
async fn xdebug_connects_back_and_stops() {
    let php = std::env::var("TUITOR_PHP").expect("TUITOR_PHP");
    let adapter_js = std::env::var("TUITOR_PHP_ADAPTER").expect("TUITOR_PHP_ADAPTER");

    let tmp = TestDir::new("php");
    let dir = tmp.path().to_path_buf();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/orders.php");
    std::fs::copy(&fixture, dir.join("orders.php")).expect("copy");
    let script = std::fs::canonicalize(dir.join("orders.php")).expect("script path");

    let adapters_file = dir.join("adapters.toml");
    std::fs::write(
        &adapters_file,
        format!("[php]\ncommand = \"node\"\nargs = [{adapter_js:?}]\nextensions = [\"php\"]\n"),
    )
    .expect("adapters.toml");
    let adapters = Adapters::load(Some(&adapters_file)).expect("adapters");

    let port = std::net::TcpListener::bind("127.0.0.1:0").expect("port").local_addr().expect("addr").port();
    let (mut engine, mut rx) = Engine::new(EngineOptions { adapters, workspace: dir.clone(), state_file: None });
    engine.toggle_breakpoint(&script, BREAK_LINE);
    engine.add_watch("$total");

    let Value::Object(args) = json!({ "port": port, "log": false }) else { unreachable!() };
    let sid = engine.start(LaunchConfig::adhoc("Listen for Xdebug", "php", RequestKind::Launch, args)).expect("start");

    // The adapter listens once configured; then PHP connects like a container would.
    pump(&mut engine, &mut rx, |e| e.session(sid).is_some_and(|s| s.state == SessionState::Running)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut child = tokio::process::Command::new(&php)
        .args(["-dxdebug.mode=debug", "-dxdebug.start_with_request=yes", "-dxdebug.client_host=127.0.0.1"])
        .arg(format!("-dxdebug.client_port={port}"))
        .arg(&script)
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn php");

    pump(&mut engine, &mut rx, |e| stops(e, sid) >= 1).await;
    let snap = engine.session(sid).and_then(|s| s.latest()).expect("snapshot").clone();
    let top = snap.top().expect("frame");
    assert_eq!(top.line, BREAK_LINE);
    assert_eq!(top.path.as_deref(), Some(script.as_path()));
    assert_eq!(snap.hit_breakpoints, [1]);
    let names: Vec<&str> = snap.locals().iter().map(|v| v.name.as_str()).collect();
    assert!(names.contains(&"$total") && names.contains(&"$item"), "locals were {names:?}");
    assert!(matches!(snap.watch("$total"), Some(Ok(_))), "watch was {:?}", snap.watch("$total"));

    // The script's own variables, in the frame that called `total`, once it is picked.
    engine.read_frame(sid, 1, 1);
    let read = |e: &Engine| e.session(sid).and_then(|s| s.latest()).is_some_and(|s| s.scopes(1).is_some());
    pump(&mut engine, &mut rx, read).await;
    let caller = engine.session(sid).and_then(|s| s.latest()).and_then(|s| s.scopes(1)).unwrap_or_default();
    let names: Vec<&str> = caller.iter().filter_map(|s| s.vars.as_deref()).flatten().map(|v| v.name.as_str()).collect();
    assert!(names.contains(&"$items") && !names.contains(&"$total"), "the caller's variables were {names:?}");

    engine.resume(sid, Step::Continue).expect("continue");
    pump(&mut engine, &mut rx, |e| stops(e, sid) >= 2).await;
    let second = engine.session(sid).and_then(|s| s.latest()).expect("second");
    assert_eq!(second.local_value("$total"), Some("89.99"));
    // `$item` reads `array(4)` at both stops, but its elements were read ahead while it
    // was collapsed, so the next item is still noticed.
    let scopes = second.scopes(0).unwrap_or_default();
    let locals = scopes.iter().find(|s| s.vars.iter().flatten().any(|v| v.name == "$item")).expect("scope");
    let item_path = vec![locals.name.clone(), "$item".to_owned()];
    assert_eq!(diff(Some(&snap), second, 0).get(&item_path), Some(&Change::Inside));

    engine.toggle_breakpoint(&script, BREAK_LINE);
    engine.resume(sid, Step::Continue).expect("continue");
    let output = tokio::time::timeout(Duration::from_secs(15), async {
        // Keep the engine responsive while PHP finishes.
        loop {
            tokio::select! {
                Some(msg) = rx.recv() => engine.handle(msg),
                status = child.wait() => break status,
            }
        }
    })
    .await
    .expect("php finished")
    .expect("php status");
    assert!(output.success());

    engine.stop(sid).expect("stop");
}

/// vscode-php-debug names no breakpoint at a stop: the location tells whether it is one of
/// a set other than the active one, which records and goes on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs TUITOR_PHP and TUITOR_PHP_ADAPTER"]
async fn breakpoints_of_another_set_record_and_go_on() {
    let php = std::env::var("TUITOR_PHP").expect("TUITOR_PHP");
    let adapter_js = std::env::var("TUITOR_PHP_ADAPTER").expect("TUITOR_PHP_ADAPTER");

    let tmp = TestDir::new("php-sets");
    let dir = tmp.path().to_path_buf();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/orders.php");
    std::fs::copy(&fixture, dir.join("orders.php")).expect("copy");
    let script = std::fs::canonicalize(dir.join("orders.php")).expect("script path");
    let adapters_file = dir.join("adapters.toml");
    std::fs::write(
        &adapters_file,
        format!("[php]\ncommand = \"node\"\nargs = [{adapter_js:?}]\nextensions = [\"php\"]\n"),
    )
    .expect("adapters.toml");
    let adapters = Adapters::load(Some(&adapters_file)).expect("adapters");

    let port = std::net::TcpListener::bind("127.0.0.1:0").expect("port").local_addr().expect("addr").port();
    let (mut engine, mut rx) = Engine::new(EngineOptions { adapters, workspace: dir.clone(), state_file: None });
    let rose = engine.breakpoints().active_set();
    engine.toggle_breakpoint(&script, 14); // `return $total;`
    let teal = engine.new_set();
    engine.toggle_breakpoint(&script, BREAK_LINE);
    engine.activate_set(rose).expect("activate");

    let Value::Object(args) = json!({ "port": port, "log": false }) else { unreachable!() };
    let sid = engine.start(LaunchConfig::adhoc("Listen for Xdebug", "php", RequestKind::Launch, args)).expect("start");
    pump(&mut engine, &mut rx, |e| e.session(sid).is_some_and(|s| s.state == SessionState::Running)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut child = tokio::process::Command::new(&php)
        .args(["-dxdebug.mode=debug", "-dxdebug.start_with_request=yes", "-dxdebug.client_host=127.0.0.1"])
        .arg(format!("-dxdebug.client_port={port}"))
        .arg(&script)
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn php");

    // Both items that are not cancelled pass line 12; the program waits at line 14.
    pump(&mut engine, &mut rx, |e| e.session(sid).is_some_and(|s| s.live().is_some())).await;
    let session = engine.session(sid).expect("session");
    let stops: Vec<(u32, Vec<tuitor_core::breakpoints::SetId>, bool)> =
        session.history.iter().map(|s| (s.top().map_or(0, |f| f.line), s.sets.clone(), s.silent)).collect();
    assert_eq!(stops, [(BREAK_LINE, vec![teal], true), (BREAK_LINE, vec![teal], true), (14, vec![rose], false)]);
    assert_eq!(session.history[1].local_value("$total"), Some("89.99"));

    engine.resume(sid, Step::Continue).expect("continue");
    let status = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            tokio::select! {
                Some(msg) = rx.recv() => engine.handle(msg),
                status = child.wait() => break status,
            }
        }
    })
    .await
    .expect("php finished")
    .expect("php status");
    assert!(status.success());
    engine.stop(sid).expect("stop");
}
