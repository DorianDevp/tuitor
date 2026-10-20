//! Debug adapters known to tuitor and launch configurations to start them with.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::jsonc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransportKind {
    /// DAP on the adapter's stdin/stdout.
    Stdio,
    /// The adapter listens on a port we choose (`${port}` in `args`).
    Server,
    /// Connect to `address`; nothing is spawned.
    Tcp,
}

/// How to run one debug adapter. Keyed by the launch config `type`.
#[derive(Debug, Clone, Deserialize)]
pub struct AdapterSpec {
    #[serde(skip)]
    pub name: String,
    #[serde(default = "default_transport")]
    pub transport: TransportKind,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub address: Option<String>,
    /// Source file extensions this adapter debugs; breakpoints in them go to its sessions.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Short label shown next to the session name.
    #[serde(default)]
    pub tag: String,
    /// `adapterID` sent in `initialize`; defaults to the adapter name.
    #[serde(default)]
    pub adapter_id: Option<String>,
    /// `type` sent in the launch/attach arguments instead of the configuration's own.
    #[serde(default)]
    pub launch_type: Option<String>,
    /// Where the official installation puts `command` when that is not on PATH. Only
    /// named when the program is missing, never run from: what runs is what PATH finds.
    #[serde(skip)]
    pub install_dir: Option<PathBuf>,
}

fn default_transport() -> TransportKind {
    TransportKind::Stdio
}

const NATIVE: &[&str] = &["rs", "c", "h", "cc", "cpp", "cxx", "hpp", "zig"];
const JS: &[&str] = &["js", "mjs", "cjs", "ts", "mts", "cts", "jsx", "tsx"];

impl AdapterSpec {
    fn builtin(name: &str, transport: TransportKind, command: &str, args: &[&str], exts: &[&str], tag: &str) -> Self {
        Self {
            name: name.to_owned(),
            transport,
            command: command.to_owned(),
            args: args.iter().map(|s| (*s).to_owned()).collect(),
            env: BTreeMap::new(),
            address: None,
            extensions: exts.iter().map(|s| (*s).to_owned()).collect(),
            tag: tag.to_owned(),
            adapter_id: None,
            launch_type: None,
            install_dir: None,
        }
    }

    /// js-debug's standalone server, for one of the debug types its VS Code extension
    /// registers. It knows only the `pwa-` types: VS Code's own resolver turns `node` and
    /// `chrome` configurations into those before they reach it, so tuitor does the same.
    fn js_debug(name: &str, server: &str, launch_type: &str) -> Self {
        // An explicit host: on its default, `localhost`, Node may listen on ::1 only.
        let args = [server, "${port}", "127.0.0.1"];
        let mut spec = Self::builtin(name, TransportKind::Server, "node", &args, JS, "js");
        spec.launch_type = (name != launch_type).then(|| launch_type.to_owned());
        spec
    }

    /// Delve's DAP server, for the `"type": "go"` of VS Code's Go extension. It builds the
    /// program itself (`"mode": "debug"`). `go install` puts dlv in Go's own bin directory,
    /// which is often not on PATH, so a dlv found there is named when PATH has none.
    fn delve() -> Self {
        let args = ["dap", "--listen", "127.0.0.1:${port}"];
        let mut spec = Self::builtin("go", TransportKind::Server, "dlv", &args, &["go"], "go");
        spec.install_dir = go_bin_dir(|name| std::env::var_os(name), dirs::home_dir());
        spec
    }

