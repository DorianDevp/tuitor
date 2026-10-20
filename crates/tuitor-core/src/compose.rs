//! `pathMappings` from docker compose: the bind mounts of a service, each a path in the
//! container and the path here it shows. `docker compose config` says what compose itself
//! makes of the project (override files, `.env`, variables, relative paths); without docker,
//! tuitor reads the file and its override the way compose does, as far as mounts go, and
//! leaves what it does not read (`include`, `extends`, `COMPOSE_FILE`) to docker compose.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Map, Value};
use yaml_rust2::{Yaml, YamlLoader};

use crate::config::resolve_program;
use crate::process::Ask;

/// The files docker compose reads, in the order it prefers them, and the overrides it takes
/// whatever the file is called; of each, the first there is.
const FILES: [&str; 4] = ["compose.yaml", "compose.yml", "docker-compose.yml", "docker-compose.yaml"];
const OVERRIDES: [&str; 4] =
    ["compose.override.yml", "compose.override.yaml", "docker-compose.override.yml", "docker-compose.override.yaml"];
/// Entries looked at to tell whether a directory holds code, before giving up and taking it
/// that it may.
const LOOK: usize = 20_000;

/// The compose file of `dir`: in it, or in the nearest directory above that has one, as
/// docker compose looks for it.
pub fn find(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().find_map(|dir| FILES.iter().map(|file| dir.join(file)).find(|path| path.is_file()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reader {
    Docker,
    Tuitor,
}

/// A project as compose sees it.
#[derive(Debug, Clone)]
pub struct Compose {
    pub file: PathBuf,
    pub read_by: Reader,
    /// Why docker compose did not read it, when tuitor then did.
    pub docker_error: Option<String>,
    pub services: Vec<Service>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub name: String,
    pub image: Option<String>,
    /// The build context, here.
    pub build: Option<PathBuf>,
    pub dockerfile: Option<String>,
    pub mounts: Vec<Mount>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mount {
    /// A path here, at `target` in the container.
    Bind { source: PathBuf, target: String },
    /// A named or anonymous volume, a tmpfs: nothing that is here.
    Other { kind: String, target: String },
}

/// Reads the project of compose file `file`: with docker compose when `docker` and it can,
/// else here.
pub fn load(file: &Path, docker: bool) -> Result<Compose> {
    let dir = file.parent().context("a compose file is in a directory")?;
    let from_docker = if docker { docker_config(dir) } else { Err("not asked".to_owned()) };
    let (value, read_by, docker_error) = match from_docker {
        Ok(value) => (value, Reader::Docker, None),
        Err(err) => {
            let env = |name: &str| std::env::var(name).ok();
            (read_files(file, &env)?, Reader::Tuitor, docker.then_some(err))
        }
    };
    Ok(Compose { file: file.to_path_buf(), read_by, docker_error, services: services(&value, dir)? })
}

/// `docker compose config --format json`, run where the compose file is.
fn docker_config(dir: &Path) -> Result<Value, String> {
    let docker = resolve_program("docker").ok_or("docker is not on PATH")?;
    let ask = Ask {
        cwd: Some(dir),
        timeout: Duration::from_secs(30),
        ..Ask::new(&docker, &["compose", "config", "--format", "json"])
    };
    let answer = crate::process::ask(&ask).ok_or("docker compose config did not answer")?;
    if !answer.success() {
        let said = answer.stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("it failed");
        return Err(format!("docker compose config: {}", said.trim()));
    }
    serde_json::from_str(&answer.stdout).map_err(|err| format!("docker compose config: {err}"))
}

/// The compose file and its override, each with `${VAR}` replaced from `env`, else from the
/// `.env` beside the file, then merged, as compose does. What would have compose read other
/// files is refused rather than read wrong.
pub fn read_files(file: &Path, env: &dyn Fn(&str) -> Option<String>) -> Result<Value> {
    let dir = file.parent().context("a compose file is in a directory")?;
    let dotenv = std::fs::read_to_string(dir.join(".env")).map(|text| dotenv(&text)).unwrap_or_default();
    let lookup = |name: &str| env(name).or_else(|| dotenv.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()));
    if lookup("COMPOSE_FILE").is_some() {
        bail!("COMPOSE_FILE names the files of this project, which tuitor reads only through docker compose");
    }
    let mut value = read_yaml(file, &lookup)?;
    if let Some(over) = OVERRIDES.iter().map(|o| dir.join(o)).find(|p| p.is_file()) {
        // An override with nothing in it changes nothing.
        let over = read_yaml(&over, &lookup)?;
        if !over.is_null() {
            merge(&mut value, over, &[]);
        }
    }
    let services = value.get("services").and_then(Value::as_object);
    if value.get("include").is_some() || services.is_some_and(|s| s.values().any(|s| s.get("extends").is_some())) {
        bail!("{} uses include or extends, which tuitor reads only through docker compose", file.display());
    }
    Ok(value)
}

fn read_yaml(file: &Path, env: &dyn Fn(&str) -> Option<String>) -> Result<Value> {
    let text = std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let docs = YamlLoader::load_from_str(&text).with_context(|| format!("parsing {}", file.display()))?;
    let mut value = docs.first().map_or(Value::Null, json);
    interpolate(&mut value, env).with_context(|| format!("in {}", file.display()))?;
    Ok(value)
}

/// A YAML node as JSON, with merge keys (`<<: *defaults`) applied.
fn json(yaml: &Yaml) -> Value {
    match yaml {
        Yaml::Hash(hash) => {
            let mut map = Map::new();
            let mut merged = Vec::new();
            for (key, value) in hash {
                match key_text(key) {
                    Some(key) if key == "<<" => merged.push(json(value)),
                    Some(key) => {
                        map.insert(key, json(value));
                    }
                    None => {}
                }
            }
            // Keys of the mapping itself win over those merged in, and earlier merges over later.
            for base in merged.into_iter().flat_map(|m| if let Value::Array(all) = m { all } else { vec![m] }) {
                if let Value::Object(base) = base {
                    for (key, value) in base {
                        map.entry(key).or_insert(value);
                    }
                }
            }
            Value::Object(map)
        }
        Yaml::Array(items) => Value::Array(items.iter().map(json).collect()),
        Yaml::String(text) | Yaml::Real(text) => Value::String(text.clone()),
        Yaml::Integer(n) => Value::from(*n),
        Yaml::Boolean(b) => Value::Bool(*b),
        Yaml::Null | Yaml::Alias(_) | Yaml::BadValue => Value::Null,
    }
}

fn key_text(key: &Yaml) -> Option<String> {
    match key {
        Yaml::String(text) | Yaml::Real(text) => Some(text.clone()),
        Yaml::Integer(n) => Some(n.to_string()),
        Yaml::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Merges an override into a compose file as compose does, as far as mounts go: mappings key by
/// key, a service's volumes by their path in the container, other lists replaced.
fn merge(base: &mut Value, over: Value, path: &[&str]) {
    match (base, over) {
        (Value::Object(base), Value::Object(over)) => {
            for (key, value) in over {
                let mut inner = path.to_vec();
                inner.push(&key);
                match base.get_mut(&key) {
                    Some(slot) => merge(slot, value, &inner),
                    None => {
                        base.insert(key.clone(), value);
                    }
                }
            }
        }
        (Value::Array(base), Value::Array(over)) if matches!(path, ["services", _, "volumes"]) => {
            for volume in over {
                let target = mount(&volume, Path::new("")).map(|m| m.target().to_owned());
                base.retain(|old| mount(old, Path::new("")).map(|m| m.target().to_owned()) != target);
                base.push(volume);
            }
        }
        (slot, over) => *slot = over,
    }
}

/// `KEY=value` lines of a `.env` file; `#` starts a comment, quotes are taken off.
fn dotenv(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (key, value) = line.split_once('=')?;
            let value = value.trim();
            let unquoted = ['"', '\''].iter().find_map(|q| value.strip_prefix(*q)?.strip_suffix(*q));
            let value = unquoted.unwrap_or_else(|| value.split(" #").next().unwrap_or(value).trim());
            Some((key.trim().to_owned(), value.to_owned()))
        })
        .collect()
}

/// Replaces `${VAR}`, `${VAR:-default}`, `${VAR-default}`, `${VAR:?error}`, `${VAR:+other}`
/// and `$VAR` in every string, `$$` by `$`, as compose does.
fn interpolate(value: &mut Value, env: &dyn Fn(&str) -> Option<String>) -> Result<()> {
    match value {
        Value::String(text) => *text = substitute(text, env)?,
        Value::Array(items) => items.iter_mut().try_for_each(|item| interpolate(item, env))?,
        Value::Object(map) => map.values_mut().try_for_each(|item| interpolate(item, env))?,
        _ => {}
    }
    Ok(())
}

fn substitute(text: &str, env: &dyn Fn(&str) -> Option<String>) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if let Some(after) = after.strip_prefix('$') {
            out.push('$');
            rest = after;
        } else if let Some(braced) = after.strip_prefix('{') {
            let end = closing(braced).ok_or_else(|| anyhow!("unclosed ${{ in `{text}`"))?;
            out.push_str(&expand(&braced[..end], env)?);
            rest = &braced[end + 1..];
        } else {
            let end = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
            if end == 0 {
                out.push('$');
            } else {
                out.push_str(&env(&after[..end]).unwrap_or_default());
            }
            rest = &after[end..];
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// Where the `}` that closes a `${` is in `text`, the rest after it, past nested `${…}`.
fn closing(text: &str) -> Option<usize> {
    let mut depth = 1;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        match c {
            '$' if chars.peek().is_some_and(|(_, next)| *next == '{') => {
                chars.next();
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => {}
        }
    }
    None
}

/// The inside of `${…}`.
fn expand(inner: &str, env: &dyn Fn(&str) -> Option<String>) -> Result<String> {
    let name_end = inner.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(inner.len());
    let (name, op) = inner.split_at(name_end);
    let value = env(name);
    let set = value.as_deref().is_some_and(|v| !v.is_empty());
    let nested = |word: &str| substitute(word, env);
    Ok(match op {
        "" => value.unwrap_or_default(),
        _ if op.starts_with(":-") => {
            if set {
                value.unwrap_or_default()
            } else {
                nested(&op[2..])?
            }
        }
        _ if op.starts_with('-') => value.map_or_else(|| nested(&op[1..]), Ok)?,
        _ if op.starts_with(":+") => {
            if set {
                nested(&op[2..])?
            } else {
                String::new()
            }
        }
        _ if op.starts_with('+') => {
            if value.is_some() {
                nested(&op[1..])?
            } else {
                String::new()
            }
        }
        _ if op.starts_with(":?") && !set => bail!("{name}: {}", &op[2..]),
        _ if op.starts_with('?') && value.is_none() => bail!("{name}: {}", &op[1..]),
        _ if op.starts_with(":?") || op.starts_with('?') => value.unwrap_or_default(),
        _ => bail!("cannot read ${{{inner}}}"),
    })
}

// ---- services --------------------------------------------------------------------------

/// The services of a compose project as docker compose prints it or as the files say, with
/// relative paths taken from `dir`, the project's directory.
fn services(value: &Value, dir: &Path) -> Result<Vec<Service>> {
    let services = value.get("services").and_then(Value::as_object).context("no `services`")?;
    Ok(services
        .iter()
        .map(|(name, service)| {
            let image = service.get("image").and_then(Value::as_str).map(str::to_owned);
            let (build, dockerfile) = match service.get("build") {
                Some(Value::String(context)) => (Some(here(context, dir)), None),
                Some(build) => (
                    build.get("context").and_then(Value::as_str).map(|c| here(c, dir)),
                    build.get("dockerfile").and_then(Value::as_str).map(str::to_owned),
                ),
                None => (None, None),
            };
            let volumes = service.get("volumes").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
            let mounts = volumes.iter().filter_map(|volume| mount(volume, dir)).collect();
            Service { name: name.clone(), image, build, dockerfile, mounts }
        })
        .collect())
}

impl Mount {
    pub fn target(&self) -> &str {
        match self {
            Mount::Bind { target, .. } | Mount::Other { target, .. } => target,
        }
    }
}

/// A volume of a service: long syntax (`type`, `source`, `target`) or short
/// (`[source:]target[:mode]`), where a source that is a path is a bind mount.
fn mount(volume: &Value, dir: &Path) -> Option<Mount> {
    let (kind, source, target) = match volume {
        Value::Object(long) => {
            let field = |name: &str| long.get(name).and_then(Value::as_str).map(str::to_owned);
            (field("type").unwrap_or_else(|| "volume".into()), field("source"), field("target")?)
        }
        Value::String(short) => {
            let parts: Vec<&str> = short.split(':').collect();
            let (source, target) = match parts.as_slice() {
                [target] => (None, *target),
                [target, mode] if is_mode(mode) => (None, *target),
                [source, target, ..] => (Some(*source), *target),
                [] => return None,
            };
            let is_path = source.is_some_and(|s| s.starts_with(['.', '/', '~']));
            let kind = if is_path { "bind" } else { "volume" };
            (kind.to_owned(), source.map(str::to_owned), target.to_owned())
        }
        _ => return None,
    };
    let target = container_path(&target);
    Some(match (kind.as_str(), source) {
        ("bind", Some(source)) => Mount::Bind { source: here(&source, dir), target },
        (_, _) => Mount::Other { kind, target },
    })
}

/// The options that can end a short volume.
fn is_mode(text: &str) -> bool {
    text.split(',').all(|o| matches!(o, "ro" | "rw" | "z" | "Z" | "cached" | "delegated" | "consistent" | "nocopy"))
}

/// A path here as compose takes it: `~` is the home directory, a relative path is from the
/// project's directory; `.` and `..` resolved.
fn here(path: &str, dir: &Path) -> PathBuf {
    let path = match path.strip_prefix('~') {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest.trim_start_matches('/')),
        None => dir.join(path),
    };
    clean(&path)
}

fn clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// A path in the container, without a trailing slash.
fn container_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() { "/".to_owned() } else { trimmed.to_owned() }
}

/// The service to map: the one named; else the one that runs PHP, when `php`; else the only one
/// with bind mounts. Otherwise why there is no telling which.
pub fn pick<'a>(compose: &'a Compose, name: Option<&str>, php: bool) -> Result<&'a Service, String> {
    let names = |services: &[&Service]| services.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ");
    let all: Vec<&Service> = compose.services.iter().collect();
    if let Some(name) = name {
        return all
            .iter()
            .find(|s| s.name == name)
            .copied()
            .ok_or_else(|| format!("{} has no service `{name}` (it has {})", file_name(&compose.file), names(&all)));
    }
    let mounted: Vec<&Service> =
        all.iter().filter(|s| s.mounts.iter().any(|m| matches!(m, Mount::Bind { .. }))).copied().collect();
    let candidates: Vec<&Service> =
        if php { mounted.iter().filter(|s| runs_php(s)).copied().collect() } else { mounted.clone() };
    match candidates.as_slice() {
        [one] => Ok(one),
        [] if mounted.is_empty() => Err(format!("no service of {} mounts a path here", file_name(&compose.file))),
        [] => Err(format!("no service looks like PHP; which of {}?", names(&mounted))),
        several => Err(format!("{} each could be it; which?", names(several))),
    }
}

