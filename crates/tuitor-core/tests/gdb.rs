//! End-to-end: the engine drives a real adapter (`gdb --interpreter=dap`) through
//! a breakpoint, a step and a delta. Skipped when gdb is not installed.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::{Map, Value};
use tokio::sync::mpsc::UnboundedReceiver;
use tuitor_core::config::{Adapters, LaunchConfig, RequestKind};
use tuitor_core::session::{SessionId, SessionState};
use tuitor_core::snapshot::{Change, diff};
use tuitor_core::{Engine, EngineMsg, EngineOptions, Step};

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
    let dir = std::env::temp_dir().join(format!("tuitor-gdb-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
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
    assert!(matches!(first.watch("sum"), Some(Ok(v)) if v == "0"));
    assert_eq!(engine.breakpoints().get(1).map(|b| b.hits), Some(1));

    // Continue to the second hit: sum changed from 0 to 10.
    engine.resume(sid, Step::Continue).expect("continue");
    pump(&mut engine, &mut rx, |e| stops(e, sid) >= 2).await;
    let session = engine.session(sid).expect("session");
    let (prev, cur) = (&session.history[0], &session.history[1]);
    assert!(!prev.live && cur.live);
    let changes = diff(Some(prev), cur);
    let locals = cur.scopes.iter().find(|s| s.vars.iter().flatten().any(|v| v.name == "sum")).expect("scope with sum");
    let sum_path = vec![locals.name.clone(), "sum".to_owned()];
    assert_eq!(changes.get(&sum_path), Some(&Change::Changed { was: "0".into() }));

    // Step over stays in the same function and records another stop.
    engine.resume(sid, Step::Over).expect("next");
    pump(&mut engine, &mut rx, |e| stops(e, sid) >= 3).await;

    // Removing the breakpoint lets the program finish.
    engine.toggle_breakpoint(&source, BREAK_LINE);
    engine.resume(sid, Step::Continue).expect("continue");
    pump(&mut engine, &mut rx, |e| e.session(sid).is_some_and(|s| s.state == SessionState::Exited)).await;
    assert!(engine.output().iter().any(|l| l.text.contains("total=60")), "program output missing");

    std::fs::remove_dir_all(dir).ok();
}