    pub fn handles(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| self.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)))
    }

    pub fn adapter_id(&self) -> &str {
        self.adapter_id.as_deref().unwrap_or(&self.name)
    }

    /// The `launch` or `attach` arguments for `config`: its fields, changed where VS Code's
    /// extensions change them before their adapters see them.
    pub fn launch_arguments(&self, config: &LaunchConfig) -> Map<String, Value> {
        let mut arguments = config.arguments.clone();
        if let Some(launch_type) = &self.launch_type {
            arguments.insert("type".into(), Value::String(launch_type.clone()));
        }
        if config.adapter == "go" {
            resolve_go(&mut arguments);
        }
        arguments
    }

    /// The program to run, once everything the adapter needs is known to be installed.
    /// Checking the script up front matters: `node` with a missing script exits with a
    /// stack trace that hides the cause. Relative scripts resolve against `cwd`, where
    /// the adapter runs.
    pub fn locate(&self, cwd: &Path) -> Result<PathBuf, Missing> {
        let program = resolve_program(&self.command).ok_or_else(|| self.missing_program())?;
        match self.script(cwd) {
            Some(script) if !script.is_file() => Err(Missing::Script(script)),
            _ => Ok(program),
        }
    }

    fn missing_program(&self) -> Missing {
        match &self.install_dir {
            Some(dir) if dir.join(&self.command).is_file() => {
                Missing::OffPath { program: self.command.clone(), dir: dir.clone() }
            }
            _ => Missing::Program(self.command.clone()),
        }
    }

    /// The script a Node-based adapter runs: its first argument that is not an option.
    fn script(&self, cwd: &Path) -> Option<PathBuf> {
        if Path::new(&self.command).file_stem()? != "node" {
            return None;
        }
        self.args.iter().find(|arg| !arg.starts_with('-')).map(|arg| cwd.join(arg))
    }
}

/// What VS Code's Go extension does to a `go` configuration before dlv gets it, done as
/// far as tuitor can:
///
/// - dlv knows no `"mode": "auto"`, which the extension's snippets write. The extension
///   makes it `test` for a `_test.go` file and `debug` for anything else, and runs a test
///   file's package. It looks at the file open in the editor too; tuitor has none.
/// - dlv's program writes to dlv's own stdout, which the extension shows as the program's
///   output. tuitor keeps an adapter's own lines apart, so it asks dlv to send the
///   program's output as output events (`"outputMode": "remote"`), unless the
///   configuration sends it elsewhere.
fn resolve_go(arguments: &mut Map<String, Value>) {
    let program = arguments.get("program").and_then(Value::as_str).unwrap_or_default().to_owned();
    let test_file = program.ends_with("_test.go");
    if let Some(mode) = arguments.get_mut("mode").filter(|mode| **mode == "auto") {
        *mode = if test_file { "test" } else { "debug" }.into();
    }
    if test_file && arguments.get("mode").is_some_and(|mode| mode == "test") {
        let package = Path::new(&program).parent().map(|dir| dir.display().to_string()).unwrap_or_default();
        arguments.insert("program".into(), package.into());
    }
    if !["outputMode", "stdoutTo", "stderrTo"].iter().any(|key| arguments.contains_key(*key)) {
        arguments.insert("outputMode".into(), "remote".into());
    }
}

/// Something an adapter needs that is not installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Missing {
    /// The command is neither an existing file nor a program on PATH.
    Program(String),
    /// The program is installed in `dir`, which is not on PATH.
    OffPath { program: String, dir: PathBuf },
    /// The interpreter is there; the script it should run is not.
    Script(PathBuf),
}

impl fmt::Display for Missing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Missing::Program(program) if Path::new(program).components().count() > 1 => {
                write!(f, "{} does not exist", home_relative(Path::new(program)))
            }
            Missing::Program(program) => write!(f, "`{program}` is not on PATH"),
            Missing::OffPath { program, dir } => {
                write!(f, "`{program}` is in {}, which is not on PATH", home_relative(dir))
            }
            Missing::Script(script) => write!(f, "{} does not exist", home_relative(script)),
        }
    }
}

/// Where to send someone whose adapter is missing, in tuitor.
pub const INSTALL_HELP: &str = "/doctor says what is missing and what installs it";

