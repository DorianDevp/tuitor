//! tuitor's own launch configurations, kept outside the repositories they are for:
//! one file, `~/.config/tuitor/projects.toml`, with a table per project keyed by its
//! absolute path. One file is one place to read, back up or carry to another machine;
//! every write re-reads it and replaces only the project being changed, so two tuitors
//! editing different projects do not undo each other.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const HEADER: &str = "\
# Launch configurations tuitor keeps for projects, outside their repositories, by
# absolute project path. Fields are those of .vscode/launch.json. tuitor rewrites
# this file when /config changes a configuration, so comments here are not kept.

";

#[derive(Debug, Default, Deserialize)]
struct Entry {
    #[serde(default)]
    configurations: Vec<Map<String, Value>>,
}

#[derive(Serialize)]
struct Written<'a> {
    configurations: Vec<Ordered<'a>>,
}

/// A configuration in launch.json's order: what it is first, then the rest.
#[derive(Serialize)]
struct Ordered<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a Value>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    adapter: Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request: Option<&'a Value>,
    #[serde(flatten)]
    rest: BTreeMap<&'a String, &'a Value>,
}

impl<'a> Ordered<'a> {
    fn new(fields: &'a Map<String, Value>) -> Self {
        let rest = fields.iter().filter(|(key, _)| !matches!(key.as_str(), "name" | "type" | "request")).collect();
        Ordered { name: fields.get("name"), adapter: fields.get("type"), request: fields.get("request"), rest }
    }
}

#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(path: PathBuf) -> Store {
        Store { path }
    }

    /// `projects.toml` in tuitor's config directory (on Linux `$XDG_CONFIG_HOME/tuitor`).
    pub fn default_path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("tuitor").join("projects.toml"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The configurations kept for `project`, or `None` when tuitor keeps none for it.
    /// An empty list is kept on purpose: the user deleted the last one.
    pub fn load(&self, project: &Path) -> Result<Option<Vec<Map<String, Value>>>> {
        Ok(self.read()?.remove(&key(project)).map(|entry| entry.configurations))
    }

    /// Replaces `project`'s configurations; other projects keep what the file has now.
    pub fn save(&self, project: &Path, configurations: &[Map<String, Value>]) -> Result<()> {
        let mut file = self.read()?;
        file.insert(key(project), Entry { configurations: configurations.iter().map(without_nulls).collect() });
        let file: BTreeMap<&String, Written<'_>> = file
            .iter()
            .map(|(project, entry)| {
                (project, Written { configurations: entry.configurations.iter().map(Ordered::new).collect() })
            })
            .collect();
        let text = toml::to_string(&file).with_context(|| format!("writing {}", self.path.display()))?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let tmp = self.path.with_extension("toml.tmp");
        std::fs::write(&tmp, format!("{HEADER}{text}")).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path).with_context(|| format!("replacing {}", self.path.display()))
    }

    fn read(&self) -> Result<BTreeMap<String, Entry>> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(err) => return Err(err).with_context(|| format!("reading {}", self.path.display())),
        };
        // A file that does not parse is never overwritten: saving fails with this error too.
        toml::from_str(&text)
            .with_context(|| format!("parsing {} (fix it; tuitor leaves it as it is)", self.path.display()))
    }
}

fn key(project: &Path) -> String {
    project.display().to_string()
}

/// TOML has no null; a null field means the field is not set.
fn without_nulls(fields: &Map<String, Value>) -> Map<String, Value> {
    fn clean(value: &Value) -> Option<Value> {
        match value {
            Value::Null => None,
            Value::Array(items) => Some(Value::Array(items.iter().filter_map(clean).collect())),
            Value::Object(map) => {
                Some(Value::Object(map.iter().filter_map(|(k, v)| Some((k.clone(), clean(v)?))).collect()))
            }
            other => Some(other.clone()),
        }
    }
    fields.iter().filter_map(|(k, v)| Some((k.clone(), clean(v)?))).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn fields(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            _ => Map::new(),
        }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tuitor-store-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn round_trips_configurations_per_project() {
        let dir = temp("round-trip");
        let store = Store::new(dir.join("projects.toml"));
        let api = Path::new("/home/me/shop/api");
        let web = Path::new("/home/me/shop/web");
        assert!(store.load(api).expect("missing file is fine").is_none());

        let listen = fields(json!({
            "name": "Listen for Xdebug", "type": "php", "request": "launch", "port": 9003,
            "pathMappings": { "/var/www/app": "/home/me/shop/api" },
            "ignore": ["**/vendor/**"], "log": null,
        }));
        let chrome = fields(json!({ "name": "Chrome", "type": "pwa-chrome", "url": "http://localhost:4200" }));
        store.save(api, std::slice::from_ref(&listen)).expect("save api");
        store.save(web, std::slice::from_ref(&chrome)).expect("save web");

        let loaded = store.load(api).expect("load").expect("api is known");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0]["port"], 9003);
        assert_eq!(loaded[0]["pathMappings"]["/var/www/app"], "/home/me/shop/api");
        assert_eq!(loaded[0]["ignore"], json!(["**/vendor/**"]));
        assert!(!loaded[0].contains_key("log"), "nulls are dropped");
        assert_eq!(store.load(web).expect("load").expect("web is known")[0]["url"], "http://localhost:4200");

        // Deleting the last configuration is remembered, not taken for a new project.
        store.save(web, &[]).expect("save empty");
        assert_eq!(store.load(web).expect("load").map(|c| c.len()), Some(0));
        assert_eq!(store.load(api).expect("load").map(|c| c.len()), Some(1));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_broken_file_is_reported_and_left_alone() {
        let dir = temp("broken");
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("projects.toml");
        std::fs::write(&path, "[\"/x\"\nname = ").expect("write");
        let store = Store::new(path.clone());
        assert!(store.load(Path::new("/x")).is_err());
        assert!(store.save(Path::new("/y"), &[]).is_err());
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "[\"/x\"\nname = ");
        std::fs::remove_dir_all(dir).ok();
    }
}
