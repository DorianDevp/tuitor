//! Paths through the examples, asked of real language servers. Each test needs its server, so
//! they are ignored by default; the variable names the program:
//!
//! ```sh
//! TUITOR_GOPLS=gopls TUITOR_RUST_ANALYZER=rust-analyzer \
//! TUITOR_TYPESCRIPT_LANGUAGE_SERVER=typescript-language-server TUITOR_INTELEPHENSE=intelephense \
//! cargo test -p tuitor-code --test servers -- --ignored
//! ```
//!
//! intelephense keeps its index in a temporary directory; `TUITOR_INTELEPHENSE_LICENCE` names
//! a licence file to read, if any.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use tuitor_code::config::ServerConfig;
use tuitor_code::{CallPath, Code, Entry, Found, Root, Servers, Update, Why};

fn example(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name);
    std::fs::canonicalize(&dir).expect("example")
}

fn server(variable: &str, initialization_options: Option<Value>) -> ServerConfig {
    let command = std::env::var(variable).unwrap_or_else(|_| panic!("{variable} names the server"));
    ServerConfig { command: Some(command), args: None, initialization_options }
}

/// The paths to `line` of `file` in `root`, once the server has answered.
async fn paths(root: &Path, servers: Servers, file: &str, line: u32) -> (Found, Code) {
    let mut code = Code::new(root.to_path_buf(), servers);
    let mut query = code.paths(root.join(file), line, Vec::new()).expect("a server for it");
    let found = tokio::time::timeout(Duration::from_secs(300), async {
        loop {
            for update in query.updates() {
                match update {
                    Update::Progress(text) => eprintln!("… {text}"),
                    Update::Done(found) => return found,
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("an answer in time")
    .expect("paths");
    for note in &found.notes {
        eprintln!("note: {note}");
    }
    for path in &found.paths {
        eprintln!("{}", describe(path));
    }
    (found, code)
}

fn describe(path: &CallPath) -> String {
    let steps: Vec<String> = path.steps.iter().map(|s| format!("{}:{}", s.name, s.line)).collect();
    let stops: Vec<String> = path
        .stops
        .iter()
        .map(|s| {
            format!("{}:{} {:?}", s.path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(), s.line, s.why)
        })
        .collect();
    format!("{:?}: {}\n    {}", path.root, steps.join(" → "), stops.join("\n    "))
}

/// The lines of `path` with why each was chosen, as `line why` with the why's name.
fn lines(path: &CallPath) -> Vec<(u32, String)> {
    path.stops
        .iter()
        .map(|stop| {
            let why = match &stop.why {
                Why::Target => "target".to_owned(),
                Why::Calls(name) => format!("calls {name}"),
                Why::Entry => "entry".to_owned(),
                Why::Writes(name) => format!("writes {name}"),
                Why::Uses(name) => format!("uses {name}"),
            };
            (stop.line, why)
        })
        .collect()
}

fn names(path: &CallPath) -> Vec<&str> {
    path.steps.iter().map(|step| step.name.as_str()).collect()
}

#[tokio::test]
#[ignore = "needs gopls: TUITOR_GOPLS"]
async fn go_from_main_down_to_line_total() {
    let servers = Servers { go: Some(server("TUITOR_GOPLS", None)), ..Servers::default() };
    let (found, mut code) = paths(&example("go"), servers, "main.go", 41).await;
    assert_eq!(found.target.name, "lineTotal");
    let path = &found.paths[0];
    assert_eq!((names(path), path.root), (vec!["main", "orderTotal", "lineTotal"], Root::Entry(Entry::Main)));
    let lines = lines(path);
    assert!(lines.contains(&(95, "calls orderTotal".into())), "{lines:?}");
    // Delve's entry is the declaration.
    assert!(lines.contains(&(55, "entry".into())) && lines.contains(&(58, "calls lineTotal".into())), "{lines:?}");
    assert_eq!(lines[lines.len() - 2..], [(40, "entry".into()), (41, "target".into())]);
    code.shutdown().await;
}

#[tokio::test]
#[ignore = "needs rust-analyzer: TUITOR_RUST_ANALYZER"]
async fn rust_from_main_down_to_line_total() {
    let servers = Servers { rust: Some(server("TUITOR_RUST_ANALYZER", None)), ..Servers::default() };
    let (found, mut code) = paths(&example("rust"), servers, "src/main.rs", 41).await;
    let path = &found.paths[0];
    assert_eq!((names(path), path.root), (vec!["main", "order_total", "line_total"], Root::Entry(Entry::Main)));
    let lines = lines(path);
    assert!(lines.contains(&(54, "entry".into())) && lines.contains(&(56, "calls line_total".into())), "{lines:?}");
    code.shutdown().await;
}

#[tokio::test]
#[ignore = "needs typescript-language-server: TUITOR_TYPESCRIPT_LANGUAGE_SERVER"]
async fn typescript_from_the_script_down_to_line_total() {
    let ts = server("TUITOR_TYPESCRIPT_LANGUAGE_SERVER", None);
    let servers = Servers { typescript: Some(ts), ..Servers::default() };
    let (found, mut code) = paths(&example("typescript"), servers, "orders.ts", 32).await;
    let path = &found.paths[0];
    assert_eq!((names(path), path.root), (vec!["orders.ts", "orderTotal", "lineTotal"], Root::Entry(Entry::Script)));
    let lines = lines(path);
    // TypeScript's highlights tell no writes; the text does.
    assert!(lines.contains(&(40, "entry".into())) && lines.contains(&(42, "calls lineTotal".into())), "{lines:?}");
    assert!(lines.contains(&(78, "calls orderTotal".into())), "{lines:?}");
    code.shutdown().await;
}

#[tokio::test]
#[ignore = "needs intelephense: TUITOR_INTELEPHENSE"]
async fn php_from_the_script_down_to_line_total() {
    let storage = std::env::temp_dir().join(format!("tuitor-intelephense-{}", std::process::id()));
    let mut options = json!({
        "storagePath": storage.join("workspace"),
        "globalStoragePath": storage.join("global"),
    });
    if let Ok(licence) = std::env::var("TUITOR_INTELEPHENSE_LICENCE") {
        options["licenceKey"] = licence.into();
    }
    let servers = Servers { php: Some(server("TUITOR_INTELEPHENSE", Some(options))), ..Servers::default() };
    let (found, mut code) = paths(&example("php"), servers, "orders.php", 51).await;
    code.shutdown().await;
    std::fs::remove_dir_all(&storage).ok();
    let path = &found.paths[0];
    assert_eq!((names(path), path.root), (vec!["orders.php", "orderTotal", "lineTotal"], Root::Entry(Entry::Script)));
    let lines = lines(path);
    assert!(lines.contains(&(66, "entry".into())) && lines.contains(&(68, "calls lineTotal".into())), "{lines:?}");
    assert!(lines.contains(&(95, "calls orderTotal".into())), "{lines:?}");
}