/// `~/…` for paths under the home directory: shorter on a one-line status, and still
/// something a shell understands.
pub fn home_relative(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// Where `go install` puts programs, from the environment (`var`): `$GOBIN`, else `bin`
/// in the first directory of `$GOPATH`, which is `~/go` when unset. Settings made with
/// `go env -w` are not read.
fn go_bin_dir(var: impl Fn(&str) -> Option<OsString>, home: Option<PathBuf>) -> Option<PathBuf> {
    let set = |name| var(name).filter(|value| !value.is_empty());
    if let Some(bin) = set("GOBIN") {
        return Some(PathBuf::from(bin));
    }
    let gopath = set("GOPATH").and_then(|paths| std::env::split_paths(&paths).next());
    Some(gopath.or_else(|| home.map(|home| home.join("go")))?.join("bin"))
}

/// Adapters that are published as VS Code extensions are unpacked from the official
/// `.vsix` (js-debug: the standalone DAP server's `.tar.gz`) of their GitHub release into
/// `<data dir>/tuitor/adapters/<name>` (on Linux `$XDG_DATA_HOME`, by default
/// `~/.local/share`). No package manager is involved, so what runs is exactly what the
/// adapter's authors released.
pub fn adapters_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_default().join("tuitor").join("adapters")
}

/// Built-in adapters, overridable from `~/.config/tuitor/adapters.toml`:
///
/// ```toml
/// [php]
/// command = "node"
/// args = ["/opt/vscode-php-debug/extension/out/phpDebug.js"]
/// ```
#[derive(Debug, Clone)]
pub struct Adapters {
    specs: BTreeMap<String, AdapterSpec>,
}

impl Default for Adapters {
    fn default() -> Self {
        use TransportKind::*;
        let vsix = adapters_dir();
        let php_debug = vsix.join("vscode-php-debug/extension/out/phpDebug.js").display().to_string();
        let codelldb = vsix.join("codelldb/extension/adapter/codelldb").display().to_string();
        // The release archive holds a `js-debug` directory; it is unpacked as it is.
        let js_debug = vsix.join("js-debug/js-debug/src/dapDebugServer.js").display().to_string();
        let specs = [
            AdapterSpec::builtin("php", Stdio, "node", &[&php_debug], &["php", "phtml"], "php"),
            AdapterSpec::builtin("gdb", Stdio, "gdb", &["--interpreter=dap", "--quiet"], NATIVE, "native"),
            AdapterSpec::builtin("lldb-dap", Stdio, "lldb-dap", &[], NATIVE, "native"),
            // CodeLLDB's VS Code configs use `"type": "lldb"`.
            AdapterSpec::builtin("lldb", Server, &codelldb, &["--port", "${port}"], NATIVE, "native"),
            AdapterSpec::js_debug("pwa-node", &js_debug, "pwa-node"),
            AdapterSpec::js_debug("node", &js_debug, "pwa-node"),
            AdapterSpec::js_debug("pwa-chrome", &js_debug, "pwa-chrome"),
            AdapterSpec::js_debug("chrome", &js_debug, "pwa-chrome"),
            AdapterSpec::delve(),
        ];
        Self { specs: specs.into_iter().map(|s| (s.name.clone(), s)).collect() }
    }
}

impl Adapters {
    /// Built-ins with the user's file merged on top (a table replaces the built-in entry).
    pub fn load(user_file: Option<&Path>) -> Result<Self> {
        let mut adapters = Self::default();
        let Some(path) = user_file.filter(|p| p.exists()) else {
            return Ok(adapters);
        };
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let user: BTreeMap<String, AdapterSpec> =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        for (name, mut spec) in user {
            spec.name = name.clone();
            if spec.tag.is_empty() {
                spec.tag = name.clone();
            }
            adapters.specs.insert(name, spec);
        }
        Ok(adapters)
    }

    pub fn get(&self, name: &str) -> Option<&AdapterSpec> {
        self.specs.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.specs.keys().map(String::as_str)
    }