/// A service whose name, image or Dockerfile says PHP (`php:8.4-fpm`, `dunglas/frankenphp`).
fn runs_php(service: &Service) -> bool {
    let php = |text: &str| text.to_lowercase().contains("php");
    let dockerfile =
        service.build.as_ref().map(|context| context.join(service.dockerfile.as_deref().unwrap_or("Dockerfile")));
    let from = dockerfile.and_then(|file| std::fs::read_to_string(file).ok()).is_some_and(|text| {
        text.lines().any(|line| line.trim_start().to_lowercase().starts_with("from ") && php(line))
    });
    php(&service.name) || service.image.as_deref().is_some_and(php) || from
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

// ---- mappings --------------------------------------------------------------------------

/// What the mounts of a service map.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mapped {
    /// Path in the container → path here, root mappings first.
    pub mappings: Vec<(String, PathBuf)>,
    /// How many mounts each root mapping stands for, by its path in the container.
    pub collapsed: Vec<(String, usize)>,
    /// Mounts left out, and why.
    pub skipped: Vec<String>,
}

/// One mapping per bind mount of a directory; mounts that share a root here and in the
/// container (`./api/src:/app/src`, `./api/config:/app/config`) become one mapping of the
/// roots when that maps nothing else: when whatever else holds code under the root here
/// (`is_code` tells code files) is mounted where it is. A vendor directory that is not
/// mounted, say, is the container's own, and mapping the root would show this one's instead.
pub fn mappings(service: &Service, is_code: &dyn Fn(&Path) -> bool) -> Mapped {
    let mut mapped = Mapped::default();
    let mut binds: Vec<Bind> = Vec::new();
    for mount in &service.mounts {
        match mount {
            Mount::Bind { source, target } if source.exists() && !source.is_dir() => {
                mapped.skipped.push(format!("{target} (a file, not a directory)"));
            }
            Mount::Bind { source, target } => {
                binds.retain(|bind| bind.there != *target);
                binds.push(Bind { here: source.clone(), there: target.clone() });
            }
            Mount::Other { kind, target } => mapped.skipped.push(format!("{target} ({kind})")),
        }
    }
    while let Some(root) = collapsible(&binds, is_code) {
        let there = root.there.display().to_string();
        mapped.collapsed.push((there.clone(), root.mounts.len()));
        mapped.mappings.push((there, root.here));
        binds.retain(|bind| !root.mounts.contains(bind));
    }
    binds.sort_by(|a, b| a.there.cmp(&b.there));
    mapped.mappings.extend(binds.into_iter().map(|bind| (bind.there, bind.here)));
    mapped
}

