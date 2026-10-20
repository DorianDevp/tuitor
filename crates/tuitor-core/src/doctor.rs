//! What debugging needs, checked on this machine (`tuitor doctor`, `/doctor`). For each
//! adapter: the program it runs, the script of one that runs on Node.js, and the runtime
//! itself. For the configurations of a project, what they need besides: a free port to
//! listen on, a browser, Xdebug in the PHP that runs the code, the program to debug. A
//! finding that is not fine says what fixes it.

use std::cell::OnceCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::config::{
    AdapterSpec, Adapters, LaunchConfig, Missing, RequestKind, TransportKind, Variables, adapters_dir,
};
use crate::config::{home_relative, resolve_program};
use crate::process::{Ask, ask};
use crate::project::{Configuration, Project, Source, browser_on_path, starts_browser};

/// The Node.js vscode-php-debug needs, as the `engines` of its package.json say.
const PHP_DEBUG_NODE: (u32, u32) = (20, 18);
/// gdb's DAP server came with gdb 14.
const GDB_DAP: u32 = 14;
/// Where vscode-php-debug listens for Xdebug when the configuration names no port.
const XDEBUG_PORT: u16 = 9003;
/// How long a program gets to answer. gdb starts its Python for the DAP check.
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    /// Worth knowing; debugging works without changing it.
    Note,
    /// Debugging fails until it is fixed.
    Problem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fix {
    /// A command for the user's shell. tuitor does not run it: it may need root, which is the
    /// user's to give.
    Shell(String),
    /// A tuitor command line, as typed after `/`.
    Command(String),
    /// What to do, in words.
    Text(String),
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub status: Status,
    /// The adapter or configuration it is about.
    pub subject: String,
    pub text: String,
    pub fix: Option<Fix>,
}

#[derive(Debug, Clone)]
pub struct Section {
    pub title: String,
    /// The project it is about; none for the adapters.
    pub dir: Option<PathBuf>,
    pub findings: Vec<Finding>,
    /// Its problems stand in the way of what was asked about: a project's configurations, or
    /// the adapters named. The other adapters are listed for what they are worth.
    pub needed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub sections: Vec<Section>,
}

impl Report {
    /// Problems in what is needed.
    pub fn blocking(&self) -> usize {
        let needed = self.sections.iter().filter(|s| s.needed).flat_map(|s| &s.findings);
        needed.filter(|f| f.status == Status::Problem).count()
    }
}

/// A project to check.
#[derive(Debug, Clone)]
pub struct ProjectInput {
    pub dir: PathBuf,
    pub configurations: Vec<Configuration>,
    /// Why launch.json or tuitor's own configurations could not be read.
    pub errors: Vec<String>,
}

