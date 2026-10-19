mod app;
mod clipboard;
mod command;
mod keymap;
mod model;
mod overlay;
mod runner;
mod script;
mod sources;
mod ui;

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde_json::{Map, Value};
use tuitor_core::config::{
    Adapters, INSTALL_HELP, LaunchConfig, RequestKind, TransportKind, Variables, load_launch_json,
};
use tuitor_core::{Engine, EngineOptions, persist};

use crate::app::App;
use crate::keymap::Keymap;

/// A terminal debugger that shows everything at once.
#[derive(Parser)]
#[command(name = "tuitor", version)]
struct Cli {
    /// Launch configuration to start (a name from .vscode/launch.json).
    config: Option<String>,

    /// Workspace directory; defaults to the current directory.
    #[arg(short = 'C', long, global = true)]
    workspace: Option<PathBuf>,

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
    /// Show the launch configurations and adapters tuitor sees.
    Configs,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let workspace = std::fs::canonicalize(cli.workspace.clone().unwrap_or_else(|| PathBuf::from(".")))
        .context("workspace directory")?;
    let config_dir = dirs::config_dir().map(|d| d.join("tuitor"));
    let _log = init_logging();

    let adapters = Adapters::load(config_dir.as_ref().map(|d| d.join("adapters.toml")).as_deref())?;
    let keymap = Keymap::load(config_dir.as_ref().map(|d| d.join("keys.toml")).as_deref())?;
    let vars = Variables { workspace: workspace.clone() };
    let launch_json = cli.launch_json.clone().unwrap_or_else(|| workspace.join(".vscode/launch.json"));
    let (configs, config_error) = if launch_json.exists() {
        match load_launch_json(&launch_json, &vars) {
            Ok(configs) => (configs, None),
            Err(err) => (Vec::new(), Some(format!("{err:#}"))),
        }
    } else {
        (Vec::new(), None)
    };

    let start = match &cli.command {
        Some(Command::Configs) => return print_configs(&configs, &adapters, &workspace, &launch_json, config_error),
        Some(Command::Exec { adapter, program, args }) => Some(exec_config(adapter, program, args, &workspace)?),
        Some(Command::Listen { port, maps }) => Some(listen_config(*port, maps, &workspace)?),
        None => match &cli.config {
            Some(name) => Some(
                configs
                    .iter()
                    .find(|c| c.name.eq_ignore_ascii_case(name))
                    .cloned()
                    .with_context(|| format!("no configuration named `{name}` in {}", launch_json.display()))?,
            ),
            None if configs.len() == 1 => configs.first().cloned(),
            None => None,
        },
    };

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(async move {
        let (engine, rx) = Engine::new(EngineOptions {
            adapters,
            workspace: workspace.clone(),
            state_file: persist::state_file(&workspace),
        });
        let mut app = App::new(engine, keymap, configs);
        if let Some(error) = config_error {
            app.execute_error(&format!("launch.json: {error}"));
        }
        match &cli.script {
            Some(script) => script::run(app, rx, start, script, parse_size(&cli.size)?).await,
            None => runner::run(app, rx, start).await,
        }
    })
}

fn exec_config(
    adapter: &str,
    program: &std::path::Path,
    args: &[String],
    workspace: &std::path::Path,
) -> Result<LaunchConfig> {
    let program = std::fs::canonicalize(program).with_context(|| format!("program {}", program.display()))?;
    let mut extra = Map::new();
    extra.insert("program".into(), Value::String(program.display().to_string()));
    extra.insert("args".into(), Value::Array(args.iter().cloned().map(Value::String).collect()));
    extra.insert("cwd".into(), Value::String(workspace.display().to_string()));
    let name = program.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "program".into());
    Ok(LaunchConfig::adhoc(&name, adapter, RequestKind::Launch, extra))
}

fn listen_config(port: u16, maps: &[String], workspace: &std::path::Path) -> Result<LaunchConfig> {
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

fn print_configs(
    configs: &[LaunchConfig],
    adapters: &Adapters,
    workspace: &std::path::Path,
    launch_json: &std::path::Path,
    error: Option<String>,
) -> Result<()> {
    println!("launch configurations ({}):", launch_json.display());
    if let Some(error) = error {
        println!("  error: {error}");
    }
    if configs.is_empty() {
        println!("  none");
    }
    for config in configs {
        let known = if adapters.get(&config.adapter).is_some() { "" } else { "  (no adapter for this type)" };
        println!("  {:<32} type={}{known}", config.name, config.adapter);
    }
    println!("\nadapters:");
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
                    println!("\n  {:<10} not installed: {what}", "");
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