/// A bind mount of a directory: the path here, and where it is in the container.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Bind {
    here: PathBuf,
    there: String,
}

/// Directories here and in the container that mounts lead down from alike.
struct Root {
    here: PathBuf,
    there: PathBuf,
    mounts: Vec<Bind>,
}

impl Root {
    /// The root of `here` and `there`, with the mounts of `binds` that it leads down to.
    fn of(here: PathBuf, there: PathBuf, binds: &[Bind]) -> Root {
        let below = |bind: &&Bind| {
            let rest = bind.here.strip_prefix(&here).ok().filter(|rest| !rest.as_os_str().is_empty());
            rest.is_some_and(|rest| Path::new(&bind.there).strip_prefix(&there).ok() == Some(rest))
        };
        let mounts = binds.iter().filter(below).cloned().collect();
        Root { here, there, mounts }
    }
}

/// The root that two or more mounts share and that maps nothing but them; of those, the one
/// that takes the most mounts, and of as many, the deepest.
fn collapsible(binds: &[Bind], is_code: &dyn Fn(&Path) -> bool) -> Option<Root> {
    let mut roots: Vec<Root> = Vec::new();
    for bind in binds {
        for (here, there) in shared_roots(&bind.here, Path::new(&bind.there)) {
            if !roots.iter().any(|root| root.here == here && root.there == there) {
                roots.push(Root::of(here, there, binds));
            }
        }
    }
    roots
        .sort_by_key(|root| (std::cmp::Reverse(root.mounts.len()), std::cmp::Reverse(root.there.components().count())));
    roots.into_iter().filter(|root| root.mounts.len() >= 2).find(|root| {
        let mounted: Vec<PathBuf> =
            root.mounts.iter().filter_map(|m| m.here.strip_prefix(&root.here).ok().map(Path::to_path_buf)).collect();
        let mut budget = LOOK;
        covered(&root.here, Path::new(""), &mounted, is_code, &mut budget)
    })
}

