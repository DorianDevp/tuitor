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

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use tuitor_code::config::ServerConfig;
use tuitor_code::{Code, Entry, Found, Observed, Root, Servers, Update, Why};

fn example(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name);
    std::fs::canonicalize(&dir).expect("example")
}

fn server(variable: &str, initialization_options: Option<Value>) -> ServerConfig {
    let command = std::env::var(variable).unwrap_or_else(|_| panic!("{variable} names the server"));
    ServerConfig { command: Some(command), args: None, initialization_options }
}

/// intelephense with its index in a temporary directory, removed with it.
struct Intelephense(PathBuf);

impl Intelephense {
    fn new(name: &str) -> (Intelephense, Servers) {
        let storage = std::env::temp_dir().join(format!("tuitor-intelephense-{name}-{}", std::process::id()));
        let mut options = json!({
            "storagePath": storage.join("workspace"),
            "globalStoragePath": storage.join("global"),
        });
        if let Ok(licence) = std::env::var("TUITOR_INTELEPHENSE_LICENCE") {
            options["licenceKey"] = licence.into();
        }
        let servers = Servers { php: Some(server("TUITOR_INTELEPHENSE", Some(options))), ..Servers::default() };
        (Intelephense(storage), servers)
    }
}

impl Drop for Intelephense {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// The trees through `line` of `file` in `root`, once the server has answered.
async fn paths(root: &Path, servers: Servers, file: &str, line: u32, observed: Vec<Observed>) -> (Found, Code) {
    let mut code = Code::new(root.to_path_buf(), servers);
    let mut query = code.paths(root.join(file), line, observed).expect("a server for it");
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
    eprintln!("{} requests", found.requests);
    for note in &found.notes {
        eprintln!("note: {note}");
    }
    eprintln!("{}", tree(&found));
    (found, code)
}

/// The trees, a stage a line, indented by depth: `◆` marks the anchor, `⇢` a call a run
/// showed.
fn tree(found: &Found) -> String {
    let mut out = String::new();
    for (i, stage) in found.stages.iter().enumerate() {
        let root = stage.root.map(|root| format!("  [{root:?}]")).unwrap_or_default();
        let cut = stage.cut.map(|cut| format!("  ({cut:?})")).unwrap_or_default();
        let mark = if stage.anchor {
            "◆ "
        } else if stage.observed {
            "⇢ "
        } else {
            ""
        };
        let sites: Vec<String> = stage.sites.iter().map(u32::to_string).collect();
        let at = if sites.is_empty() { String::new() } else { format!("  @{}", sites.join(",")) };
        let name = &found.function(i).name;
        out.push_str(&format!("{:>3} {}{mark}{name}{at}{root}{cut}\n", i, "  ".repeat(stage.depth)));
    }
    out
}

/// The end-to-end paths, each as the names of its functions.
fn e2e(found: &Found) -> Vec<Vec<String>> {
    let names = |leaf: usize| found.chain(leaf).iter().map(|s| found.function(*s).name.clone()).collect();
    found.paths().into_iter().map(names).collect()
}

/// The names below the first stage of `name` in the tree.
fn calls_of(found: &Found, name: &str) -> Vec<String> {
    let stage = (0..found.stages.len()).find(|s| found.function(*s).name == name).expect(name);
    found.stages[stage].children.iter().map(|c| found.function(*c).name.clone()).collect()
}

/// The lines of the end-to-end path through the stage `leaf`, as `line why`.
fn lines(found: &Found, leaf: usize) -> Vec<(u32, String)> {
    let chosen: BTreeSet<usize> = found.path_through(leaf).into_iter().collect();
    found
        .selection(&chosen)
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

fn has(paths: &[Vec<String>], path: &[&str]) -> bool {
    paths.iter().any(|known| known.iter().map(String::as_str).eq(path.iter().copied()))
}

#[tokio::test]
#[ignore = "needs gopls: TUITOR_GOPLS"]
async fn go_from_main_through_order_total_into_what_it_calls() {
    let servers = Servers { go: Some(server("TUITOR_GOPLS", None)), ..Servers::default() };
    let (found, mut code) = paths(&example("go"), servers, "main.go", 57, Vec::new()).await;
    code.shutdown().await;
    assert_eq!(found.target().name, "orderTotal");
    assert_eq!(found.stages[0].root, Some(Root::Entry(Entry::Main)));
    let paths = e2e(&found);
    // Below the function asked about, the paths part: one into lineTotal, one into discount.
    assert!(has(&paths, &["main", "orderTotal", "lineTotal"]), "{paths:?}");
    assert!(has(&paths, &["main", "orderTotal", "discount"]), "{paths:?}");
    // main goes on after orderTotal: money, beside the paths.
    assert!(calls_of(&found, "main").contains(&"money".to_owned()));
    let leaf = found.paths()[0];
    let lines = lines(&found, leaf);
    // Delve's entry is the declaration.
    assert!(lines.contains(&(95, "calls orderTotal".into())), "{lines:?}");
    assert!(lines.contains(&(55, "entry".into())) && lines.contains(&(58, "calls lineTotal".into())), "{lines:?}");
    assert!(lines.contains(&(57, "target".into())), "{lines:?}");
}

#[tokio::test]
#[ignore = "needs rust-analyzer: TUITOR_RUST_ANALYZER"]
async fn rust_from_main_through_order_total_into_what_it_calls() {
    let servers = Servers { rust: Some(server("TUITOR_RUST_ANALYZER", None)), ..Servers::default() };
    let (found, mut code) = paths(&example("rust"), servers, "src/main.rs", 55, Vec::new()).await;
    code.shutdown().await;
    assert_eq!(found.target().name, "order_total");
    assert_eq!(found.stages[0].root, Some(Root::Entry(Entry::Main)));
    let paths = e2e(&found);
    assert!(has(&paths, &["main", "order_total", "line_total"]), "{paths:?}");
    assert!(has(&paths, &["main", "order_total", "discount"]), "{paths:?}");
    // rust-analyzer reports no call inside a macro's arguments: `money` in `println!`.
    assert!(found.unseen.contains("macros"));
    let lines = lines(&found, found.paths()[0]);
    assert!(lines.contains(&(54, "entry".into())) && lines.contains(&(56, "calls line_total".into())), "{lines:?}");
}

#[tokio::test]
#[ignore = "needs typescript-language-server: TUITOR_TYPESCRIPT_LANGUAGE_SERVER"]
async fn typescript_from_the_script_through_order_total_into_what_it_calls() {
    let ts = server("TUITOR_TYPESCRIPT_LANGUAGE_SERVER", None);
    let servers = Servers { typescript: Some(ts), ..Servers::default() };
    let (found, mut code) = paths(&example("typescript"), servers, "orders.ts", 41, Vec::new()).await;
    code.shutdown().await;
    assert_eq!(found.stages[0].root, Some(Root::Entry(Entry::Script)));
    let paths = e2e(&found);
    assert!(has(&paths, &["orders.ts", "orderTotal", "lineTotal"]), "{paths:?}");
    assert!(has(&paths, &["orders.ts", "orderTotal", "discount"]), "{paths:?}");
    assert!(calls_of(&found, "orders.ts").contains(&"money".to_owned()));
    let lines = lines(&found, found.paths()[0]);
    // TypeScript's highlights tell no writes; the text does.
    assert!(lines.contains(&(40, "entry".into())) && lines.contains(&(42, "calls lineTotal".into())), "{lines:?}");
    assert!(lines.contains(&(78, "calls orderTotal".into())), "{lines:?}");
}

#[tokio::test]
#[ignore = "needs typescript-language-server: TUITOR_TYPESCRIPT_LANGUAGE_SERVER"]
async fn angular_from_ng_on_init_through_order_total_into_the_helpers() {
    let ts = server("TUITOR_TYPESCRIPT_LANGUAGE_SERVER", None);
    let servers = Servers { typescript: Some(ts), ..Servers::default() };
    let (found, mut code) = paths(&example("angular"), servers, "src/app/app.ts", 67, Vec::new()).await;
    code.shutdown().await;
    assert_eq!(found.target().name, "App.orderTotal");
    // Angular calls ngOnInit, and render from the template: no code the server sees does.
    assert_eq!(found.stages[0].root, Some(Root::Uncalled));
    let paths = e2e(&found);
    assert!(has(&paths, &["App.ngOnInit", "App.render", "App.orderTotal", "lineTotal"]), "{paths:?}");
    assert!(has(&paths, &["App.ngOnInit", "App.render", "App.orderTotal", "discount"]), "{paths:?}");
    assert!(calls_of(&found, "App.render").contains(&"money".to_owned()));
    assert!(found.unseen.contains("templates"));
}

#[tokio::test]
#[ignore = "needs intelephense: TUITOR_INTELEPHENSE"]
async fn php_from_the_script_down_to_line_total() {
    let (_storage, servers) = Intelephense::new("php");
    let (found, mut code) = paths(&example("php"), servers, "orders.php", 51, Vec::new()).await;
    code.shutdown().await;
    let paths = e2e(&found);
    assert_eq!(paths[0], ["orders.php", "orderTotal", "lineTotal"]);
    assert_eq!(found.stages[0].root, Some(Root::Entry(Entry::Script)));
    // From the script, the request goes on into the other functions it calls.
    let script = calls_of(&found, "orders.php");
    assert!(script.contains(&"money".to_owned()), "{script:?}");
    let lines = lines(&found, found.paths()[0]);
    assert!(lines.contains(&(66, "entry".into())) && lines.contains(&(68, "calls lineTotal".into())), "{lines:?}");
    assert!(lines.contains(&(95, "calls orderTotal".into())), "{lines:?}");
}

#[tokio::test]
#[ignore = "needs intelephense: TUITOR_INTELEPHENSE"]
async fn php_paths_from_both_controllers_through_the_helper_and_on() {
    let root = example("php-paths");
    let (_storage, servers) = Intelephense::new("php-paths");
    // What a run's stack showed: the dispatcher calls the listener.
    let observed = vec![Observed {
        path: root.join("src/Event/Dispatcher.php"),
        line: 21,
        callee: "App\\Event\\LoyaltyListener->onOrderTotalled".into(),
        callee_path: root.join("src/Event/LoyaltyListener.php"),
    }];
    let (found, mut code) = paths(&root, servers, "src/Helper/Money.php", 16, observed).await;
    code.shutdown().await;
    assert_eq!(found.target().name, "Money::lineTotal");
    let paths = e2e(&found);
    for controller in ["InvoiceController::print", "OrderController::show"] {
        assert!(has(&paths, &[controller, "OrderService::total", "Money::lineTotal"]), "{paths:?}");
        let through = [controller, "OrderService::total", "Dispatcher::dispatch"];
        let listener = [&through[..], &["LoyaltyListener::onOrderTotalled", "Money::lineTotal"]].concat();
        assert!(has(&paths, &listener), "{paths:?}");
        // The controller goes on below the paths: the repository and the helper's format.
        let calls = calls_of(&found, controller);
        assert!(calls.contains(&"OrderRepository::find".to_owned()), "{calls:?}");
        assert!(calls.contains(&"Money::format".to_owned()), "{calls:?}");
    }
    // The paths part in the service: straight to the helper, or through the dispatcher.
    let total = (0..found.stages.len()).find(|s| found.function(*s).name == "OrderService::total").expect("total");
    assert_eq!(found.forks(total), 2);
    let listener = found.stages.iter().position(|s| s.observed).expect("a call a run showed");
    assert_eq!(found.function(listener).name, "LoyaltyListener::onOrderTotalled");
}
