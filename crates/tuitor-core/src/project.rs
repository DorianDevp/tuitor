//! A project is a directory tuitor debugs in: what kind of project it is, told by
//! marker files, and its launch configurations. Those come from `.vscode/launch.json`,
//! which tuitor only reads, and from tuitor's own [`Store`], which it edits.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use crate::config::{LaunchConfig, Variables, load_launch_json};
use crate::store::Store;

/// Kinds of project tuitor recognises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Rust,
    Go,
    Php,
    Js,
    C,
}

impl Kind {
    /// In order of precedence: a project gets a default configuration for the first kind
    /// it is. A Makefile is common in projects of every language, so C comes last.
    pub const ALL: [Kind; 5] = [Kind::Rust, Kind::Go, Kind::Php, Kind::Js, Kind::C];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Rust => "rust",
            Kind::Go => "go",
            Kind::Php => "php",
            Kind::Js => "js",
            Kind::C => "c",
        }
    }

    pub fn parse(name: &str) -> Option<Kind> {
        Some(match name {
            "rust" | "rs" => Kind::Rust,
            "go" => Kind::Go,
            "php" => Kind::Php,
            "js" | "ts" => Kind::Js,
            "c" => Kind::C,
            _ => return None,
        })
    }

    /// The launch `type` of the configuration tuitor creates for this kind.
    pub fn adapter(self) -> &'static str {
        match self {
            Kind::Rust | Kind::C => "gdb",
            Kind::Go => "go",
            Kind::Php => "php",
            Kind::Js => "pwa-chrome",
        }
    }

    fn files(self) -> &'static [&'static str] {
        match self {
            Kind::Rust => &["Cargo.toml"],
            // Without one, the directory does not build as a package, which is what the
            // configuration this kind gets launches.
            Kind::Go => &["go.mod"],
            Kind::Php => &["composer.json"],
            Kind::Js => &["package.json", "angular.json", "tsconfig.json"],
            Kind::C => &["Makefile", "makefile", "GNUmakefile", "CMakeLists.txt"],
        }
    }

    fn extensions(self) -> &'static [&'static str] {
        match self {
            Kind::Php => &["php"],
            Kind::C => &["c"],
            Kind::Rust | Kind::Go | Kind::Js => &[],
        }
    }

    fn marks(self, file: &str) -> bool {
        let extension = Path::new(file).extension().and_then(|e| e.to_str());
        self.files().contains(&file) || extension.is_some_and(|e| self.extensions().contains(&e))
    }
}

/// The kinds of project `dir` is, from the files directly in it, in order of precedence.
pub fn detect(dir: &Path) -> Vec<Kind> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let files: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    Kind::ALL.into_iter().filter(|kind| files.iter().any(|file| kind.marks(file))).collect()
}

/// A configuration to start from for launch type `adapter` in `dir`. Native programs
/// are guessed; the user corrects what the guess got wrong in CONFIG.
pub fn template(adapter: &str, dir: &Path) -> Map<String, Value> {
    let workspace = "${workspaceFolder}";
    let fields = match adapter {
        "php" => json!({ "name": "Listen for Xdebug", "port": 9003 }),
        // As VS Code's Go extension writes it, with the mode it resolves its own `auto` to:
        // dlv builds the package in the project directory and runs it.
        "go" => json!({ "name": "Launch Package", "mode": "debug", "program": workspace }),
        "gdb" | "lldb-dap" | "lldb" => {
            let program = native_program(dir);
            let name =
                Path::new(&program).file_name().map_or_else(|| program.clone(), |n| n.to_string_lossy().into_owned());
            json!({ "name": name, "program": format!("{workspace}/{program}"), "cwd": workspace })
        }
        "pwa-chrome" | "chrome" => {
            let browser = std::env::var_os("PATH").and_then(|path| browser_in(&path));
            browser_launch(workspace, browser)
        }
        "pwa-node" => {
            json!({ "name": "Launch Node", "program": format!("{workspace}/{}", node_main(dir)), "cwd": workspace })
        }
        other => json!({ "name": other }),
    };
    let Value::Object(mut fields) = fields else { return Map::new() };
    fields.insert("type".into(), adapter.into());
    fields.insert("request".into(), "launch".into());
    fields
}

