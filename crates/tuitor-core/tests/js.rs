//! End-to-end with js-debug's standalone DAP server and Node: the program runs in a child
//! session that js-debug asks for with `startDebugging`. Needs the server, so it is ignored
//! by default (the path is where README's install steps put it):
//!
//! ```sh
//! TUITOR_JS_ADAPTER=~/.local/share/tuitor/adapters/js-debug/js-debug/src/dapDebugServer.js \
//! cargo test -p tuitor-core --test js -- --ignored
//! ```

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;
use tuitor_core::config::{Adapters, LaunchConfig, RequestKind};
use tuitor_core::log::OutputKind;
use tuitor_core::session::{Session, SessionId};
use tuitor_core::snapshot::{Change, diff};
use tuitor_core::{Engine, EngineMsg, EngineOptions, Step};

const BREAK_LINE: u32 = 50; // `total += lineTotal(line);`

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

fn child_of(engine: &Engine, parent: SessionId) -> Option<&Session> {
    engine.sessions().iter().find(|s| s.parent == Some(parent))
}

fn child_stops(engine: &Engine, parent: SessionId) -> usize {
    child_of(engine, parent).map_or(0, |s| s.history.len())
}

/// A directory with the Node example and an adapters.toml that runs js-debug for `adapter`.
fn workspace(name: &str, adapter: &str) -> (PathBuf, PathBuf, Adapters) {
    let server = std::env::var("TUITOR_JS_ADAPTER").expect("TUITOR_JS_ADAPTER");
    let dir = std::env::temp_dir().join(format!("tuitor-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/js/orders.js");
    std::fs::copy(&example, dir.join("orders.js")).expect("copy");
    let script = std::fs::canonicalize(dir.join("orders.js")).expect("script path");

    // The built-in entry with the server from the environment.
    let builtin = Adapters::default();
    let spec = builtin.get(adapter).expect("built-in adapter");
    let launch_type = spec.launch_type.as_deref().map(|t| format!("launch_type = {t:?}\n")).unwrap_or_default();
    let file = dir.join("adapters.toml");
    let toml = format!(
        "[{adapter}]\ntransport = \"server\"\ncommand = \"node\"\nargs = [{server:?}, \"${{port}}\", \"127.0.0.1\"]\n\
         extensions = {:?}\n{launch_type}",
        spec.extensions,
    );
    std::fs::write(&file, toml).expect("adapters.toml");
    (dir, script, Adapters::load(Some(&file)).expect("adapters"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs TUITOR_JS_ADAPTER"]
async fn node_runs_in_a_child_session() {
    let (dir, script, adapters) = workspace("js", "pwa-node");
    let (mut engine, mut rx) = Engine::new(EngineOptions { adapters, workspace: dir.clone(), state_file: None });
    engine.toggle_breakpoint(&script, BREAK_LINE);
    engine.add_watch("total");

    let Value::Object(args) = json!({
        "program": script.display().to_string(),
        "cwd": dir.display().to_string(),
        "console": "internalConsole",
    }) else {
        unreachable!()
    };
    let parent = engine.start(LaunchConfig::adhoc("orders", "pwa-node", RequestKind::Launch, args)).expect("start");
    let tmp = engine.session(parent).and_then(|s| s.temp_dir()).map(Path::to_path_buf).expect("a TMPDIR of its own");

    // js-debug starts Node, then asks for a child session in which the program runs.
    pump(&mut engine, &mut rx, |e| child_stops(e, parent) >= 1).await;
    let child = child_of(&engine, parent).expect("child session");
    let cid = child.id;
    assert!(child.name.starts_with(&engine.session(parent).expect("parent").name), "child named {}", child.name);
    assert!(child.config.arguments.contains_key("__pendingTargetId"));
    assert!(engine.session(parent).expect("parent").history.is_empty(), "the parent itself never stops");
    // js-debug makes its directory, which it never removes, in the one tuitor gave it.
    let made = |name: String| name.starts_with("vscode-js-debug-");
    let entries = std::fs::read_dir(&tmp).expect("TMPDIR").filter_map(Result::ok);
    assert!(entries.map(|e| e.file_name().to_string_lossy().into_owned()).any(made), "{} unused", tmp.display());

    let first = child.latest().expect("snapshot");
    let top = first.top().expect("frame");
    assert_eq!((top.line, top.path.as_deref()), (BREAK_LINE, Some(script.as_path())));
    assert_eq!(top.name, "orderTotal");
    assert_eq!(first.hit_breakpoints, [1]);
    assert_eq!(first.local_value("total"), Some("0"));
    assert!(matches!(first.watch("total"), Some(Ok(v)) if v == "0"), "watch was {:?}", first.watch("total"));
    // Breakpoints went to the child, where the code runs, and not to the parent.
    let bp = engine.breakpoints().get(1).expect("breakpoint");
    assert_eq!(bp.bindings.keys().copied().collect::<Vec<_>>(), [cid]);
    assert_eq!(bp.verified(), Some(true));

    engine.resume(cid, Step::Continue).expect("continue");
    pump(&mut engine, &mut rx, |e| child_stops(e, parent) >= 2).await;
    let child = engine.session(cid).expect("child");
    let (prev, cur) = (&child.history[0], &child.history[1]);
    let scope = cur.scopes.iter().find(|s| s.vars.iter().flatten().any(|v| v.name == "total")).expect("scope");
    let total = vec![scope.name.clone(), "total".to_owned()];
    assert_eq!(diff(Some(prev), cur).get(&total), Some(&Change::Changed { was: "0".into() }));
    assert_eq!(cur.local_value("total"), Some("8900"));

    // The program runs to its end once the breakpoint is gone; then both sessions end.
    engine.toggle_breakpoint(&script, BREAK_LINE);
    engine.resume(cid, Step::Continue).expect("continue");
    pump(&mut engine, &mut rx, |e| e.sessions().iter().all(|s| !s.is_alive())).await;
    let printed = |text: &str| engine.output().iter().any(|l| l.kind == OutputKind::Stdout && l.text.contains(text));
    assert!(printed("revenue 564.03"), "program output missing");
    // With the adapter gone, so is its temporary directory.
    engine.shutdown().await;
    assert!(!tmp.exists(), "{} is left over", tmp.display());

    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs TUITOR_JS_ADAPTER"]
async fn stopping_a_child_stops_the_program() {
    let (dir, script, adapters) = workspace("js-stop", "node");
    let (mut engine, mut rx) = Engine::new(EngineOptions { adapters, workspace: dir.clone(), state_file: None });
    // `node`, as VS Code's own configurations say; tuitor sends it as `pwa-node`.
    let Value::Object(args) = json!({ "program": script.display().to_string(), "stopOnEntry": true }) else {
        unreachable!()
    };
    let parent = engine.start(LaunchConfig::adhoc("orders", "node", RequestKind::Launch, args)).expect("start");
    pump(&mut engine, &mut rx, |e| child_stops(e, parent) >= 1).await;
    let cid = child_of(&engine, parent).expect("child").id;

    engine.stop(cid).expect("stop");
    pump(&mut engine, &mut rx, |e| e.sessions().iter().all(|s| !s.is_alive())).await;
    assert!(!engine.output().iter().any(|l| l.text.contains("revenue")), "the program ran on after stop");

    std::fs::remove_dir_all(dir).ok();
}