    pub fn all(&self) -> impl Iterator<Item = &AdapterSpec> {
        self.specs.values()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Launch,
    Attach,
}

/// One entry of `configurations` in launch.json, with variables already expanded.
#[derive(Debug, Clone)]
pub struct LaunchConfig {
    pub name: String,
    pub adapter: String,
    pub request: RequestKind,
    /// The whole object; adapters receive it verbatim as launch/attach arguments.
    pub arguments: Map<String, Value>,
}

impl LaunchConfig {
    pub fn from_value(value: Value) -> Result<Self> {
        let Value::Object(arguments) = value else {
            bail!("a configuration must be an object");
        };
        let field = |key: &str| arguments.get(key).and_then(Value::as_str).map(str::to_owned);
        let name = field("name").unwrap_or_else(|| "unnamed".to_owned());
        let Some(adapter) = field("type") else {
            bail!("configuration `{name}` has no `type`");
        };
        let request = match field("request").as_deref() {
            Some("attach") => RequestKind::Attach,
            Some("launch") | None => RequestKind::Launch,
            Some(other) => bail!("configuration `{name}`: unknown request `{other}`"),
        };
        Ok(Self { name, adapter, request, arguments })
    }

    /// Ad-hoc config built from the command line.
    pub fn adhoc(name: &str, adapter: &str, request: RequestKind, extra: Map<String, Value>) -> Self {
        let mut arguments = extra;
        arguments.insert("name".into(), name.into());
        arguments.insert("type".into(), adapter.into());
        let request_name = match request {
            RequestKind::Launch => "launch",
            RequestKind::Attach => "attach",
        };
        arguments.insert("request".into(), request_name.into());
        Self { name: name.to_owned(), adapter: adapter.to_owned(), request, arguments }
    }
}

/// Values for `${...}` placeholders in launch.json.
#[derive(Debug, Clone)]
pub struct Variables {
    pub workspace: PathBuf,
}

impl Variables {
    fn lookup(&self, name: &str) -> Option<String> {
        if let Some(var) = name.strip_prefix("env:") {
            return Some(std::env::var(var).unwrap_or_default());
        }
        let home = || dirs::home_dir().map(|h| h.display().to_string()).unwrap_or_default();
        Some(match name {
            "workspaceFolder" | "workspaceRoot" | "cwd" => self.workspace.display().to_string(),
            "workspaceFolderBasename" => {
                self.workspace.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            }
            "userHome" => home(),
            "pathSeparator" => std::path::MAIN_SEPARATOR.to_string(),
            _ => return None,
        })
    }

    pub fn expand(&self, input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut rest = input;
        while let Some(start) = rest.find("${") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(end) = after.find('}') else {
                out.push_str(&rest[start..]);
                return out;
            };
            match self.lookup(&after[..end]) {
                Some(value) => out.push_str(&value),
                None => out.push_str(&rest[start..start + 2 + end + 1]),
            }
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        out
    }

    pub fn expand_value(&self, value: &mut Value) {
        match value {
            Value::String(s) => *s = self.expand(s),
            Value::Array(items) => items.iter_mut().for_each(|v| self.expand_value(v)),
            Value::Object(map) => map.values_mut().for_each(|v| self.expand_value(v)),
            _ => {}
        }
    }
}

/// Reads `configurations` from a VS Code style launch.json.
pub fn load_launch_json(path: &Path, vars: &Variables) -> Result<Vec<LaunchConfig>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut doc: Value =
        serde_json::from_str(&jsonc::strip(&text)).with_context(|| format!("parsing {}", path.display()))?;
    vars.expand_value(&mut doc);
    let Some(Value::Array(configs)) = doc.get_mut("configurations").map(Value::take) else {
        bail!("{} has no `configurations` array", path.display());
    };
    configs.into_iter().map(LaunchConfig::from_value).collect()
}