/// Launch types that start a browser, through js-debug.
pub fn starts_browser(adapter: &str) -> bool {
    matches!(adapter, "pwa-chrome" | "chrome")
}

/// Browsers js-debug can debug, by the names their packages put on PATH, in the order a
/// new configuration takes them: Google Chrome, then Chromium as distributions name it,
/// then Edge.
const BROWSERS: [&str; 6] = [
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
    "microsoft-edge",
    "microsoft-edge-stable",
];

/// The first of [`BROWSERS`] in a directory of `path`, a value of PATH.
fn browser_in(path: &OsStr) -> Option<PathBuf> {
    let dirs: Vec<PathBuf> = std::env::split_paths(path).collect();
    BROWSERS.iter().find_map(|name| dirs.iter().map(|dir| dir.join(name)).find(|file| file.is_file()))
}

/// The page `ng serve` serves, in `browser`, with sources under `workspace`. Without
/// `runtimeExecutable` js-debug looks for Google Chrome only, so a machine with just
/// Chromium needs it named.
fn browser_launch(workspace: &str, browser: Option<PathBuf>) -> Value {
    let mut fields = json!({ "name": "Launch Chrome", "url": "http://localhost:4200", "webRoot": workspace });
    if let Some(browser) = browser {
        fields["runtimeExecutable"] = browser.display().to_string().into();
    }
    fields
}

/// The program a native project builds, relative to `dir`: Cargo's debug binary, an
/// executable already built in `dir`, or one named like the directory.
fn native_program(dir: &Path) -> String {
    if let Some(package) = cargo_package(dir) {
        return format!("target/debug/{package}");
    }
    let mut built: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| is_binary(&entry.path()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    built.sort();
    built.into_iter().next().unwrap_or_else(|| dir_name(dir))
}

fn cargo_package(dir: &Path) -> Option<String> {
    let manifest: toml::Table = toml::from_str(&std::fs::read_to_string(dir.join("Cargo.toml")).ok()?).ok()?;
    Some(manifest.get("package")?.get("name")?.as_str()?.to_owned())
}

/// An executable file that is not a script.
#[cfg(unix)]
fn is_binary(path: &Path) -> bool {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    let executable = std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    let mut start = [0u8; 2];
    let script = std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut start)).is_ok() && start == *b"#!";
    executable && !script
}

#[cfg(not(unix))]
fn is_binary(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "exe")
}

/// `main` of package.json, else Node's own default.
fn node_main(dir: &Path) -> String {
    let package: Option<Value> =
        std::fs::read_to_string(dir.join("package.json")).ok().and_then(|text| serde_json::from_str(&text).ok());
    package.and_then(|p| p.get("main")?.as_str().map(str::to_owned)).unwrap_or_else(|| "index.js".to_owned())
}

fn dir_name(dir: &Path) -> String {
    dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "program".to_owned())
}

/// Where a configuration is defined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// `.vscode/launch.json` (or `--launch-json`); tuitor only reads it.
    LaunchJson(PathBuf),
    /// tuitor's own, in the [`Store`].
    Tuitor,
}

#[derive(Debug, Clone)]
pub struct Configuration {
    pub source: Source,
    /// launch.json's have their variables expanded as they are read, as before; tuitor's
    /// own are kept as stored, which is what CONFIG shows and edits, and expanded at start.
    pub fields: Map<String, Value>,
}

impl Configuration {
    pub fn name(&self) -> &str {
        self.fields.get("name").and_then(Value::as_str).unwrap_or("unnamed")
    }

    pub fn adapter(&self) -> &str {
        self.fields.get("type").and_then(Value::as_str).unwrap_or("?")
    }

    pub fn is_own(&self) -> bool {
        self.source == Source::Tuitor
    }

