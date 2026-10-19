//! End-to-end with Delve's DAP server, which builds the Go example and runs it. Needs Go and
//! dlv, so it is ignored by default (the path is where `go install` puts dlv):
//!
//! ```sh
//! TUITOR_DLV="$(go env GOPATH)/bin/dlv" cargo test -p tuitor-core --test go -- --ignored
//! ```

mod common;

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;
use tuitor_core::config::{Adapters, LaunchConfig, RequestKind};
use tuitor_core::log::OutputKind;
use tuitor_core::session::{SessionId, SessionState};
use tuitor_core::snapshot::{Change, diff};
use tuitor_core::{Engine, EngineMsg, EngineOptions, Step};

use common::TestDir;

const BREAK_LINE: u32 = 58; // `total += lineTotal(line)`
const LINE_TOTAL: u32 = 40; // `func lineTotal(line Line) int {`

async fn pump(engine: &mut Engine, rx: &mut UnboundedReceiver<EngineMsg>, mut done: impl FnMut(&Engine) -> bool) {
    // dlv builds the program first, which takes a while with a cold build cache.
    let result = tokio::time::timeout(Duration::from_secs(120), async {
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

fn exited(engine: &Engine, sid: SessionId) -> bool {
    engine.session(sid).is_some_and(|s| s.state == SessionState::Exited)
}

/// A directory with the Go example and an adapters.toml that runs dlv from the environment.
fn workspace(name: &str) -> (TestDir, Adapters) {
    let dlv = std::env::var("TUITOR_DLV").expect("TUITOR_DLV");
    let tmp = TestDir::new(name);
    let dir = tmp.path();
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/go");
    for file in ["go.mod", "main.go"] {
        std::fs::copy(example.join(file), dir.join(file)).expect("copy");
    }

    // The built-in entry with dlv from the environment.
    let builtin = Adapters::default();
    let spec = builtin.get("go").expect("built-in adapter");
    let file = dir.join("adapters.toml");
    let toml = format!(
        "[go]\ntransport = \"server\"\ncommand = {dlv:?}\nargs = {:?}\nextensions = {:?}\n",
        spec.args, spec.extensions
    );
    std::fs::write(&file, toml).expect("adapters.toml");
    (tmp, Adapters::load(Some(&file)).expect("adapters"))
}

fn launch(fields: Value) -> LaunchConfig {
    let Value::Object(args) = fields else { unreachable!() };
    LaunchConfig::adhoc("orders", "go", RequestKind::Launch, args)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs TUITOR_DLV"]
async fn delve_builds_stops_steps_and_cleans_up() {
    let (tmp, adapters) = workspace("go");
    let dir = tmp.path();
    let source = dir.join("main.go");
    let (mut engine, mut rx) = Engine::new(EngineOptions { adapters, workspace: dir.to_path_buf(), state_file: None });
    engine.toggle_breakpoint(&source, BREAK_LINE);
    engine.add_watch("total");

    // As the example's launch.json has it: dlv builds the package in the directory.
    let sid = engine.start(launch(json!({ "mode": "debug", "program": dir.display().to_string() }))).expect("start");
    pump(&mut engine, &mut rx, |e| stops(e, sid) >= 1).await;
    let first = engine.session(sid).and_then(|s| s.latest()).expect("snapshot");
    let top = first.top().expect("frame");
    assert_eq!((top.line, top.path.as_deref()), (BREAK_LINE, Some(source.as_path())));
    assert_eq!(top.name, "main.orderTotal");
    assert_eq!(first.frames[1].name, "main.main");
    assert_eq!(first.hit_breakpoints, [1]);
    assert_eq!(first.local_value("total"), Some("0"));
    assert!(matches!(first.watch("total"), Some(Ok(v)) if v == "0"), "watch was {:?}", first.watch("total"));
    // The order is a struct whose fields were read ahead, so a change inside would show.
    let scopes = first.scopes(0).unwrap_or_default();
    let order = scopes.iter().filter_map(|s| s.vars.as_deref()).flatten().find(|v| v.name == "order");
    let fields: Vec<&str> = order.and_then(|o| o.children.as_deref()).into_iter().flatten().map(|v| &*v.name).collect();
    assert_eq!(fields, ["ID", "Customer", "Lines"]);
    // dlv's own build, next to the sources, until the session ends.
    let built = |dir: &Path| {
        let entries = std::fs::read_dir(dir).expect("workspace").filter_map(Result::ok);
        entries.map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("__debug_bin")).count()
    };
    assert_eq!(built(dir), 1);

    engine.resume(sid, Step::Continue).expect("continue");
    pump(&mut engine, &mut rx, |e| stops(e, sid) >= 2).await;
    let session = engine.session(sid).expect("session");
    let (prev, cur) = (&session.history[0], &session.history[1]);
    let scopes = cur.scopes(0).unwrap_or_default();
    let scope = scopes.iter().find(|s| s.vars.iter().flatten().any(|v| v.name == "total")).expect("scope");
    let total = vec![scope.name.clone(), "total".to_owned()];
    assert_eq!(diff(Some(prev), cur, 0).get(&total), Some(&Change::Changed { was: "0".into() }));
    assert_eq!(cur.local_value("total"), Some("8900"));

    engine.resume(sid, Step::In).expect("step in");
    pump(&mut engine, &mut rx, |e| stops(e, sid) >= 3).await;
    let stepped = engine.session(sid).and_then(|s| s.latest()).expect("snapshot");
    let names: Vec<&str> = stepped.frames.iter().take(3).map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["main.lineTotal", "main.orderTotal", "main.main"]);
    assert_eq!(stepped.top().map(|f| f.line), Some(LINE_TOTAL));

    // Without the breakpoint the program runs to its end; dlv removes its build.
    engine.toggle_breakpoint(&source, BREAK_LINE);
    engine.resume(sid, Step::Continue).expect("continue");
    pump(&mut engine, &mut rx, |e| exited(e, sid)).await;
    let printed = |text: &str| engine.output().iter().any(|l| l.kind == OutputKind::Stdout && l.text.contains(text));
    // As the program's, not as dlv's own lines: tuitor asks dlv for it as output events.
    assert!(printed("revenue 564.03"), "program output missing");
    engine.shutdown().await;
    assert_eq!(built(dir), 0, "dlv left its build behind");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs TUITOR_DLV"]
async fn auto_mode_tests_a_test_file_as_vs_code_does() {
    let (tmp, adapters) = workspace("go-test");
    let dir = tmp.path();
    let test = dir.join("orders_test.go");
    std::fs::write(
        &test,
        "package main\n\nimport \"testing\"\n\nfunc TestLineTotal(t *testing.T) {\n\
         \tif got := lineTotal(Line{Qty: 2, Price: 450}); got != 900 {\n\t\tt.Fatalf(\"lineTotal = %d\", got)\n\t}\n}\n",
    )
    .expect("test file");
    let (mut engine, mut rx) = Engine::new(EngineOptions { adapters, workspace: dir.to_path_buf(), state_file: None });
    engine.toggle_breakpoint(&dir.join("main.go"), LINE_TOTAL + 1);

    // VS Code's "Go: Launch package" snippet with `program` on a test file: dlv knows no
    // `auto` and gets `test` for the file's package.
    let sid = engine.start(launch(json!({ "mode": "auto", "program": test.display().to_string() }))).expect("start");
    pump(&mut engine, &mut rx, |e| stops(e, sid) >= 1).await;
    let stop = engine.session(sid).and_then(|s| s.latest()).expect("snapshot");
    let names: Vec<&str> = stop.frames.iter().take(2).map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["orders.lineTotal", "orders.TestLineTotal"]);

    engine.stop(sid).expect("stop");
    pump(&mut engine, &mut rx, |e| exited(e, sid)).await;
    engine.shutdown().await;
}