/// The pairs of directories above `host` and `container` that the same path leads down from:
/// for `/w/api/src` and `/app/src`, `/w/api` and `/app`. Neither is a root directory.
fn shared_roots(host: &Path, container: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mut roots = Vec::new();
    let (mut h, mut c) = (host, container);
    while let (Some(h_name), Some(c_name), Some(h_up), Some(c_up)) =
        (h.file_name(), c.file_name(), h.parent(), c.parent())
    {
        if h_name != c_name || h_up.parent().is_none() || c_up.parent().is_none() {
            break;
        }
        roots.push((h_up.to_path_buf(), c_up.to_path_buf()));
        (h, c) = (h_up, c_up);
    }
    roots
}

/// Whether everything under `root/rel` that may hold code is mounted (`mounted`, relative to
/// `root`) or above a mount and covered itself.
fn covered(root: &Path, rel: &Path, mounted: &[PathBuf], is_code: &dyn Fn(&Path) -> bool, budget: &mut usize) -> bool {
    let Ok(entries) = std::fs::read_dir(root.join(rel)) else { return false };
    for entry in entries {
        let Ok(entry) = entry else { return false };
        let path = rel.join(entry.file_name());
        if mounted.contains(&path) {
            continue;
        }
        let above = mounted.iter().any(|m| m.starts_with(&path));
        let ok = if above {
            covered(root, &path, mounted, is_code, budget)
        } else {
            !holds_code(&root.join(&path), is_code, budget)
        };
        if !ok {
            return false;
        }
    }
    true
}