    /// What the engine starts; its arguments go to the adapter verbatim.
    pub fn launch(&self, vars: &Variables) -> Result<LaunchConfig> {
        let mut value = Value::Object(self.fields.clone());
        if self.is_own() {
            vars.expand_value(&mut value);
        }
        LaunchConfig::from_value(value)
    }
}

pub struct Project {
    pub dir: PathBuf,
    pub kinds: Vec<Kind>,
    /// launch.json's first, then tuitor's own.
    pub configurations: Vec<Configuration>,
    /// Why launch.json or the store could not be read.
    pub errors: Vec<String>,
    /// The store has an entry for this project, possibly empty. A broken store counts as
    /// known, so that no default is created over configurations that could not be read.
    known: bool,
}

impl Project {
    /// Reads what tuitor knows about `dir`; problems end up in `errors`.
    pub fn open(dir: &Path, store: &Store, launch_json: Option<&Path>) -> Project {
        let dir = dir.to_path_buf();
        let launch_json = launch_json.map_or_else(|| dir.join(".vscode/launch.json"), Path::to_path_buf);
        let vars = Variables { workspace: dir.clone() };
        let mut configurations = Vec::new();
        let mut errors = Vec::new();
        if launch_json.exists() {
            match load_launch_json(&launch_json, &vars) {
                Ok(configs) => configurations.extend(configs.into_iter().map(|config| Configuration {
                    source: Source::LaunchJson(launch_json.clone()),
                    fields: config.arguments,
                })),
                Err(err) => errors.push(format!("{err:#}")),
            }
        }
        let known = match store.load(&dir) {
            Ok(Some(own)) => {
                configurations.extend(own.into_iter().map(|fields| Configuration { source: Source::Tuitor, fields }));
                true
            }
            Ok(None) => false,
            Err(err) => {
                errors.push(format!("{err:#}"));
                true
            }
        };
        Project { kinds: detect(&dir), dir, configurations, errors, known }
    }

    pub fn name(&self) -> String {
        dir_name(&self.dir)
    }

    pub fn variables(&self) -> Variables {
        Variables { workspace: self.dir.clone() }
    }

    /// Something tuitor can debug, or has been told about: worth a tab of its own.
    pub fn is_project(&self) -> bool {
        self.known || !self.kinds.is_empty() || !self.configurations.is_empty()
    }

    /// Gives a project with no configuration anywhere one for the kind it is, kept by
    /// tuitor. Returns its index.
    pub fn create_default(&mut self, store: &Store) -> Result<Option<usize>> {
        if self.known || !self.configurations.is_empty() {
            return Ok(None);
        }
        let Some(kind) = self.kinds.first() else { return Ok(None) };
        self.add(store, kind.adapter()).map(Some)
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        let wanted = name.to_lowercase();
        let names = || self.configurations.iter().map(|c| c.name().to_lowercase());
        names().position(|n| n == wanted).or_else(|| names().position(|n| n.contains(&wanted)))
    }

    // ---- editing tuitor's own -----------------------------------------------------------

    /// Adds a configuration of launch type `adapter` (or of the kind it names: `rust`, `js`…).
    /// Returns its index.
    pub fn add(&mut self, store: &Store, adapter: &str) -> Result<usize> {
        let adapter = match Kind::parse(adapter) {
            Some(kind) => kind.adapter(),
            None => adapter,
        };
        let mut fields = template(adapter, &self.dir);
        let base = fields.get("name").and_then(Value::as_str).unwrap_or(adapter).to_owned();
        let mut name = base.clone();
        for n in 2.. {
            if self.find_exact(&name).is_none() {
                break;
            }
            name = format!("{base} {n}");
        }
        fields.insert("name".into(), name.into());
        let mut own = self.own();
        own.push(fields.clone());
        store.save(&self.dir, &own)?;
        self.known = true;
        self.configurations.push(Configuration { source: Source::Tuitor, fields });
        Ok(self.configurations.len() - 1)
    }