impl ProjectInput {
    pub fn of(project: &Project) -> ProjectInput {
        ProjectInput {
            dir: project.dir.clone(),
            configurations: project.configurations.clone(),
            errors: project.errors.clone(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Adapter types (or the names of what they run: `js-debug`, `codelldb`, `delve`) to
    /// check; none checks every adapter.
    pub only: Vec<String>,
    /// The PHP to look for Xdebug in where a configuration names none; `php` on PATH otherwise.
    pub php: Option<String>,
    /// Ports this tuitor's own sessions listen on: taken, but by what is meant to.
    pub own_ports: Vec<u16>,
}

/// Checks the adapters, and what the configurations of `projects` need.
pub fn check(adapters: &Adapters, projects: &[ProjectInput], options: &Options) -> Report {
    let probe = Probe::default();
    let cwd = projects.first().map_or_else(|| std::env::current_dir().unwrap_or_default(), |p| p.dir.clone());
    let wanted = match wanted_types(adapters, &options.only) {
        Ok(types) => types,
        Err(finding) => {
            let adapters = Section { title: "adapters".into(), dir: None, findings: vec![finding], needed: true };
            return Report { sections: vec![adapters] };
        }
    };
    let groups: Vec<Group> = groups(adapters)
        .into_iter()
        .filter(|group| wanted.is_empty() || group.iter().any(|spec| wanted.contains(&spec.name)))
        .map(|specs| {
            let findings = adapter(specs[0], &cwd, &probe, options);
            Group { names: specs.iter().map(|s| s.name.clone()).collect(), findings }
        })
        .collect();
    let mut sections = vec![Section {
        title: "adapters".into(),
        dir: None,
        findings: groups.iter().flat_map(|g| g.findings.iter().map(|f| f.about(&g.subject()))).collect(),
        needed: !wanted.is_empty(),
    }];
    for project in projects {
        let mut findings: Vec<Finding> =
            project.errors.iter().map(|error| Finding::new(Status::Problem, error.clone(), None)).collect();
        for config in &project.configurations {
            findings.extend(configuration(project, config, adapters, &groups, &probe, options));
        }
        if project.configurations.is_empty() {
            findings.push(Finding::new(Status::Note, "no launch configurations; /config add ‹type› adds one", None));
        }
        let title = home_relative(&project.dir);
        sections.push(Section { title, dir: Some(project.dir.clone()), findings, needed: true });
    }
    Report { sections }
}

/// The adapter types `only` names, or none for all of them.
fn wanted_types(adapters: &Adapters, only: &[String]) -> Result<Vec<String>, Finding> {
    let mut types = Vec::new();
    for name in only {
        let aliases: &[&str] = match name.as_str() {
            "vscode-php-debug" | "php-debug" | "xdebug" => &["php"],
            "js-debug" => &["pwa-node", "pwa-chrome"],
            "codelldb" => &["lldb"],
            "delve" | "dlv" => &["go"],
            other => &[other][..],
        };
        if let Some(unknown) = aliases.iter().find(|alias| adapters.get(alias).is_none()) {
            let known: Vec<&str> = adapters.names().collect();
            let text = format!("no adapter `{unknown}` (known: {})", known.join(", "));
            return Err(Finding { status: Status::Problem, subject: name.clone(), text, fix: None });
        }
        types.extend(aliases.iter().map(|alias| (*alias).to_owned()));
    }
    Ok(types)
}

/// Adapters that run the same program for the same kind of target, checked once: js-debug
/// serves `node` as `pwa-node` and `chrome` as `pwa-chrome`.
fn groups(adapters: &Adapters) -> Vec<Vec<&AdapterSpec>> {
    let target = |spec: &AdapterSpec| spec.launch_type.clone().unwrap_or_else(|| spec.name.clone());
    let same = |a: &AdapterSpec, b: &AdapterSpec| {
        (a.transport, &a.command, &a.args, target(a)) == (b.transport, &b.command, &b.args, target(b))
    };
    let mut groups: Vec<Vec<&AdapterSpec>> = Vec::new();
    for spec in adapters.all() {
        match groups.iter_mut().find(|group| same(group[0], spec)) {
            Some(group) => group.push(spec),
            None => groups.push(vec![spec]),
        }
    }
    // The type the others become first: `pwa-chrome, chrome`.
    for group in &mut groups {
        group.sort_by_key(|spec| spec.launch_type.is_some());
    }
    // As README lists them, then the user's own.
    let rank = |group: &Vec<&AdapterSpec>| ORDER.iter().position(|name| *name == group[0].name).unwrap_or(ORDER.len());
    groups.sort_by_key(rank);
    groups
}

/// The built-in adapters in the order README lists them.
const ORDER: [&str; 7] = ["php", "gdb", "lldb-dap", "lldb", "pwa-node", "pwa-chrome", "go"];

/// An adapter group's findings, without subject.
struct Group {
    names: Vec<String>,
    findings: Vec<Finding>,
}

impl Group {
    fn subject(&self) -> String {
        self.names.join(", ")
    }
}

impl Finding {
    fn new(status: Status, text: impl Into<String>, fix: Option<Fix>) -> Finding {
        Finding { status, subject: String::new(), text: text.into(), fix }
    }

    fn ok(text: impl Into<String>) -> Finding {
        Finding::new(Status::Ok, text, None)
    }

    fn about(&self, subject: &str) -> Finding {
        Finding { subject: subject.to_owned(), ..self.clone() }
    }
}

/// Answers several adapters need, asked once.
#[derive(Default)]
struct Probe {
    node: OnceCell<Option<String>>,
}

impl Probe {
    /// `node --version`: `v26.8.1`.
    fn node(&self) -> Option<&str> {
        let version = || {
            let node = resolve_program("node")?;
            let answer = ask(&Ask::new(&node, &["--version"]))?;
            Some(answer.stdout.trim().to_owned()).filter(|v| answer.success() && !v.is_empty())
        };
        self.node.get_or_init(version).as_deref()
    }
}

// ---- adapters -----------------------------------------------------------------------------

fn adapter(spec: &AdapterSpec, cwd: &Path, probe: &Probe, options: &Options) -> Vec<Finding> {
    if spec.transport == TransportKind::Tcp {
        return vec![match &spec.address {
            Some(address) => Finding::ok(format!("connects to {address}; nothing to install")),
            None => Finding::new(Status::Problem, "a `tcp` adapter needs an `address` (adapters.toml)", None),
        }];
    }
    let mut findings = match spec.locate(cwd) {
        Ok(program) => installed(spec, &program, probe),
        Err(missing) => vec![not_installed(spec, &missing)],
    };
    match spec.launch_type.as_deref().unwrap_or(&spec.name) {
        "php" => findings.push(local_xdebug(options)),
        "pwa-chrome" => findings.push(browser()),
        _ => {}
    }
    findings
}

/// What runs, and what it says it is.
fn installed(spec: &AdapterSpec, program: &Path, probe: &Probe) -> Vec<Finding> {
    match program_name(program).as_str() {
        "node" => vec![on_node(spec, probe)],
        "gdb" => vec![gdb(program)],
        "dlv" => {
            let version = ask(&Ask::new(program, &["version"]))
                .and_then(|a| a.stdout.lines().find_map(|l| l.strip_prefix("Version: ").map(str::to_owned)));
            vec![Finding::ok(version.map_or_else(|| "Delve".to_owned(), |v| format!("Delve {v}")))]
        }
        _ => {
            let what = package_version(program).map_or_else(
                || format!("runs {}", home_relative(program)),
                |(name, version)| format!("{name} {version}"),
            );
            vec![Finding::ok(what)]
        }
    }
}

/// An adapter that runs on Node.js: its script's package, and Node's version.
fn on_node(spec: &AdapterSpec, probe: &Probe) -> Finding {
    let Some(node) = probe.node() else {
        return Finding::new(Status::Problem, "`node --version` does not answer", None);
    };
    let script = spec.args.iter().find(|arg| !arg.starts_with('-')).map(PathBuf::from);
    let named = |s: &Path| package_version(s).map_or_else(|| package(s), |(name, v)| format!("{name} {v}"));
    let name = script.as_deref().map_or_else(|| spec.name.clone(), named);
    if spec.name == "php" && version_below(node, PHP_DEBUG_NODE) {
        let (major, minor) = PHP_DEBUG_NODE;
        let text = format!("Node.js {node} is too old: vscode-php-debug needs {major}.{minor} or newer");
        return Finding::new(Status::Problem, text, Some(Fix::Shell(distro(&NODE))));
    }
    Finding::ok(format!("{name} on Node.js {node}"))
}

fn gdb(program: &Path) -> Finding {
    let version = ask(&Ask::new(program, &["--version"]))
        .and_then(|answer| Some(answer.stdout.lines().next()?.split_whitespace().last()?.to_owned()));
    let Some(version) = version else {
        return Finding::new(Status::Problem, "`gdb --version` does not answer", None);
    };
    let major: u32 = version.split('.').next().and_then(|m| m.parse().ok()).unwrap_or(0);
    if major < GDB_DAP {
        let text = format!("gdb {version} is too old: its DAP server came with gdb {GDB_DAP}");
        return Finding::new(Status::Problem, text, Some(Fix::Shell(distro(&GDB))));
    }
    // What tuitor does first: start the DAP server and ask it to initialize.
    let request = r#"{"seq": 1, "type": "request", "command": "initialize", "arguments": {"adapterID": "gdb"}}"#;
    let input = format!("Content-Length: {}\r\n\r\n{request}", request.len());
    let ask =
        Ask { input: Some(input.as_bytes()), timeout: TIMEOUT, ..Ask::new(program, &["--interpreter=dap", "--quiet"]) };
    match crate::process::ask(&ask) {
        Some(answer) if answer.stdout.contains(r#""success": true"#) => {
            Finding::ok(format!("gdb {version}, its DAP server answers"))
        }
        _ => Finding::new(
            Status::Problem,
            format!("gdb {version} does not answer as a DAP server; it is written in Python, which this gdb may lack"),
            Some(Fix::Shell("gdb -batch -ex 'python print(1)'".into())),
        ),
    }
}

/// What is missing, and what installs it.
fn not_installed(spec: &AdapterSpec, missing: &Missing) -> Finding {
    let readme = || Some(Fix::Text("README, \"Installing adapters\"".into()));
    match missing {
        Missing::Program(program) if program == "node" => {
            let text = format!("Node.js is not on PATH; {} runs on it", script_package(spec));
            Finding::new(Status::Problem, text, Some(Fix::Shell(distro(&NODE))))
        }
        Missing::Program(program) if program == "gdb" => {
            let text = "`gdb` is not on PATH: gdb 14 or newer, built with Python";
            Finding::new(Status::Problem, text, Some(Fix::Shell(distro(&GDB))))
        }
        Missing::Program(program) if program == "lldb-dap" => {
            let text = "`lldb-dap` is not on PATH; it comes with LLDB (Debian: lldb-dap-NN, to link as lldb-dap)";
            Finding::new(Status::Problem, text, Some(Fix::Shell(distro(&LLDB))))
        }
        Missing::Program(program) if program == "dlv" && resolve_program("go").is_none() => {
            let text = "`dlv` is not on PATH, nor is Go, which builds it";
            Finding::new(Status::Problem, text, Some(Fix::Shell(distro(&GO))))
        }
        Missing::Program(program) if program == "dlv" => {
            let fix = Fix::Shell("go install github.com/go-delve/delve/cmd/dlv@latest".into());
            Finding::new(Status::Problem, "`dlv` is not on PATH", Some(fix))
        }
        Missing::Program(program) if Path::new(program).starts_with(adapters_dir()) => {
            let text = format!("{} is not installed: {missing}", package(Path::new(program)));
            Finding::new(Status::Problem, text, readme())
        }
        Missing::OffPath { dir, .. } => {
            let fix = Fix::Shell(format!("export PATH=\"$PATH:{}\"", dir.display()));
            Finding::new(Status::Problem, format!("{missing}; add it in ~/.bashrc or ~/.zshrc"), Some(fix))
        }
        Missing::Script(script) if script.starts_with(adapters_dir()) => {
            let text = format!("{} is not installed: {missing}", package(script));
            Finding::new(Status::Problem, text, readme())
        }
        Missing::Program(_) | Missing::Script(_) => Finding::new(Status::Problem, missing.to_string(), None),
    }
}

/// The browser a configuration tuitor makes for `pwa-chrome` names.
fn browser() -> Finding {
    match browser_on_path() {
        Some(browser) => Finding::ok(format!("browser {}", home_relative(&browser))),
        None => Finding::new(
            Status::Problem,
            "no Chrome, Chromium or Edge on PATH: a browser configuration starts one",
            Some(Fix::Shell(distro(&CHROMIUM))),
        ),
    }
}

/// The installation an adapter file belongs to: its directory under `<adapters>`, else the
/// file's name.
fn package(file: &Path) -> String {
    let under = file.strip_prefix(adapters_dir()).ok().and_then(|rest| rest.iter().next());
    under.unwrap_or_else(|| file.file_name().unwrap_or(file.as_os_str())).to_string_lossy().into_owned()
}

fn script_package(spec: &AdapterSpec) -> String {
    spec.args.iter().find(|arg| !arg.starts_with('-')).map_or_else(|| spec.name.clone(), |s| package(Path::new(s)))
}

/// The name and version in the nearest package.json above an adapter's file, within its
/// installation: VS Code extensions have one in `extension/`.
fn package_version(file: &Path) -> Option<(String, String)> {
    let root = adapters_dir();
    let dir = file.ancestors().skip(1).take_while(|dir| dir.starts_with(&root) && *dir != root).find_map(|dir| {
        let text = std::fs::read_to_string(dir.join("package.json")).ok()?;
        serde_json::from_str::<Value>(&text).ok()
    })?;
    let version = dir.get("version")?.as_str()?.to_owned();
    Some((package(file), version))
}

fn program_name(program: &Path) -> String {
    program.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Whether `v26.8.1` is older than `major.minor`.
fn version_below(version: &str, (major, minor): (u32, u32)) -> bool {
    let mut parts = version.trim_start_matches('v').split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    let found = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    found < (major, minor)
}

// ---- configurations ---------------------------------------------------------------------

fn configuration(
    project: &ProjectInput,
    config: &Configuration,
    adapters: &Adapters,
    groups: &[Group],
    probe: &Probe,
    options: &Options,
) -> Vec<Finding> {
    let subject = config.name().to_owned();
    let about = |findings: Vec<Finding>| findings.into_iter().map(|f| f.about(&subject)).collect::<Vec<_>>();
    let vars = Variables { workspace: project.dir.clone() };
    let launch = match config.launch(&vars) {
        Ok(launch) => launch,
        Err(err) => return about(vec![Finding::new(Status::Problem, format!("{err:#}"), None)]),
    };
    let Some(spec) = adapters.get(&launch.adapter) else {
        let text = format!("no adapter for type `{}` (tuitor configs lists them)", launch.adapter);
        return about(vec![Finding::new(Status::Problem, text, None)]);
    };
    let mut findings = adapter_summary(spec, groups, &project.dir, probe, options);
    if launch.request == RequestKind::Launch {
        match launch.adapter.as_str() {
            "php" => {
                findings.extend(listen_port(&launch).map(|port| free_port(port, options)));
                findings.push(xdebug_for(&launch, options));
            }
            adapter if starts_browser(adapter) => findings.push(runtime_executable(&launch, config)),
            "gdb" | "lldb-dap" | "lldb" => findings.extend(program(&launch, &project.dir)),
            _ => {}
        }
    }
    about(findings)
}

/// The adapter's own findings, as the adapters section has them: its problems, or that it
/// is ready.
fn adapter_summary(spec: &AdapterSpec, groups: &[Group], cwd: &Path, probe: &Probe, options: &Options) -> Vec<Finding> {
    let own = groups.iter().find(|g| g.names.contains(&spec.name)).map(|g| g.findings.clone());
    // An adapter left out of the adapters section is checked here.
    let own = own.unwrap_or_else(|| adapter(spec, cwd, probe, options));
    let problems: Vec<Finding> = own.iter().filter(|f| f.status == Status::Problem).cloned().collect();
    if !problems.is_empty() {
        return problems;
    }
    let ready = own.first().map_or_else(|| spec.name.clone(), |f| f.text.clone());
    vec![Finding::ok(format!("{}: {ready}", spec.name))]
}

/// The port a configuration listens on: vscode-php-debug's for Xdebug.
pub fn listen_port(launch: &LaunchConfig) -> Option<u16> {
    if launch.adapter != "php" || launch.request != RequestKind::Launch {
        return None;
    }
    let port = launch.arguments.get("port").and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok());
    Some(port.unwrap_or(XDEBUG_PORT))
}

/// Another program on the port keeps vscode-php-debug from listening there.
fn free_port(port: u16, options: &Options) -> Finding {
    if options.own_ports.contains(&port) {
        return Finding::ok(format!("port {port}: a session of this tuitor listens on it"));
    }
    if port_taken(port) {
        let text = format!("port {port} is taken: another program, perhaps another debugger, listens on it");
        return Finding::new(Status::Problem, text, Some(Fix::Shell(format!("ss -ltnp 'sport = :{port}'"))));
    }
    Finding::ok(format!("port {port} is free"))
}

/// Something listens on `port`, on any address: binding it fails.
fn port_taken(port: u16) -> bool {
    let in_use = |address: (&str, u16)| {
        std::net::TcpListener::bind(address).is_err_and(|err| err.kind() == std::io::ErrorKind::AddrInUse)
    };
    in_use(("0.0.0.0", port)) || in_use(("::", port))
}

/// Xdebug in the PHP a configuration runs. vscode-php-debug starts the PHP of a configuration
/// with a `program` itself, which then has to have Xdebug. One that only listens is for PHP
/// that something else starts, the CLI, php-fpm or PHP in a container, which tuitor cannot see:
/// what the `php` here lacks is then a note. One that maps paths is for PHP elsewhere.
fn xdebug_for(launch: &LaunchConfig, options: &Options) -> Finding {
    let mapped = launch.arguments.get("pathMappings").and_then(Value::as_object).is_some_and(|m| !m.is_empty());
    if mapped {
        return Finding::new(Status::Note, "PHP runs elsewhere (pathMappings); its Xdebug is not checked", None);
    }
    let php = launch.arguments.get("runtimeExecutable").and_then(Value::as_str).map(str::to_owned);
    let php = php.or_else(|| options.php.clone()).unwrap_or_else(|| "php".into());
    let finding = xdebug(&php, true);
    if launch.arguments.contains_key("program") {
        return finding;
    }
    noted(finding, "PHP that something else starts (php-fpm, a container) needs Xdebug where it runs")
}

/// A problem as a note, with why it may not be one.
fn noted(finding: Finding, why: &str) -> Finding {
    match finding.status {
        Status::Problem => Finding { status: Status::Note, text: format!("{}; {why}", finding.text), ..finding },
        _ => finding,
    }
}

/// Xdebug in the PHP on PATH, for the adapters section: what PHP lacks here is a note, as
/// the PHP to debug may as well run in a container.
fn local_xdebug(options: &Options) -> Finding {
    let php = options.php.clone().unwrap_or_else(|| "php".into());
    if resolve_program(&php).is_none() {
        let text = format!("no `{php}` here to look for Xdebug in; PHP in a container needs Xdebug there");
        return Finding::new(Status::Note, text, None);
    }
    noted(xdebug(&php, false), "PHP in a container needs it there")
}

/// What the PHP at `php` says of Xdebug: loaded and debugging, loaded but in another mode,
/// installed but not loaded (Arch's package comments it out), or missing. `local`: PHP is to
/// run here.
fn xdebug(php: &str, local: bool) -> Finding {
    let Some(program) = resolve_program(php) else {
        let text = format!("`{php}` is not {}", if Path::new(php).is_absolute() { "a program" } else { "on PATH" });
        let fix = local.then(|| Fix::Text("install PHP, or set runtimeExecutable to one".into()));
        return Finding::new(Status::Problem, text, fix);
    };
    // Xdebug off, so that asking does not try to reach a debugger.
    let ask = Ask { env: &[("XDEBUG_MODE", "off")], ..Ask::new(&program, &["-r", XDEBUG_INFO]) };
    let info: Option<Value> = crate::process::ask(&ask).and_then(|a| serde_json::from_str(&a.stdout).ok());
    let Some(info) = info else {
        return Finding::new(Status::Problem, format!("{} does not answer", home_relative(&program)), None);
    };
    let field = |name: &str| info.get(name).and_then(Value::as_str).unwrap_or_default().to_owned();
    let php_name = home_relative(&program);
    let version = field("xdebug");
    if version.is_empty() {
        let scanned: Vec<PathBuf> =
            field("scanned").split(',').map(str::trim).filter(|f| !f.is_empty()).map(PathBuf::from).collect();
        return not_loaded(&php_name, Path::new(&field("ext_dir")), &scanned);
    }
    if version_below(&version, (3, 0)) {
        let text = format!("{php_name} has Xdebug {version}; vscode-php-debug needs Xdebug 3");
        return Finding::new(Status::Problem, text, Some(Fix::Shell(distro(&XDEBUG))));
    }
    let mode = field("mode");
    if !mode.split(',').any(|m| m.trim() == "debug") {
        let text = format!(
            "{php_name} has Xdebug {version} in mode `{mode}`: it debugs where XDEBUG_MODE=debug is set, \
             or with xdebug.mode=debug in its ini file"
        );
        return Finding::new(Status::Note, text, None);
    }
    Finding::ok(format!("{php_name} has Xdebug {version}"))
}

/// What PHP says of Xdebug and where it looks for it, as JSON.
const XDEBUG_INFO: &str = r#"echo json_encode(["xdebug" => phpversion("xdebug"), "ext_dir" => ini_get("extension_dir"),
    "scanned" => php_ini_scanned_files(), "mode" => ini_get("xdebug.mode")]);"#;

/// Xdebug is not loaded: installed with its ini line commented out, installed and not named
/// in any ini file, or not installed.
fn not_loaded(php: &str, ext_dir: &Path, scanned: &[PathBuf]) -> Finding {
    let library = ext_dir.join("xdebug.so");
    if !library.is_file() {
        let text = format!("{php} has no Xdebug (`{php} -v` would name it)");
        let fix = format!("{}   (or pie install xdebug/xdebug)", distro(&XDEBUG));
        return Finding::new(Status::Problem, text, Some(Fix::Shell(fix)));
    }
    let library = home_relative(&library);
    match scanned.iter().find(|ini| std::fs::read_to_string(ini).is_ok_and(|text| commented_out(&text))) {
        Some(ini) => Finding::new(
            Status::Problem,
            format!("Xdebug is installed ({library}), but {} has it commented out", ini.display()),
            Some(Fix::Shell(format!("sudo sed -i 's/^;[[:space:]]*zend_extension/zend_extension/' {}", ini.display()))),
        ),
        None => Finding::new(
            Status::Problem,
            format!("Xdebug is installed ({library}), but {php} does not load it"),
            Some(Fix::Text(format!("add zend_extension=xdebug.so to an ini file `{php} --ini` lists"))),
        ),
    }
}

/// An ini file with Xdebug's line commented out, as Arch's package installs it.
fn commented_out(ini: &str) -> bool {
    ini.lines().any(|line| {
        let Some(rest) = line.trim_start().strip_prefix(';') else { return false };
        let rest = rest.trim_start();
        rest.starts_with("zend_extension") && rest.contains("xdebug")
    })
}

/// The browser a `pwa-chrome` launch starts: `runtimeExecutable`, else Google Chrome, the
/// only one js-debug looks for.
fn runtime_executable(launch: &LaunchConfig, config: &Configuration) -> Finding {
    let set = |path: PathBuf| match &config.source {
        Source::Tuitor => Fix::Command(format!("config set runtimeExecutable {}", path.display())),
        Source::LaunchJson(_) => Fix::Text(format!("set runtimeExecutable to {} in launch.json", path.display())),
    };
    let fix = || browser_on_path().map(set).or_else(|| Some(Fix::Shell(distro(&CHROMIUM))));
    match launch.arguments.get("runtimeExecutable").and_then(Value::as_str) {
        Some(browser) => match resolve_program(browser) {
            Some(path) => Finding::ok(format!("browser {}", home_relative(&path))),
            None => Finding::new(Status::Problem, format!("runtimeExecutable {browser} is not a program here"), fix()),
        },
        None => match ["google-chrome", "google-chrome-stable"].into_iter().find_map(resolve_program) {
            Some(chrome) => Finding::ok(format!("browser {} (js-debug's default)", home_relative(&chrome))),
            None => Finding::new(
                Status::Problem,
                "no runtimeExecutable, and js-debug finds only Google Chrome, which is not on PATH",
                fix(),
            ),
        },
    }
}

/// The program a native configuration debugs: it has to be built.
fn program(launch: &LaunchConfig, dir: &Path) -> Option<Finding> {
    let program = launch.arguments.get("program").and_then(Value::as_str)?;
    let path = dir.join(program);
    (!path.is_file()).then(|| {
        Finding::new(Status::Problem, format!("{} does not exist; build it first", home_relative(&path)), None)
    })
}

// ---- distributions ------------------------------------------------------------------------

/// A package's names in the distributions tuitor can name a command for.
struct Packages {
    arch: &'static str,
    debian: &'static str,
    fedora: &'static str,
}

const NODE: Packages = Packages { arch: "nodejs", debian: "nodejs", fedora: "nodejs" };
const GDB: Packages = Packages { arch: "gdb", debian: "gdb", fedora: "gdb" };
const LLDB: Packages = Packages { arch: "lldb", debian: "lldb", fedora: "lldb" };
const GO: Packages = Packages { arch: "go", debian: "golang-go", fedora: "golang" };
const CHROMIUM: Packages = Packages { arch: "chromium", debian: "chromium", fedora: "chromium" };
const XDEBUG: Packages = Packages { arch: "xdebug", debian: "php-xdebug", fedora: "php-xdebug" };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Arch,
    Debian,
    Fedora,
}

