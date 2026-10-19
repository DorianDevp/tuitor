mod app;
mod clipboard;
mod command;
mod keymap;
mod model;
mod overlay;
mod paths;
mod picker;
mod runner;
mod script;
mod sources;
mod tab;
mod ui;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde_json::{Map, Value};
use tokio::sync::mpsc;
use tuitor_core::config::{Adapters, INSTALL_HELP, LaunchConfig, RequestKind, TransportKind, home_relative};
use tuitor_core::project::{Project, Source};
use tuitor_core::store::Store;

use crate::app::App;
use crate::keymap::Keymap;

/// A terminal debugger that shows everything at once.
#[derive(Parser)]
#[command(name = "tuitor", version)]
struct Cli {
    /// Launch configuration to start (a name from .vscode/launch.json or tuitor's own).
    config: Option<String>,

    /// Project directory; defaults to the current directory.
    #[arg(short = 'C', long, global = true)]
    workspace: Option<PathBuf>,

    /// Open another project in a tab of its own; repeatable.
    #[arg(short = 'o', long = "open", value_name = "DIR", global = true)]
    open: Vec<PathBuf>,

    /// launch.json to read; defaults to <workspace>/.vscode/launch.json.
    #[arg(long, global = true)]
    launch_json: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,

    /// Run commands from a file without a terminal (development and screenshots).
    #[arg(long, hide = true, global = true)]
    script: Option<PathBuf>,

    /// Screen size for --script.
    #[arg(long, hide = true, global = true, default_value = "160x46")]
    size: String,
}

#[derive(Subcommand)]
enum Command {
    /// Debug a program directly, e.g. `tuitor exec ./target/debug/app -- --flag`.
    Exec {
        /// Adapter to use (see `tuitor configs`).
        #[arg(long, default_value = "gdb")]
        adapter: String,
        program: PathBuf,
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Wait for Xdebug to connect (PHP on this machine or in a container).
    Listen {
        #[arg(long, default_value_t = 9003)]
        port: u16,
        /// Path mapping remote=local, e.g. `--map /var/www/app=./api`; repeatable.
        #[arg(long = "map")]
        maps: Vec<String>,
    },
    /// Show the projects, launch configurations and adapters tuitor sees.
    Configs,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_dir = dirs::config_dir().map(|d| d.join("tuitor"));
    let _log = init_logging();

    let adapters = Adapters::load(config_dir.as_ref().map(|d| d.join("adapters.toml")).as_deref())?;
    let keymap = Keymap::load(config_dir.as_ref().map(|d| d.join("keys.toml")).as_deref())?;
    let store = Store::new(Store::default_path().context("no config directory for tuitor's configurations")?);
    let workspace = directory(cli.workspace.as_deref().unwrap_or(Path::new(".")))?;
    let first = Project::open(&workspace, &store, cli.launch_json.as_deref());
    let others = cli.open.iter().map(|dir| directory(dir)).collect::<Result<Vec<_>>>()?;

    let start = match &cli.command {
        Some(Command::Configs) => {
            let mut projects = vec![first];
            projects.extend(others.iter().map(|dir| Project::open(dir, &store, None)));
            return print_configs(&projects, &adapters, &store);
        }
        Some(Command::Exec { adapter, program, args }) => Some(exec_config(adapter, program, args, &workspace)?),
        Some(Command::Listen { port, maps }) => Some(listen_config(*port, maps, &workspace)?),
        None => match &cli.config {
            Some(name) => Some(named(&first, name)?),
            // The one configuration of the only project starts by itself.
            None if others.is_empty() && first.configurations.len() == 1 => {
                first.configurations[0].launch(&first.variables()).ok()
            }
            None => None,
        },
    };
    // Unless asked for, the current directory gets a tab only if it is a project.
    let open_first = cli.workspace.is_some() || start.is_some() || first.is_project();

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(async move {
        let (inbox, rx) = mpsc::unbounded_channel();
        let mut app = App::new(keymap, adapters, store, inbox);
        let mut projects: Vec<Project> = open_first.then_some(first).into_iter().collect();
        for dir in others {
            if !projects.iter().any(|p| p.dir == dir) {
                projects.push(Project::open(&dir, &app.store, None));
            }
        }
        let mut first_tab = None;
        for project in projects {
            let (id, outcome) = app.open(project);
            first_tab.get_or_insert(id);
            app.report(outcome);
        }
        if let Some(id) = first_tab {
            app.tabs.activate(id);
        }
        match &cli.script {
            Some(script) => script::run(app, rx, start, script, parse_size(&cli.size)?).await,
            None => runner::run(app, rx, start).await,
        }
    })
}

fn directory(path: &Path) -> Result<PathBuf> {
    let dir = std::fs::canonicalize(path).with_context(|| format!("project directory {}", path.display()))?;
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }
    Ok(dir)
}