/// Finds an executable: a path is taken as given, a bare name is looked up on PATH.
/// Nowhere else, so the adapter that runs is the one the user installed.
pub fn resolve_program(program: &str) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(program))
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn vars() -> Variables {
        Variables { workspace: PathBuf::from("/home/me/shop") }
    }

    #[test]
    fn expands_known_variables_and_keeps_unknown() {
        let v = vars();
        assert_eq!(v.expand("${workspaceFolder}/api"), "/home/me/shop/api");
        assert_eq!(v.expand("${workspaceFolderBasename}-${unknown}"), "shop-${unknown}");
        assert_eq!(v.expand("tail ${"), "tail ${");
    }

    #[test]
    fn loads_configurations_with_comments() {
        let dir = std::env::temp_dir().join(format!("tuitor-launch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        let file = dir.join("launch.json");
        std::fs::write(
            &file,
            r#"{
              // generated by VS Code
              "version": "0.2.0",
              "configurations": [
                {
                  "name": "Listen for Xdebug",
                  "type": "php",
                  "request": "launch",
                  "port": 9003,
                  "pathMappings": { "/var/www/app": "${workspaceFolder}/api" },
                },
              ],
            }"#,
        )
        .expect("write");
        let configs = load_launch_json(&file, &vars()).expect("load");
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].adapter, "php");
        assert_eq!(configs[0].request, RequestKind::Launch);
        assert_eq!(configs[0].arguments["pathMappings"]["/var/www/app"], "/home/me/shop/api");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn node_adapters_need_their_script() {
        let dir = std::env::temp_dir().join(format!("tuitor-locate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        let node = dir.join("node");
        std::fs::write(&node, "").expect("stand-in for node");
        let command = node.display().to_string();
        let mut spec =
            AdapterSpec::builtin("js", TransportKind::Stdio, &command, &["--no-warnings", "dap.js"], &[], "");
        assert_eq!(spec.locate(&dir), Err(Missing::Script(dir.join("dap.js"))));
        std::fs::write(dir.join("dap.js"), "").expect("script");
        assert_eq!(spec.locate(&dir), Ok(node));

        spec.command = "tuitor-no-such-adapter".into();
        let missing = spec.locate(&dir).expect_err("not on PATH");
        assert_eq!(missing.to_string(), "`tuitor-no-such-adapter` is not on PATH");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn vsix_adapters_are_found_in_the_data_dir() {
        let adapters = Adapters::default();
        let php = adapters.get("php").expect("php");
        assert_eq!(php.command, "node");
        assert_eq!(Path::new(&php.args[0]), adapters_dir().join("vscode-php-debug/extension/out/phpDebug.js"));
        let codelldb = adapters.get("lldb").expect("lldb");
        assert_eq!(Path::new(&codelldb.command), adapters_dir().join("codelldb/extension/adapter/codelldb"));
    }

    #[test]
    fn js_debug_serves_every_javascript_type() {
        let adapters = Adapters::default();
        let server = adapters_dir().join("js-debug/js-debug/src/dapDebugServer.js");
        for (name, launch_type) in
            [("pwa-node", None), ("node", Some("pwa-node")), ("pwa-chrome", None), ("chrome", Some("pwa-chrome"))]
        {
            let spec = adapters.get(name).expect(name);
            assert_eq!(spec.transport, TransportKind::Server);
            assert_eq!((spec.command.as_str(), Path::new(&spec.args[0])), ("node", server.as_path()));
            assert_eq!(spec.args[1..], ["${port}", "127.0.0.1"]);
            assert_eq!(spec.launch_type.as_deref(), launch_type, "{name}");
            assert_eq!(spec.tag, "js");
            for file in ["app.js", "a.mjs", "a.cjs", "app.ts", "a.mts", "a.cts", "App.jsx", "App.tsx"] {
                assert!(spec.handles(Path::new(file)), "{name} should handle {file}");
            }
        }
    }

    #[test]
    fn delve_serves_go() {
        let adapters = Adapters::default();
        let go = adapters.get("go").expect("go");
        assert_eq!((go.transport, go.command.as_str()), (TransportKind::Server, "dlv"));
        assert_eq!(go.args, ["dap", "--listen", "127.0.0.1:${port}"]);
        assert_eq!((go.tag.as_str(), go.launch_type.as_deref()), ("go", None));
        assert!(go.handles(Path::new("/src/orders/main.go")));
        assert!(!go.handles(Path::new("/src/orders/go.mod")));
    }

    #[test]
    fn go_configurations_are_resolved_for_dlv() {
        let adapters = Adapters::default();
        let go = adapters.get("go").expect("go");
        let resolved = |fields: Value| {
            let Value::Object(fields) = fields else { unreachable!() };
            let config = LaunchConfig::adhoc("pkg", "go", RequestKind::Launch, fields);
            Value::Object(go.launch_arguments(&config))
        };
        let package = resolved(json!({ "mode": "auto", "program": "/src/orders" }));
        assert_eq!((package["mode"].as_str(), package["program"].as_str()), (Some("debug"), Some("/src/orders")));
        assert_eq!(package["type"], "go");
        let test = resolved(json!({ "mode": "auto", "program": "/src/orders/orders_test.go" }));
        assert_eq!((test["mode"].as_str(), test["program"].as_str()), (Some("test"), Some("/src/orders")));
        let exec = resolved(json!({ "mode": "exec", "program": "/src/orders/orders" }));
        assert_eq!((exec["mode"].as_str(), exec["program"].as_str()), (Some("exec"), Some("/src/orders/orders")));
        // The program's output comes as output events, unless it is sent elsewhere.
        assert_eq!(package["outputMode"], "remote");
        assert_eq!(resolved(json!({ "outputMode": "local" }))["outputMode"], "local");
        assert!(resolved(json!({ "stdoutTo": "/tmp/out" })).get("outputMode").is_none());
        let chrome = adapters.get("chrome").expect("chrome");
        let config = LaunchConfig::adhoc("ng", "chrome", RequestKind::Launch, Map::new());
        assert_eq!(chrome.launch_arguments(&config)["type"], "pwa-chrome");
    }

    #[test]
    fn a_program_off_path_is_named_where_it_is() {
        let dir = std::env::temp_dir().join(format!("tuitor-off-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        let mut spec = AdapterSpec::builtin("go", TransportKind::Server, "tuitor-no-such-dlv", &[], &[], "go");
        spec.install_dir = Some(dir.clone());
        assert_eq!(spec.locate(&dir), Err(Missing::Program("tuitor-no-such-dlv".into())));
        std::fs::write(dir.join("tuitor-no-such-dlv"), "").expect("stand-in for dlv");
        let missing = spec.locate(&dir).expect_err("still not on PATH");
        assert_eq!(missing, Missing::OffPath { program: "tuitor-no-such-dlv".into(), dir: dir.clone() });
        assert_eq!(
            missing.to_string(),
            format!("`tuitor-no-such-dlv` is in {}, which is not on PATH", home_relative(&dir))
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn go_installs_into_gobin_or_the_first_gopath() {
        let home = Some(PathBuf::from("/home/me"));
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |name: &str| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| OsString::from(v))
        };
        assert_eq!(go_bin_dir(env(&[]), home.clone()), Some(PathBuf::from("/home/me/go/bin")));
        assert_eq!(go_bin_dir(env(&[("GOBIN", "")]), home.clone()), Some(PathBuf::from("/home/me/go/bin")));
        let gopath = env(&[("GOPATH", "/opt/go:/home/me/go")]);
        assert_eq!(go_bin_dir(gopath, home.clone()), Some(PathBuf::from("/opt/go/bin")));
        let gobin = env(&[("GOPATH", "/opt/go"), ("GOBIN", "/usr/local/bin")]);
        assert_eq!(go_bin_dir(gobin, home), Some(PathBuf::from("/usr/local/bin")));
        assert_eq!(go_bin_dir(env(&[]), None), None);
    }

    #[test]
    fn user_adapters_override_builtins() {
        let dir = std::env::temp_dir().join(format!("tuitor-adapters-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        let file = dir.join("adapters.toml");
        std::fs::write(&file, "[php]\ncommand = \"node\"\nargs = [\"phpDebug.js\"]\nextensions = [\"php\"]\n")
            .expect("write");
        let adapters = Adapters::load(Some(&file)).expect("load");
        let php = adapters.get("php").expect("php");
        assert_eq!(php.command, "node");
        assert_eq!(php.tag, "php");
        assert!(php.handles(Path::new("/x/OrderService.php")));
        assert!(adapters.get("gdb").is_some());
        std::fs::remove_dir_all(dir).ok();
    }
}