    pub fn delete(&mut self, store: &Store, index: usize) -> Result<Configuration> {
        self.editable(index)?;
        let mut configurations = self.configurations.clone();
        let removed = configurations.remove(index);
        store.save(&self.dir, &own(&configurations))?;
        self.configurations = configurations;
        Ok(removed)
    }

    pub fn set(&mut self, store: &Store, index: usize, field: &str, value: Value) -> Result<()> {
        if value.is_null() {
            bail!("`null` is no value; /config unset {field} removes the field");
        }
        self.edit(store, index, |fields| {
            fields.insert(field.to_owned(), value);
            Ok(())
        })
    }

    pub fn unset(&mut self, store: &Store, index: usize, field: &str) -> Result<()> {
        self.edit(store, index, |fields| fields.remove(field).map(drop).ok_or_else(|| anyhow!("no field `{field}`")))
    }

    /// Maps `remote` (a path the adapter sees) to `local` in `pathMappings`; an empty
    /// `local` removes the mapping. A relative `local` is taken from the project directory.
    pub fn map(&mut self, store: &Store, index: usize, remote: &str, local: &str) -> Result<()> {
        let local = (!local.is_empty()).then(|| {
            let path = self.dir.join(local);
            std::fs::canonicalize(&path).unwrap_or(path).display().to_string()
        });
        self.edit(store, index, |fields| {
            let mut mappings = match fields.remove("pathMappings") {
                Some(Value::Object(map)) => map,
                _ => Map::new(),
            };
            match local {
                Some(local) => {
                    mappings.insert(remote.to_owned(), local.into());
                }
                None => {
                    mappings.remove(remote).ok_or_else(|| anyhow!("no mapping for {remote}"))?;
                }
            }
            if !mappings.is_empty() {
                fields.insert("pathMappings".into(), Value::Object(mappings));
            }
            Ok(())
        })
    }

    /// Changes one of tuitor's own configurations, checks the result and saves it; on any
    /// error nothing changes.
    fn edit(
        &mut self,
        store: &Store,
        index: usize,
        change: impl FnOnce(&mut Map<String, Value>) -> Result<()>,
    ) -> Result<()> {
        self.editable(index)?;
        let mut configurations = self.configurations.clone();
        change(&mut configurations[index].fields)?;
        let edited = &configurations[index];
        edited.launch(&self.variables())?;
        let name = edited.fields.get("name").and_then(Value::as_str).unwrap_or_default();
        if name.trim().is_empty() {
            bail!("a configuration needs a `name`");
        }
        if self.find_exact(name).is_some_and(|other| other != index) {
            bail!("there is already a configuration named `{name}`");
        }
        store.save(&self.dir, &own(&configurations))?;
        self.configurations = configurations;
        Ok(())
    }

    fn editable(&self, index: usize) -> Result<()> {
        let config = self.configurations.get(index).ok_or_else(|| anyhow!("no configuration selected"))?;
        match &config.source {
            Source::Tuitor => Ok(()),
            Source::LaunchJson(path) => bail!(
                "`{}` is defined in {}, which tuitor only reads; /config add makes one of tuitor's own",
                config.name(),
                path.strip_prefix(&self.dir).unwrap_or(path).display()
            ),
        }
    }

    fn find_exact(&self, name: &str) -> Option<usize> {
        self.configurations.iter().position(|c| c.name().eq_ignore_ascii_case(name))
    }

    fn own(&self) -> Vec<Map<String, Value>> {
        own(&self.configurations)
    }
}

fn own(configurations: &[Configuration]) -> Vec<Map<String, Value>> {
    configurations.iter().filter(|c| c.is_own()).map(|c| c.fields.clone()).collect()
}

/// A value typed after `/config set field`: JSON when it reads as JSON (`9003`, `true`,
/// `["--verbose"]`, `"text"`), otherwise the text itself.
pub fn parse_value(text: &str) -> Value {
    serde_json::from_str(text.trim()).unwrap_or_else(|_| Value::String(text.trim().to_owned()))
}

