//! End-to-end: the engine drives a real adapter (`gdb --interpreter=dap`) through
//! a breakpoint, a step and a delta. Skipped when gdb is not installed.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::{Map, Value};
use tokio::sync::mpsc::UnboundedReceiver;
use tuitor_core::config::{Adapters, LaunchConfig, RequestKind};
use tuitor_core::log::OutputKind;
use tuitor_core::session::{SessionId, SessionState};
use tuitor_core::snapshot::{Change, diff};
use tuitor_core::{Engine, EngineMsg, EngineOptions, Step};

use common::TestDir;

const BREAK_LINE: u32 = 4; // `sum += price;`

fn have_gdb() -> bool {
    Command::new("gdb").arg("--version").output().is_ok_and(|o| o.status.success())
}

fn build_fixture(dir: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sum.rs");
    std::fs::copy(&src, dir.join("sum.rs")).expect("copy fixture");
    let bin = dir.join("sum");
    let status = Command::new("rustc")
        .args(["-g", "-C", "opt-level=0", "-o"])
        .arg(&bin)
        .arg(dir.join("sum.rs"))
        .status()
        .expect("run rustc");
    assert!(status.success(), "rustc failed");
    bin
}

/// Feeds engine messages until `done` holds or the timeout expires.
async fn pump(engine: &mut Engine, rx: &mut UnboundedReceiver<EngineMsg>, mut done: impl FnMut(&Engine) -> bool) {
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        while !done(engine) {
            let msg = rx.recv().await.expect("engine channel open");
            engine.handle(msg);
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
async fn breakpoint_step_and_delta_with_gdb() {
    if !have_gdb() {
        eprintln!("gdb not installed; skipping");
        return;
    }
    let tmp = TestDir::new("gdb");
    let dir = tmp.path().to_path_buf();
    let bin = build_fixture(&dir);
    let source = std::fs::canonicalize(dir.join("sum.rs")).expect("source path");

    let (mut engine, mut rx) =
        Engine::new(EngineOptions { adapters: Adapters::default(), workspace: dir.clone(), state_file: None });
    engine.toggle_breakpoint(&source, BREAK_LINE);
    engine.add_watch("sum");

    let mut args = Map::new();
    args.insert("program".into(), Value::String(bin.display().to_string()));
    args.insert("cwd".into(), Value::String(dir.display().to_string()));
    let sid = engine.start(LaunchConfig::adhoc("sum", "gdb", RequestKind::Launch, args)).expect("start session");
    // gdb writes its own text from another thread, which can lag behind its responses. Waiting
    // before the breakpoint's answer is handled (and `configurationDone` sent) keeps the test
    // independent of gdb's thread scheduling.
    pump(&mut engine, &mut rx, |e| e.session(sid).is_some_and(|s| s.state == SessionState::Running)).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // First hit: price = 10, sum = 0.
    pump(&mut engine, &mut rx, |e| stops(e, sid) >= 1).await;
    let session = engine.session(sid).expect("session");
    let first = session.latest().expect("snapshot");
    let top = first.top().expect("frame");
    assert_eq!(top.line, BREAK_LINE);
    assert_eq!(top.path.as_deref(), Some(source.as_path()));
    assert!(top.name.contains("total"), "top frame was {}", top.name);
    assert_eq!(first.hit_breakpoints, [1]);
    assert_eq!(first.local_value("sum"), Some("0"));
    // Children of collapsed variables are read ahead, so that a change inside can be marked.
    let prices = first.locals().iter().find(|v| v.name == "prices");
    assert!(prices.is_some_and(|v| v.children.is_some()), "`prices` was not read ahead: {prices:?}");
    assert!(matches!(first.watch("sum"), Some(Ok(v)) if v == "0"));
    assert_eq!(engine.breakpoints().get(1).map(|b| b.hits), Some(1));

    // The caller's variables are read once its frame is picked, not at every stop.
    assert!(first.scopes(1).is_none(), "the caller was read with the stop");
    engine.read_frame(sid, 1, 1);
    let read = |e: &Engine| e.session(sid).and_then(|s| s.latest()).is_some_and(|s| s.scopes(1).is_some());
    pump(&mut engine, &mut rx, read).await;
    let first = engine.session(sid).and_then(|s| s.latest()).expect("snapshot");
    let caller = first.scopes(1).unwrap_or_default().iter().filter_map(|s| s.vars.as_deref()).flatten();
    let names: Vec<&str> = caller.map(|v| v.name.as_str()).collect();
    assert!(first.frames[1].name.contains("main"), "frame #1 was {}", first.frames[1].name);
    assert!(names.contains(&"prices") && !names.contains(&"sum"), "main's variables were {names:?}");

    // Continue to the second hit: sum changed from 0 to 10.
    engine.resume(sid, Step::Continue).expect("continue");
    pump(&mut engine, &mut rx, |e| stops(e, sid) >= 2).await;
    let session = engine.session(sid).expect("session");
    let (prev, cur) = (&session.history[0], &session.history[1]);
    assert!(!prev.live && cur.live);
    let changes = diff(Some(prev), cur, 0);
    let scopes = cur.scopes(0).unwrap_or_default();
    let locals = scopes.iter().find(|s| s.vars.iter().flatten().any(|v| v.name == "sum")).expect("scope with sum");
    let sum_path = vec![locals.name.clone(), "sum".to_owned()];
    assert_eq!(changes.get(&sum_path), Some(&Change::Changed { was: "0".into() }));

    // Step over stays in the same function and records another stop.
    engine.resume(sid, Step::Over).expect("next");
    pump(&mut engine, &mut rx, |e| stops(e, sid) >= 3).await;

    // Removing the breakpoint lets the program finish.
    engine.toggle_breakpoint(&source, BREAK_LINE);
    engine.resume(sid, Step::Continue).expect("continue");
    pump(&mut engine, &mut rx, |e| e.session(sid).is_some_and(|s| s.state == SessionState::Exited)).await;
    // gdb ≥ 15 loads the program only at `configurationDone` and reports the breakpoint set
    // before that as pending, in output with category stdout. That text is gdb's, not the program's.
    let kind = |text: &str| engine.output().iter().find(|l| l.text.contains(text)).map(|l| l.kind);
    assert_eq!(kind("total=60"), Some(OutputKind::Stdout), "program output missing");
    assert!(matches!(kind("pending"), None | Some(OutputKind::Console)), "gdb's own output shown as the program's");
}

/// Breakpoints of a set other than the active one record the stop and let the program go
/// on; switching the active set while stopped makes the other set's next hit stop.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn breakpoints_of_another_set_record_and_go_on_with_gdb() {
    if !have_gdb() {
        eprintln!("gdb not installed; skipping");
        return;
    }
    let tmp = TestDir::new("gdb-sets");
    let dir = tmp.path().to_path_buf();
    let bin = build_fixture(&dir);
    let source = std::fs::canonicalize(dir.join("sum.rs")).expect("source path");

    let (mut engine, mut rx) =
        Engine::new(EngineOptions { adapters: Adapters::default(), workspace: dir.clone(), state_file: None });
    let rose = engine.breakpoints().active_set();
    engine.toggle_breakpoint(&source, 11); // `let result = total(&prices);`
    let teal = engine.new_set().expect("set");
    engine.toggle_breakpoint(&source, BREAK_LINE);
    engine.activate_set(rose).expect("activate");

    let mut args = Map::new();
    args.insert("program".into(), Value::String(bin.display().to_string()));
    args.insert("cwd".into(), Value::String(dir.display().to_string()));
    let sid = engine.start(LaunchConfig::adhoc("sum", "gdb", RequestKind::Launch, args)).expect("start session");
    pump(&mut engine, &mut rx, |e| e.session(sid).is_some_and(|s| s.state == SessionState::Running)).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // The active set's breakpoint stops.
    pump(&mut engine, &mut rx, |e| stops(e, sid) >= 1).await;
    let first = engine.session(sid).and_then(|s| s.latest()).expect("snapshot");
    assert_eq!((first.top().map(|f| f.line), first.set, first.silent), (Some(11), Some(rose), false));

    // The other set's breakpoint stops once it is active...
    engine.activate_set(teal).expect("activate");
    engine.resume(sid, Step::Continue).expect("continue");
    pump(&mut engine, &mut rx, |e| e.session(sid).is_some_and(|s| s.is_stopped()) && stops(e, sid) >= 2).await;
    let second = engine.session(sid).and_then(|s| s.latest()).expect("snapshot");
    assert_eq!((second.top().map(|f| f.line), second.set, second.silent), (Some(BREAK_LINE), Some(teal), false));

    // ...and records its next hits without stopping once it is not: the program runs to its end.
    engine.activate_set(rose).expect("activate");
    engine.resume(sid, Step::Continue).expect("continue");
    pump(&mut engine, &mut rx, |e| e.session(sid).is_some_and(|s| s.state == SessionState::Exited)).await;
    let session = engine.session(sid).expect("session");
    let passed: Vec<(bool, Option<&str>)> =
        session.history[2..].iter().map(|s| (s.silent, s.local_value("sum"))).collect();
    assert_eq!(passed, [(true, Some("10")), (true, Some("30"))]);
    assert!(session.history[2..].iter().all(|s| s.set == Some(teal) && !s.live && s.watches.is_empty()));
    assert_eq!(session.latest_stop().map(|s| s.number), Some(2));
    assert_eq!(engine.breakpoints().get(2).map(|b| b.hits), Some(3));
    assert!(engine.output().iter().any(|l| l.text.contains("total=60")), "the program did not run to its end");
}