/// Whether `path` is or holds a code file; when it cannot tell (unreadable, too much to look
/// at), that it may.
fn holds_code(path: &Path, is_code: &dyn Fn(&Path) -> bool, budget: &mut usize) -> bool {
    let Some(left) = budget.checked_sub(1) else { return true };
    *budget = left;
    let Ok(meta) = std::fs::metadata(path) else { return true };
    if !meta.is_dir() {
        return is_code(path);
    }
    let Ok(entries) = std::fs::read_dir(path) else { return true };
    entries.into_iter().any(|entry| entry.map_or(true, |e| holds_code(&e.path(), is_code, budget)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tuitor-compose-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("dir");
        dir
    }

    fn write(dir: &Path, file: &str, text: &str) {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        std::fs::write(path, text).expect("write");
    }

    fn php(path: &Path) -> bool {
        path.extension().is_some_and(|e| e == "php")
    }

    fn bind(source: &Path, target: &str) -> Mount {
        Mount::Bind { source: source.to_path_buf(), target: target.into() }
    }

    fn service(mounts: Vec<Mount>) -> Service {
        Service { name: "php".into(), image: None, build: None, dockerfile: None, mounts }
    }

    #[test]
    fn variables_take_defaults_and_say_what_is_missing() {
        let env = |name: &str| match name {
            "APP" => Some("/srv/app".to_owned()),
            "EMPTY" => Some(String::new()),
            _ => None,
        };
        let s = |text: &str| substitute(text, &env);
        assert_eq!(s("${APP}/src:$APP").ok().as_deref(), Some("/srv/app/src:/srv/app"));
        assert_eq!(s("${NOPE:-./api}").ok().as_deref(), Some("./api"));
        assert_eq!(s("${EMPTY:-x}|${EMPTY-x}|${NOPE-y}").ok().as_deref(), Some("x||y"));
        assert_eq!(s("${APP:+on}${NOPE:+on}").ok().as_deref(), Some("on"));
        assert_eq!(s("$$HOME ${NOPE:-${APP}/var}").ok().as_deref(), Some("$HOME /srv/app/var"));
        assert!(s("${NOPE:?set NOPE}").is_err());
        assert!(s("${APP").is_err());
    }

    #[test]
    fn dotenv_lines() {
        let vars = dotenv("# comment\nAPI_DIR=./api\nexport TAG=\"8.4 fpm\"\nPORT=9003 # note\n\nQUOTED='a#b'\n");
        let get = |k: &str| vars.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
        assert_eq!(
            (get("API_DIR"), get("TAG"), get("PORT"), get("QUOTED")),
            (Some("./api"), Some("8.4 fpm"), Some("9003"), Some("a#b"))
        );
    }

    #[test]
    fn short_and_long_volumes() {
        let dir = Path::new("/w/shop");
        let short = |text: &str| mount(&Value::String(text.into()), dir);
        assert_eq!(
            short("./api/src:/var/www/app/src:ro"),
            Some(bind(Path::new("/w/shop/api/src"), "/var/www/app/src"))
        );
        assert_eq!(short("../shared:/shared/"), Some(bind(Path::new("/w/shared"), "/shared")));
        assert_eq!(
            short("/etc/localtime:/etc/localtime:ro"),
            Some(bind(Path::new("/etc/localtime"), "/etc/localtime"))
        );
        assert_eq!(
            short("db_data:/var/lib/postgresql/data"),
            Some(Mount::Other { kind: "volume".into(), target: "/var/lib/postgresql/data".into() })
        );
        assert_eq!(short("/var/cache"), Some(Mount::Other { kind: "volume".into(), target: "/var/cache".into() }));
        assert_eq!(short("/var/cache:ro"), Some(Mount::Other { kind: "volume".into(), target: "/var/cache".into() }));
        let long = serde_json::json!({ "type": "bind", "source": "./api/public", "target": "/app/public" });
        assert_eq!(mount(&long, dir), Some(bind(Path::new("/w/shop/api/public"), "/app/public")));
        let tmpfs = serde_json::json!({ "type": "tmpfs", "target": "/tmp" });
        assert_eq!(mount(&tmpfs, dir), Some(Mount::Other { kind: "tmpfs".into(), target: "/tmp".into() }));
    }

    #[test]
    fn mounts_that_share_roots_and_cover_the_code_collapse() {
        let dir = temp("collapse");
        let api = dir.join("api");
        for file in ["src/Kernel.php", "config/bundles.php", "public/index.php", "composer.json", ".env"] {
            write(&api, file, "");
        }
        write(&api, ".git/HEAD", "ref");
        let mounts = ["src", "config", "public"].map(|d| bind(&api.join(d), &format!("/var/www/app/{d}")));
        let mut svc = service(mounts.to_vec());
        svc.mounts.push(Mount::Other { kind: "volume".into(), target: "/var/www/app/vendor".into() });
        svc.mounts.push(bind(&dir.join("var/xdebug"), "/tmp/xdebug"));
        let mapped = mappings(&svc, &php);
        assert_eq!(
            mapped.mappings,
            [("/var/www/app".to_owned(), api.clone()), ("/tmp/xdebug".to_owned(), dir.join("var/xdebug"))]
        );
        assert_eq!(mapped.collapsed, [("/var/www/app".to_owned(), 3)]);
        assert_eq!(mapped.skipped, ["/var/www/app/vendor (volume)"]);

        // Code here that is not mounted is the container's own copy: one mapping per mount.
        write(&api, "vendor/autoload.php", "");
        let mapped = mappings(&svc, &php);
        assert!(mapped.collapsed.is_empty());
        assert_eq!(mapped.mappings.len(), 4);
        assert_eq!(mapped.mappings[0], ("/tmp/xdebug".to_owned(), dir.join("var/xdebug")));
        assert_eq!(mapped.mappings[1], ("/var/www/app/config".to_owned(), api.join("config")));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn mounts_that_lead_down_different_paths_stay_apart() {
        let dir = temp("apart");
        write(&dir, "api/src/A.php", "");
        write(&dir, "lib/B.php", "");
        write(&dir, "docker/php.ini", "");
        let svc = service(vec![
            bind(&dir.join("api/src"), "/app/src"),
            bind(&dir.join("lib"), "/opt/lib"),
            bind(&dir.join("docker/php.ini"), "/usr/local/etc/php/php.ini"),
        ]);
        let mapped = mappings(&svc, &php);
        assert_eq!(
            mapped.mappings,
            [("/app/src".to_owned(), dir.join("api/src")), ("/opt/lib".to_owned(), dir.join("lib"))]
        );
        assert_eq!(mapped.skipped, ["/usr/local/etc/php/php.ini (a file, not a directory)"]);
        // A mount at the root of the container is never collapsed into `/`.
        assert_eq!(shared_roots(Path::new("/w/app"), Path::new("/app")), []);
        assert_eq!(shared_roots(Path::new("/w/api/src/A"), Path::new("/app/src/A")).len(), 2);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn compose_files_are_read_merged_and_interpolated_without_docker() {
        let dir = temp("files");
        write(&dir, "api/Dockerfile", "FROM dunglas/frankenphp:1-php8.4 AS base\n");
        for file in ["api/src/Kernel.php", "api/config/services.php", "api/public/index.php", "api/composer.json"] {
            write(&dir, file, "");
        }
        write(&dir, ".env", "API_DIR=./api\nAPP_ROOT=/var/www/app\n");
        write(
            &dir,
            "compose.yaml",
            r#"
x-php: &php
  build: { context: ./api }
  extra_hosts: ["host.docker.internal:host-gateway"]
  environment:
    XDEBUG_MODE: ${XDEBUG_MODE:-off}
services:
  php:
    <<: *php
    volumes:
      - ${API_DIR}/src:${APP_ROOT}/src
      - ${API_DIR}/config:${APP_ROOT}/config:ro
      - vendor:${APP_ROOT}/vendor
  worker:
    <<: *php
    command: ["bin/console", "messenger:consume"]
  database:
    image: postgres:${POSTGRES_VERSION:-16}-alpine
    volumes: ["db_data:/var/lib/postgresql/data:rw"]
  pwa:
    image: node:22
    volumes:
      - type: bind
        source: /etc/ssl/certs
        target: /certs
volumes: { vendor: {}, db_data: {} }
"#,
        );
        write(
            &dir,
            "compose.override.yaml",
            r#"
services:
  php:
    volumes:
      - ./api/public:/var/www/app/public
      - ./api/config:/var/www/app/config
"#,
        );
        let env = |name: &str| (name == "XDEBUG_MODE").then(|| "debug".to_owned());
        let value = read_files(&dir.join("compose.yaml"), &env).expect("read");
        assert_eq!(value["services"]["php"]["environment"]["XDEBUG_MODE"], "debug");
        assert_eq!(value["services"]["database"]["image"], "postgres:16-alpine");
        assert_eq!(value["services"]["worker"]["extra_hosts"][0], "host.docker.internal:host-gateway");
        let compose = Compose {
            file: dir.join("compose.yaml"),
            read_by: Reader::Tuitor,
            docker_error: None,
            services: services(&value, &dir).expect("services"),
        };
        // Both PHP services mount nothing but the worker; the database and pwa are no PHP.
        let php_service = pick(&compose, None, true).expect("php");
        assert_eq!(php_service.name, "php");
        assert_eq!(php_service.build.as_deref(), Some(dir.join("api").as_path()));
        assert_eq!(pick(&compose, Some("pwa"), true).map(|s| s.name.as_str()), Ok("pwa"));
        assert!(pick(&compose, Some("nginx"), true).is_err());
        let mapped = mappings(php_service, &php);
        assert_eq!(mapped.mappings, [("/var/www/app".to_owned(), dir.join("api"))]);
        assert_eq!(mapped.collapsed, [("/var/www/app".to_owned(), 3)]);
        assert_eq!(mapped.skipped, ["/var/www/app/vendor (volume)"]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn what_tuitor_does_not_read_is_left_to_docker() {
        let dir = temp("refused");
        let none = |_: &str| None;
        let read = || read_files(&dir.join("compose.yaml"), &none);
        write(&dir, "compose.yaml", "services:\n  php:\n    extends: { file: base.yaml, service: php }\n");
        assert!(read().is_err());
        write(&dir, "compose.yaml", "include: [base.yaml]\nservices: {}\n");
        assert!(read().is_err());
        write(&dir, "compose.yaml", "services:\n  php: { image: php }\n");
        write(&dir, "compose.override.yml", "# nothing here yet\n");
        assert_eq!(read().ok().map(|v| v["services"]["php"]["image"].clone()), Some(Value::from("php")));
        write(&dir, ".env", "COMPOSE_FILE=compose.yaml:compose.dev.yaml\n");
        assert!(read().is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_nearest_compose_file_above_is_found() {
        let dir = temp("find");
        write(&dir, "docker-compose.yml", "services: {}\n");
        std::fs::create_dir_all(dir.join("api/src")).expect("dirs");
        assert_eq!(find(&dir.join("api/src")), Some(dir.join("docker-compose.yml")));
        write(&dir, "api/compose.yaml", "services: {}\n");
        assert_eq!(find(&dir.join("api/src")), Some(dir.join("api/compose.yaml")));
        // Of two in one directory, the one compose prefers.
        write(&dir, "docker-compose.yaml", "services: {}\n");
        assert_eq!(find(&dir), Some(dir.join("docker-compose.yml")));
        std::fs::remove_dir_all(dir).ok();
    }
}
