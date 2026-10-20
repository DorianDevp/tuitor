mod app;
mod clipboard;
mod command;
mod doctor;
mod editor;
mod flow;
mod installer;
mod keymap;
mod mappings;
mod model;
mod outline;
mod overlay;
mod paths;
mod picker;
mod places;
mod runner;
mod script;
mod search;
mod sets;
mod signals;
mod sources;
mod tab;
mod ui;

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde_json::{Map, Value};
use tokio::sync::mpsc;
use tuitor_core::config::{Adapters, LaunchConfig, RequestKind, TransportKind, home_relative};
use tuitor_core::doctor::{Options, ProjectInput};
use tuitor_core::install::{self, Output, Package, Plan};
use tuitor_core::project::{Project, Source};
use tuitor_core::store::Store;

use crate::app::App;
use crate::editor::Settings;
use crate::keymap::Keymap;
use crate::signals::Signals;

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
    /// Check what debugging needs: each adapter, and what the project's configurations need
    /// (a free port, Xdebug, a browser). Exits with 1 when something needed is missing.
    Doctor {
        /// Adapters to check, by type (php, gdb, pwa-chrome…) or name (js-debug, codelldb,
        /// delve); they are then needed too. Without, every adapter is listed.
        adapters: Vec<String>,
        /// The PHP to look for Xdebug in, instead of `php` on PATH.
        #[arg(long, value_name = "PATH")]
        php: Option<String>,
    },
    /// Install an adapter from its official release (vscode-php-debug, js-debug, CodeLLDB) into
    /// tuitor's adapter directory, or Delve with `go install`; asks first. For gdb, LLDB and
    /// Xdebug it names your distribution's package.
    Install {
        /// An adapter type (php, pwa-node, pwa-chrome, lldb, go…) or a name (vscode-php-debug,
        /// js-debug, codelldb, delve, gdb, lldb-dap, xdebug).
        adapter: String,
        /// The release to install, instead of the one README names (Delve: the latest).
        #[arg(long)]
        version: Option<String>,
        /// Install without asking.
        #[arg(long, short)]
        yes: bool,
    },
}

fn main() -> Result<ExitCode> {
    let cli = Cli::parse();
    let config_dir = dirs::config_dir().map(|d| d.join("tuitor"));
    let _log = init_logging();

    let adapters = Adapters::load(config_dir.as_ref().map(|d| d.join("adapters.toml")).as_deref())?;
    let keymap = Keymap::load(config_dir.as_ref().map(|d| d.join("keys.toml")).as_deref())?;
    let settings = Settings::load(config_dir.as_ref().map(|d| d.join("config.toml")).as_deref())?;
    let store = Store::new(Store::default_path().context("no config directory for tuitor's configurations")?);
    let workspace = directory(cli.workspace.as_deref().unwrap_or(Path::new(".")))?;
    let first = Project::open(&workspace, &store, cli.launch_json.as_deref());
    let others = cli.open.iter().map(|dir| directory(dir)).collect::<Result<Vec<_>>>()?;

    let start = match &cli.command {
        Some(Command::Configs) => {
            let mut projects = vec![first];
            projects.extend(others.iter().map(|dir| Project::open(dir, &store, None)));
            return print_configs(&projects, &adapters, &store).map(|()| ExitCode::SUCCESS);
        }
        Some(Command::Doctor { adapters: only, php }) => {
            // The current directory counts only if it is a project, as for a tab.
            let first = (cli.workspace.is_some() || first.is_project()).then_some(first);
            let mut projects: Vec<ProjectInput> = first.iter().map(ProjectInput::of).collect();
            projects.extend(others.iter().map(|dir| ProjectInput::of(&Project::open(dir, &store, None))));
            let options = Options { only: only.clone(), php: php.clone(), own_ports: Vec::new() };
            let report = tuitor_core::doctor::check(&adapters, &projects, &options);
            print!("{}", doctor::print(&report));
            return Ok(if report.blocking() == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE });
        }
        Some(Command::Install { adapter, version, yes }) => {
            return install_cli(&adapters, adapter, version.as_deref(), *yes, &workspace);
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
        // Before anything is started that a signal would leave behind.
        let signals = Signals::new().context("listening for signals")?;
        let (inbox, rx) = mpsc::unbounded_channel();
        let mut app = App::new(keymap, settings, adapters, store, inbox);
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
            Some(script) => script::run(app, rx, start, script, parse_size(&cli.size)?, signals).await,
            None => runner::run(app, rx, start, signals).await,
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
        println!("\n`tuitor doctor` says what is missing and what installs it.");
    }
    Ok(())
}

/// `tuitor install`: says what it downloads from where, or runs, asks, installs, and checks
/// the result as starting a session does.
fn install_cli(adapters: &Adapters, name: &str, version: Option<&str>, yes: bool, cwd: &Path) -> Result<ExitCode> {
    let package = Package::parse(name).with_context(|| {
        let names: Vec<&str> = Package::ALL.iter().map(|p| p.name()).collect();
        format!("nothing called `{name}` to install (known: {}, or an adapter type)", names.join(", "))
    })?;
    let plan = install::plan(package, version)?;
    match &plan {
        Plan::Distro { command, .. } => {
            println!("{package} comes with your distribution; tuitor does not install it, as that takes root:");
            println!("    {command}");
            let found = installer::found(package, adapters, cwd);
            for line in &found {
                println!("{line}");
            }
            // There already, it needs nothing.
            let there = !found.is_empty() && found.iter().all(|line| line.starts_with('✓'));
            return Ok(if there { ExitCode::SUCCESS } else { ExitCode::FAILURE });
        }
        Plan::Release(release) => {
            println!("{package} {}, released by its authors on GitHub:", release.version);
            println!("    {}", release.url);
            let replacing = if release.dir.exists() { ", in place of what is there" } else { "" };
            println!("into {}{replacing}", home_relative(&release.dir));
            if let Some(tool) = release.missing_tools().first() {
                bail!("installing it needs `{tool}`, which is not on PATH");
            }
        }
        Plan::Go(go) => {
            println!("Delve, built by Go as its installation guide says:");
            println!("    {}", go.command());
        }
    }
    if !yes && !confirm("Install it? [y/N] ")? {
        println!("Nothing was installed.");
        return Ok(ExitCode::FAILURE);
    }
    let mut step = |text: String| println!("==> {text}");
    match &plan {
        Plan::Release(release) => release.install(Output::Terminal, &mut step)?,
        Plan::Go(go) => drop(go.install(Output::Terminal, &mut step)?),
        Plan::Distro { .. } => {}
    }
    let found = installer::found(package, adapters, cwd);
    for line in &found {
        println!("{line}");
    }
    Ok(if found.iter().all(|line| line.starts_with('✓')) { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

/// Asks a yes/no question on the terminal; anything but yes is no.
fn confirm(question: &str) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!("there is no terminal to ask on; --yes installs without asking");
    }
    print!("{question}");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
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