/// The command that installs `packages` from this distribution, or each distribution's
/// command when it is not one of those. Without `sudo`, which is the user's to add.
fn distro(packages: &Packages) -> String {
    let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    match family(&release) {
        Some(Family::Arch) => format!("pacman -S {}", packages.arch),
        Some(Family::Debian) => format!("apt-get install {}", packages.debian),
        Some(Family::Fedora) => format!("dnf install {}", packages.fedora),
        None => format!(
            "pacman -S {} · apt-get install {} · dnf install {}",
            packages.arch, packages.debian, packages.fedora
        ),
    }
}

/// The family of the distribution `/etc/os-release` describes, by its `ID` and `ID_LIKE`.
fn family(os_release: &str) -> Option<Family> {
    let ids: Vec<&str> = os_release
        .lines()
        .filter_map(|line| line.strip_prefix("ID=").or_else(|| line.strip_prefix("ID_LIKE=")))
        .flat_map(|value| value.trim_matches('"').split_whitespace())
        .collect();
    let has = |names: &[&str]| ids.iter().any(|id| names.contains(id));
    if has(&["arch"]) {
        Some(Family::Arch)
    } else if has(&["debian", "ubuntu"]) {
        Some(Family::Debian)
    } else if has(&["fedora", "rhel"]) {
        Some(Family::Fedora)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distributions_are_told_apart_by_their_id_and_what_they_are_like() {
        assert_eq!(family("NAME=\"Arch Linux\"\nID=arch\n"), Some(Family::Arch));
        assert_eq!(family("ID=ubuntu\nID_LIKE=debian\n"), Some(Family::Debian));
        assert_eq!(family("ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n"), Some(Family::Fedora));
        assert_eq!(family("ID=endeavouros\nID_LIKE=arch\n"), Some(Family::Arch));
        assert_eq!(family("ID=nixos\n"), None);
    }

    #[test]
    fn versions_compare_by_major_and_minor() {
        assert!(version_below("v18.20.4", (20, 18)));
        assert!(version_below("v20.17.0", (20, 18)));
        assert!(!version_below("v20.18.0", (20, 18)));
        assert!(!version_below("v26.8.1", (20, 18)));
        assert!(version_below("2.9.8", (3, 0)));
        assert!(!version_below("3.5.3", (3, 0)));
    }

    #[test]
    fn a_commented_xdebug_line_is_found() {
        assert!(commented_out("; Xdebug\n;zend_extension=xdebug.so\n"));
        assert!(commented_out("  ; zend_extension = /usr/lib/php/modules/xdebug.so"));
        assert!(!commented_out("zend_extension=xdebug.so\n;xdebug.mode=debug\n"));
        assert!(!commented_out(";zend_extension=opcache.so\n"));
    }

    #[test]
    fn js_debug_types_are_checked_once_per_target() {
        let adapters = Adapters::default();
        let names: Vec<Vec<&str>> =
            groups(&adapters).iter().map(|g| g.iter().map(|s| s.name.as_str()).collect()).collect();
        assert!(names.contains(&vec!["pwa-chrome", "chrome"]), "{names:?}");
        assert!(names.contains(&vec!["pwa-node", "node"]), "{names:?}");
        assert!(names.contains(&vec!["php"]));
        assert_eq!(names.iter().map(Vec::len).sum::<usize>(), adapters.names().count());
    }

    #[test]
    fn only_named_adapters_are_needed() {
        let adapters = Adapters::default();
        let only = |names: &[&str]| wanted_types(&adapters, &names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>());
        assert_eq!(only(&[]).ok(), Some(vec![]));
        assert_eq!(only(&["js-debug"]).ok(), Some(vec!["pwa-node".into(), "pwa-chrome".into()]));
        assert_eq!(only(&["codelldb", "gdb"]).ok(), Some(vec!["lldb".into(), "gdb".into()]));
        let unknown = only(&["frobnicate"]).expect_err("no such adapter");
        assert!(unknown.text.starts_with("no adapter `frobnicate`"));
    }

    #[test]
    fn a_listening_port_is_taken() {
        let listener = std::net::TcpListener::bind(("0.0.0.0", 0)).expect("bind");
        let port = listener.local_addr().expect("address").port();
        assert!(port_taken(port));
        drop(listener);
        assert!(!port_taken(port));
    }

    #[test]
    fn configurations_report_what_their_adapter_lacks() {
        let dir = std::env::temp_dir().join(format!("tuitor-doctor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let adapters_toml = dir.join("adapters.toml");
        std::fs::write(&adapters_toml, "[gdb]\ncommand = \"tuitor-no-such-gdb\"\n").expect("adapters.toml");
        let adapters = Adapters::load(Some(&adapters_toml)).expect("adapters");
        let fields = serde_json::json!({
            "name": "orders", "type": "gdb", "request": "launch", "program": "${workspaceFolder}/orders",
        });
        let Value::Object(fields) = fields else { unreachable!() };
        let configurations = vec![Configuration { source: Source::Tuitor, fields }];
        let project = ProjectInput { dir: dir.clone(), configurations, errors: Vec::new() };
        let options = Options { only: vec!["gdb".into()], ..Options::default() };
        let report = check(&adapters, std::slice::from_ref(&project), &options);
        let findings = &report.sections[1].findings;
        assert!(findings.iter().all(|f| f.subject == "orders"));
        let texts: Vec<&str> = findings.iter().map(|f| f.text.as_str()).collect();
        assert_eq!(texts[0], "`tuitor-no-such-gdb` is not on PATH");
        assert!(texts[1].ends_with("/orders does not exist; build it first"), "{texts:?}");
        // The adapter was named, and the project needs it.
        assert_eq!(report.blocking(), 3);
        std::fs::remove_dir_all(dir).ok();
    }
}