/// The inverse of [`parse_value`], for the palette pre-filled with a field's value.
pub fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) if parse_value(text) == *value => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tuitor-project-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("tmp dir");
        dir
    }

    fn touch(dir: &Path, files: &[&str]) {
        for file in files {
            std::fs::write(dir.join(file), "").expect("write");
        }
    }

    #[test]
    fn detects_kinds_from_marker_files_in_precedence_order() {
        let dir = temp("detect");
        assert_eq!(detect(&dir), []);
        touch(&dir, &["Makefile", "index.php"]);
        assert_eq!(detect(&dir), [Kind::Php, Kind::C]);
        touch(&dir, &["angular.json"]);
        assert_eq!(detect(&dir), [Kind::Php, Kind::Js, Kind::C]);
        std::fs::create_dir(dir.join("Cargo.toml")).expect("a directory is no marker");
        assert!(!detect(&dir).contains(&Kind::Rust));

        let rust = temp("detect-rust");
        touch(&rust, &["Cargo.toml"]);
        assert_eq!(detect(&rust), [Kind::Rust]);
        let c = temp("detect-c");
        touch(&c, &["orders.c"]);
        assert_eq!(detect(&c), [Kind::C]);
        let go = temp("detect-go");
        touch(&go, &["main.go", "Makefile"]);
        assert_eq!(detect(&go), [Kind::C], "no go.mod, no Go project");
        touch(&go, &["go.mod", "package.json"]);
        assert_eq!(detect(&go), [Kind::Go, Kind::Js, Kind::C]);
        for dir in [dir, rust, c, go] {
            std::fs::remove_dir_all(dir).ok();
        }
    }

    #[test]
    fn templates_guess_what_to_run() {
        let dir = temp("template");
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"ledger\"\n").expect("manifest");
        let native = template("gdb", &dir);
        assert_eq!(native["program"], "${workspaceFolder}/target/debug/ledger");
        assert_eq!(native["name"], "ledger");
        assert_eq!(native["request"], "launch");

        let php = template("php", &dir);
        assert_eq!((php["type"].as_str(), php["port"].as_u64()), (Some("php"), Some(9003)));
        let go = template(Kind::Go.adapter(), &dir);
        assert_eq!((go["type"].as_str(), go["request"].as_str()), (Some("go"), Some("launch")));
        assert_eq!((go["mode"].as_str(), go["program"].as_str()), (Some("debug"), Some("${workspaceFolder}")));
        let chrome = template(Kind::Js.adapter(), &dir);
        assert_eq!(chrome["url"], "http://localhost:4200");
        assert_eq!(chrome["webRoot"], "${workspaceFolder}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_browser_configuration_names_the_browser_found_on_path() {
        let dir = temp("browsers");
        let (first, second) = (dir.join("first"), dir.join("second"));
        std::fs::create_dir_all(&first).expect("first");
        std::fs::create_dir_all(&second).expect("second");
        let path = std::env::join_paths([&first, &second]).expect("PATH");
        assert_eq!(browser_in(&path), None);
        // Chrome before Chromium, wherever on PATH each is.
        touch(&first, &["chromium"]);
        assert_eq!(browser_in(&path), Some(first.join("chromium")));
        touch(&second, &["google-chrome"]);
        assert_eq!(browser_in(&path), Some(second.join("google-chrome")));

        let chromium = browser_launch("/w", Some(PathBuf::from("/usr/bin/chromium")));
        assert_eq!(chromium["runtimeExecutable"], "/usr/bin/chromium");
        assert!(browser_launch("/w", None).get("runtimeExecutable").is_none(), "js-debug looks for Chrome itself");
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn native_programs_are_built_executables_not_scripts() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("binary");
        for (file, text) in [("a-run.sh", "#!/bin/sh\n"), ("orders", "\x7fELF")] {
            std::fs::write(dir.join(file), text).expect("write");
            std::fs::set_permissions(dir.join(file), std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        assert_eq!(template("gdb", &dir)["program"], "${workspaceFolder}/orders");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_new_project_gets_one_default_configuration_kept_by_tuitor() {
        let dir = temp("default");
        let store = Store::new(dir.join("config/projects.toml"));
        let project_dir = dir.join("api");
        std::fs::create_dir_all(&project_dir).expect("project");
        touch(&project_dir, &["composer.json"]);

        let mut project = Project::open(&project_dir, &store, None);
        assert!(project.is_project());
        assert_eq!(project.create_default(&store).expect("create"), Some(0));
        assert_eq!(project.configurations[0].name(), "Listen for Xdebug");
        assert_eq!(project.create_default(&store).expect("again"), None);

        let reopened = Project::open(&project_dir, &store, None);
        assert_eq!(reopened.configurations.len(), 1);
        assert!(reopened.configurations[0].is_own());
        let launch = reopened.configurations[0].launch(&reopened.variables()).expect("launch");
        assert_eq!(launch.adapter, "php");
        assert_eq!(launch.arguments["port"], 9003);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn edits_are_checked_and_saved() {
        let dir = temp("edit");
        let store = Store::new(dir.join("projects.toml"));
        let mut project = Project::open(&dir, &store, None);
        let index = project.add(&store, "php").expect("add");
        project.set(&store, index, "port", parse_value("9010")).expect("set port");
        project.map(&store, index, "/var/www/app", ".").expect("map");
        assert!(project.unset(&store, index, "type").is_err(), "a configuration needs a type");
        assert!(project.set(&store, index, "request", parse_value("debug")).is_err());
        assert_eq!(project.add(&store, "php").expect("second"), 1);
        assert_eq!(project.configurations[1].name(), "Listen for Xdebug 2");
        assert!(project.set(&store, 1, "name", parse_value("listen for xdebug")).is_err(), "names are unique");

        let saved = Project::open(&dir, &store, None);
        let fields = &saved.configurations[index].fields;
        assert_eq!(fields["port"], 9010);
        assert_eq!(fields["type"], "php");
        let local = std::fs::canonicalize(&dir).expect("dir").display().to_string();
        assert_eq!(fields["pathMappings"]["/var/www/app"], local.as_str());

        project.map(&store, index, "/var/www/app", "").expect("unmap");
        assert!(!project.configurations[index].fields.contains_key("pathMappings"));
        project.delete(&store, 1).expect("delete");
        assert_eq!(Project::open(&dir, &store, None).configurations.len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn launch_json_is_read_only_and_listed_first() {
        let dir = temp("launch-json");
        std::fs::create_dir_all(dir.join(".vscode")).expect("dir");
        std::fs::write(
            dir.join(".vscode/launch.json"),
            r#"{ "configurations": [{ "name": "orders", "type": "gdb", "request": "launch", "cwd": "${workspaceFolder}" }] }"#,
        )
        .expect("launch.json");
        let store = Store::new(dir.join("projects.toml"));
        let mut project = Project::open(&dir, &store, None);
        assert_eq!(project.create_default(&store).expect("nothing to create"), None);
        assert!(project.set(&store, 0, "cwd", parse_value("/tmp")).is_err());
        project.add(&store, "rust").expect("add");
        assert!(matches!(project.configurations[0].source, Source::LaunchJson(_)));
        assert_eq!(project.configurations[0].fields["cwd"], dir.display().to_string());
        assert_eq!(project.configurations[1].adapter(), "gdb");
        assert_eq!(project.find("ORD"), Some(0));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn typed_values_round_trip() {
        assert_eq!(parse_value("9003"), json!(9003));
        assert_eq!(parse_value(" true "), json!(true));
        assert_eq!(parse_value(r#"["--verbose", "-x"]"#), json!(["--verbose", "-x"]));
        assert_eq!(parse_value("http://localhost:4200"), json!("http://localhost:4200"));
        assert_eq!(parse_value(r#""9003""#), json!("9003"));
        for value in [json!("9003"), json!("./build/app"), json!(42), json!({ "A": "1" }), json!("say \"hi\"")] {
            assert_eq!(parse_value(&value_text(&value)), value);
        }
        assert_eq!(value_text(&json!("${workspaceFolder}/orders")), "${workspaceFolder}/orders");
    }
}