fn named(project: &Project, name: &str) -> Result<LaunchConfig> {
    let config = project.configurations.iter().find(|c| c.name().eq_ignore_ascii_case(name)).with_context(|| {
        let names: Vec<&str> = project.configurations.iter().map(|c| c.name()).collect();
        format!("no configuration named `{name}` in {} (have: {})", home_relative(&project.dir), names.join(", "))
    })?;
    config.launch(&project.variables())
}

fn exec_config(adapter: &str, program: &Path, args: &[String], workspace: &Path) -> Result<LaunchConfig> {
    let program = std::fs::canonicalize(program).with_context(|| format!("program {}", program.display()))?;
    let mut extra = Map::new();
    extra.insert("program".into(), Value::String(program.display().to_string()));
    extra.insert("args".into(), Value::Array(args.iter().cloned().map(Value::String).collect()));
    extra.insert("cwd".into(), Value::String(workspace.display().to_string()));
    let name = program.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "program".into());
    Ok(LaunchConfig::adhoc(&name, adapter, RequestKind::Launch, extra))
}

fn listen_config(port: u16, maps: &[String], workspace: &Path) -> Result<LaunchConfig> {
    let mut mappings = Map::new();
    for map in maps {
        let Some((remote, local)) = map.split_once('=') else { bail!("--map expects remote=local, got `{map}`") };
        let local = workspace.join(local);
        let local = std::fs::canonicalize(&local).unwrap_or(local);
        mappings.insert(remote.to_owned(), Value::String(local.display().to_string()));
    }
    let mut extra = Map::new();
    extra.insert("port".into(), Value::from(port));
    if !mappings.is_empty() {
        extra.insert("pathMappings".into(), Value::Object(mappings));
    }
    Ok(LaunchConfig::adhoc("Listen for Xdebug", "php", RequestKind::Launch, extra))
}

fn print_configs(projects: &[Project], adapters: &Adapters, store: &Store) -> Result<()> {
    for project in projects {
        let kinds: Vec<&str> = project.kinds.iter().map(|k| k.name()).collect();
        let kinds = if kinds.is_empty() { "no known kind".to_owned() } else { kinds.join(", ") };
        println!("{} ({kinds}):", home_relative(&project.dir));
        for error in &project.errors {
            println!("  error: {error}");
        }
        if project.configurations.is_empty() {
            println!("  no launch configurations");
        }
        for config in &project.configurations {
            let source = match &config.source {
                Source::LaunchJson(path) => path.strip_prefix(&project.dir).unwrap_or(path).display().to_string(),
                Source::Tuitor => "tuitor's own".to_owned(),
            };
            let known = if adapters.get(config.adapter()).is_some() { "" } else { "  (no adapter for this type)" };
            println!("  {:<32} type={:<12} {source}{known}", config.name(), config.adapter());
        }
        println!();
    }
    println!("tuitor's own configurations are kept in {}", home_relative(store.path()));
    println!("\nadapters:");
    let workspace = projects.first().map(|p| p.dir.as_path()).unwrap_or(Path::new("."));
    let mut missing = false;
    for spec in adapters.all() {
        let command = std::iter::once(&spec.command).chain(&spec.args).map(String::as_str).collect::<Vec<_>>();
        print!("  {:<10} {}", spec.name, command.join(" "));
        match spec.transport {
            TransportKind::Tcp => println!("  → {}", spec.address.as_deref().unwrap_or("(no address)")),
            TransportKind::Stdio | TransportKind::Server => match spec.locate(workspace) {
                Ok(program) => println!("  → {}", program.display()),
                // Paths can be long; the reason gets a line of its own.
                Err(what) => {
                    missing = true;
                    println!("\n  {:<10} cannot run: {what}", "");
                }
            },
        }
    }
    if missing {
        println!("\nTo install a missing adapter, {INSTALL_HELP}.");
    }
    Ok(())
}

fn parse_size(size: &str) -> Result<(u16, u16)> {
    let (w, h) = size.split_once('x').context("--size is WIDTHxHEIGHT")?;
    Ok((w.parse()?, h.parse()?))
}

/// Logs go to a file; the terminal belongs to the UI. `TUITOR_LOG=dap=trace` shows protocol traffic.
fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let dir = dirs::state_dir().or_else(dirs::data_local_dir)?.join("tuitor");
    std::fs::create_dir_all(&dir).ok()?;
    let file = tracing_appender::rolling::never(dir, "tuitor.log");
    let (writer, guard) = tracing_appender::non_blocking(file);
    let filter = tracing_subscriber::EnvFilter::try_from_env("TUITOR_LOG").unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt().with_writer(writer).with_ansi(false).with_env_filter(filter).init();
    Some(guard)
}
