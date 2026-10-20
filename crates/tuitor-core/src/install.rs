//! Installing an adapter from where its authors publish it (`tuitor install`, `/install`).
//! vscode-php-debug and CodeLLDB are unpacked from the `.vsix` of their GitHub release,
//! js-debug from its DAP server's `.tar.gz`, into tuitor's adapter directory, as README's
//! "Installing adapters" does by hand. Delve is built by `go install`, as its installation
//! guide says. What comes with the distribution (gdb, LLDB, Xdebug) is only named: installing
//! it takes root, which is the user's to give.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::config::{Adapters, Missing, adapters_dir, resolve_program};
use crate::doctor::{self, Packages};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Package {
    PhpDebug,
    JsDebug,
    CodeLldb,
    Delve,
    Gdb,
    LldbDap,
    Xdebug,
}

impl Package {
    pub const ALL: [Package; 7] = [
        Package::PhpDebug,
        Package::JsDebug,
        Package::CodeLldb,
        Package::Delve,
        Package::Gdb,
        Package::LldbDap,
        Package::Xdebug,
    ];

    /// Its name, which is also its directory under `<adapters>` for a release.
    pub fn name(self) -> &'static str {
        match self {
            Package::PhpDebug => "vscode-php-debug",
            Package::JsDebug => "js-debug",
            Package::CodeLldb => "codelldb",
            Package::Delve => "delve",
            Package::Gdb => "gdb",
            Package::LldbDap => "lldb-dap",
            Package::Xdebug => "xdebug",
        }
    }

    /// A package by its name, or by the launch type of an adapter it is.
    pub fn parse(name: &str) -> Option<Package> {
        Some(match name {
            "vscode-php-debug" | "php-debug" | "php" => Package::PhpDebug,
            "js-debug" | "pwa-node" | "node" | "pwa-chrome" | "chrome" => Package::JsDebug,
            "codelldb" | "lldb" => Package::CodeLldb,
            "delve" | "dlv" | "go" => Package::Delve,
            "gdb" => Package::Gdb,
            "lldb-dap" => Package::LldbDap,
            "xdebug" => Package::Xdebug,
            _ => return None,
        })
    }

    /// The adapter types that run it.
    pub fn adapters(self) -> &'static [&'static str] {
        match self {
            Package::PhpDebug => &["php"],
            Package::JsDebug => &["pwa-node", "node", "pwa-chrome", "chrome"],
            Package::CodeLldb => &["lldb"],
            Package::Delve => &["go"],
            Package::Gdb => &["gdb"],
            Package::LldbDap => &["lldb-dap"],
            Package::Xdebug => &[],
        }
    }

    /// The release installed when none is asked for: the one README names.
    fn default_version(self) -> &'static str {
        match self {
            Package::PhpDebug => "1.40.2",
            Package::JsDebug => "1.140.0",
            Package::CodeLldb => "1.12.3",
            _ => "latest",
        }
    }
}

impl fmt::Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// What installing a package takes.
#[derive(Debug, Clone)]
pub enum Plan {
    /// A release archive from GitHub, unpacked into `<adapters>/<name>`.
    Release(Release),
    /// `go install`, which builds the program into Go's bin directory.
    Go(GoInstall),
    /// A package of the distribution, which tuitor does not install.
    Distro { package: Package, command: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Archive {
    Zip,
    TarGz,
}

#[derive(Debug, Clone)]
pub struct Release {
    pub package: Package,
    pub version: String,
    pub url: String,
    pub dir: PathBuf,
    archive: Archive,
    /// What the adapter runs, relative to `dir`: there once the archive is unpacked.
    pub runs: &'static str,
}

#[derive(Debug, Clone)]
pub struct GoInstall {
    /// `github.com/go-delve/delve/cmd/dlv@latest`.
    pub target: String,
}

impl GoInstall {
    pub fn command(&self) -> String {
        format!("go install {}", self.target)
    }
}

/// What installing `package` takes, at `version` (a release such as `1.40.2`; by default the
/// one README names, and for Delve the latest).
pub fn plan(package: Package, version: Option<&str>) -> Result<Plan> {
    let version = version.map(|v| v.trim_start_matches('v')).unwrap_or(package.default_version()).to_owned();
    let release = |url: String, archive, runs| {
        let dir = adapters_dir().join(package.name());
        Plan::Release(Release { package, version: version.clone(), url, dir, archive, runs })
    };
    let distro = |packages: &Packages| Plan::Distro { package, command: doctor::distro(packages) };
    Ok(match package {
        Package::PhpDebug => release(
            format!("https://github.com/xdebug/vscode-php-debug/releases/download/v{version}/php-debug-{version}.vsix"),
            Archive::Zip,
            "extension/out/phpDebug.js",
        ),
        Package::JsDebug => release(
            format!(
                "https://github.com/microsoft/vscode-js-debug/releases/download/v{version}/js-debug-dap-v{version}.tar.gz"
            ),
            Archive::TarGz,
            "js-debug/src/dapDebugServer.js",
        ),
        Package::CodeLldb => {
            let platform = codelldb_platform(std::env::consts::OS, std::env::consts::ARCH)?;
            release(
                format!("https://github.com/vadimcn/codelldb/releases/download/v{version}/codelldb-{platform}.vsix"),
                Archive::Zip,
                "extension/adapter/codelldb",
            )
        }
        Package::Delve => {
            let version = if version == "latest" { version } else { format!("v{version}") };
            Plan::Go(GoInstall { target: format!("github.com/go-delve/delve/cmd/dlv@{version}") })
        }
        Package::Gdb => distro(&doctor::GDB),
        Package::LldbDap => distro(&doctor::LLDB),
        Package::Xdebug => distro(&doctor::XDEBUG),
    })
}

/// The name CodeLLDB gives its `.vsix` for a platform, as Rust names the platform.
fn codelldb_platform(os: &str, arch: &str) -> Result<&'static str> {
    Ok(match (os, arch) {
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("linux", "arm") => "linux-armhf",
        ("macos", "x86_64") => "darwin-x64",
        ("macos", "aarch64") => "darwin-arm64",
        _ => bail!("CodeLLDB publishes no release for {os} on {arch}"),
    })
}

/// How the programs an installation runs show their progress: on the terminal, or kept for
/// tuitor to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    Terminal,
    Quiet,
}

impl Release {
    /// What it needs that is not there: curl, and unzip or tar.
    pub fn missing_tools(&self) -> Vec<&'static str> {
        let unpack = match self.archive {
            Archive::Zip => "unzip",
            Archive::TarGz => "tar",
        };
        ["curl", unpack].into_iter().filter(|tool| resolve_program(tool).is_none()).collect()
    }

    /// Downloads and unpacks the release next to `dir`, and puts it in its place only once it
    /// is complete, so that a failed download leaves an installation that worked as it was.
    /// `step` hears of each step as it starts.
    pub fn install(&self, output: Output, step: &mut dyn FnMut(String)) -> Result<()> {
        if let Some(tool) = self.missing_tools().first() {
            bail!("installing {} needs `{tool}`, which is not on PATH", self.package);
        }
        if !self.dir.is_absolute() {
            bail!("there is no data directory to install {} into: XDG_DATA_HOME and HOME are not set", self.package);
        }
        let parent = self.dir.parent().context("the adapter directory has no parent")?;
        // Of this process alone: another installation at the same time keeps its own.
        let staging = parent.join(format!(".{}.partial-{}", self.package.name(), std::process::id()));
        let result = self.unpack_into(&staging, output, step).and_then(|()| {
            if self.dir.exists() {
                step(format!("replacing {}", self.dir.display()));
                std::fs::remove_dir_all(&self.dir).with_context(|| format!("removing {}", self.dir.display()))?;
            } else {
                step(format!("moving it to {}", self.dir.display()));
            }
            std::fs::rename(&staging, &self.dir).with_context(|| format!("moving it to {}", self.dir.display()))
        });
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&staging);
        }
        result
    }

    fn unpack_into(&self, staging: &Path, output: Output, step: &mut dyn FnMut(String)) -> Result<()> {
        if staging.exists() {
            std::fs::remove_dir_all(staging).with_context(|| format!("removing {}", staging.display()))?;
        }
        std::fs::create_dir_all(staging).with_context(|| format!("creating {}", staging.display()))?;
        let file_name = self.url.rsplit('/').next().unwrap_or("release");
        let archive = staging.join(file_name);
        step(format!("downloading {}", self.url));
        let progress = if output == Output::Terminal { "--progress-bar" } else { "--silent" };
        let mut command = Command::new("curl");
        command.args([progress, "--show-error", "--fail", "--location", "--proto", "=https"]);
        // A download that stalls for a minute fails rather than hangs.
        command.args(["--connect-timeout", "30", "--speed-limit", "1", "--speed-time", "60"]);
        command.arg("--output").arg(&archive).arg(&self.url);
        run(command, output).with_context(|| format!("downloading {}", self.url))?;
        step(format!("unpacking {file_name}"));
        let mut command = match self.archive {
            Archive::Zip => {
                let mut unzip = Command::new("unzip");
                unzip.arg("-q").arg(&archive).arg("-d").arg(staging);
                unzip
            }
            Archive::TarGz => {
                let mut tar = Command::new("tar");
                tar.arg("-xzf").arg(&archive).arg("-C").arg(staging);
                tar
            }
        };
        command.stdin(Stdio::null());
        run(command, output).with_context(|| format!("unpacking {file_name}"))?;
        std::fs::remove_file(&archive).with_context(|| format!("removing {}", archive.display()))?;
        if !staging.join(self.runs).is_file() {
            bail!("{file_name} has no {}: is {} a release of {}?", self.runs, self.version, self.package);
        }
        Ok(())
    }
}

impl GoInstall {
    /// Runs `go install`; Go downloads and builds the module, which takes a while the first
    /// time. Returns the directory it put the program in.
    pub fn install(&self, output: Output, step: &mut dyn FnMut(String)) -> Result<PathBuf> {
        let Some(go) = resolve_program("go") else {
            bail!("`go` is not on PATH; Go builds Delve: {}", doctor::distro(&doctor::GO));
        };
        step(self.command());
        let mut command = Command::new(&go);
        command.args(["install", &self.target]).stdin(Stdio::null());
        run(command, output).context("go install")?;
        let bin = go_bin(&go).context("`go env` does not say where go install puts programs")?;
        if !bin.join("dlv").is_file() {
            bail!("go install did not put dlv in {}", bin.display());
        }
        Ok(bin)
    }
}

/// Where `go install` puts programs, as Go itself says: `GOBIN`, else `bin` in the first
/// directory of `GOPATH`.
fn go_bin(go: &Path) -> Option<PathBuf> {
    let answer = Command::new(go).args(["env", "GOBIN", "GOPATH"]).stdin(Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&answer.stdout).into_owned();
    let mut lines = text.lines();
    let gobin = lines.next().unwrap_or_default().trim();
    if !gobin.is_empty() {
        return Some(PathBuf::from(gobin));
    }
    let gopath = lines.next()?.trim();
    Some(std::env::split_paths(gopath).next()?.join("bin"))
}

/// Runs a step: on the terminal, or with what it says kept for the error.
fn run(mut command: Command, output: Output) -> Result<()> {
    let program = command.get_program().to_string_lossy().into_owned();
    match output {
        Output::Terminal => {
            let status = command.status().with_context(|| format!("starting {program}"))?;
            if !status.success() {
                bail!("{program} failed ({status})");
            }
        }
        Output::Quiet => {
            let answer = command.output().with_context(|| format!("starting {program}"))?;
            if !answer.status.success() {
                let said = String::from_utf8_lossy(&answer.stderr);
                let last: Vec<&str> = said.lines().filter(|l| !l.trim().is_empty()).rev().take(3).collect();
                let last: Vec<&str> = last.into_iter().rev().collect();
                bail!("{program} failed ({}): {}", answer.status, last.join(" · "));
            }
        }
    }
    Ok(())
}

/// What an adapter finds once its package is installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// The program it runs.
    Runs(PathBuf),
    Missing(Missing),
    /// adapters.toml has it run something else.
    Elsewhere,
}

/// What `locate`, which every session start asks, says of each adapter the package is for.
pub fn verify(package: Package, adapters: &Adapters, cwd: &Path) -> Vec<(String, Found)> {
    let installed = adapters_dir().join(package.name());
    package
        .adapters()
        .iter()
        .filter_map(|name| adapters.get(name))
        .map(|spec| {
            let ours = matches!(package, Package::Delve | Package::Gdb | Package::LldbDap)
                || Path::new(&spec.command).starts_with(&installed)
                || spec.args.iter().any(|arg| Path::new(arg).starts_with(&installed));
            let found = match spec.locate(cwd) {
                _ if !ours => Found::Elsewhere,
                Ok(program) => Found::Runs(program),
                Err(missing) => Found::Missing(missing),
            };
            (spec.name.clone(), found)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_adapter_types_name_packages() {
        for package in Package::ALL {
            assert_eq!(Package::parse(package.name()), Some(package));
            for adapter in package.adapters() {
                assert_eq!(Package::parse(adapter), Some(package), "{adapter}");
            }
        }
        assert_eq!(Package::parse("frobnicate"), None);
    }

    #[test]
    fn releases_go_where_the_adapters_look() {
        let adapters = Adapters::default();
        let release = |package| match plan(package, None).expect("plan") {
            Plan::Release(release) => release,
            other => panic!("{other:?}"),
        };
        let php = release(Package::PhpDebug);
        assert_eq!(adapters.get("php").map(|s| PathBuf::from(&s.args[0])), Some(php.dir.join(php.runs)));
        assert_eq!(
            php.url,
            "https://github.com/xdebug/vscode-php-debug/releases/download/v1.40.2/php-debug-1.40.2.vsix"
        );
        let js = release(Package::JsDebug);
        assert_eq!(adapters.get("pwa-node").map(|s| PathBuf::from(&s.args[0])), Some(js.dir.join(js.runs)));
        assert!(js.url.ends_with("/v1.140.0/js-debug-dap-v1.140.0.tar.gz"));
        if let Ok(Plan::Release(lldb)) = plan(Package::CodeLldb, Some("v1.12.0")) {
            assert_eq!(adapters.get("lldb").map(|s| PathBuf::from(&s.command)), Some(lldb.dir.join(lldb.runs)));
            assert!(lldb.url.starts_with("https://github.com/vadimcn/codelldb/releases/download/v1.12.0/codelldb-"));
        }
    }

    #[test]
    fn delve_is_built_by_go_and_the_rest_comes_with_the_distribution() {
        let Ok(Plan::Go(latest)) = plan(Package::Delve, None) else { panic!("go install") };
        assert_eq!(latest.command(), "go install github.com/go-delve/delve/cmd/dlv@latest");
        let Ok(Plan::Go(pinned)) = plan(Package::Delve, Some("1.27.2")) else { panic!("go install") };
        assert_eq!(pinned.target, "github.com/go-delve/delve/cmd/dlv@v1.27.2");
        for package in [Package::Gdb, Package::LldbDap, Package::Xdebug] {
            let Ok(Plan::Distro { command, .. }) = plan(package, None) else { panic!("{package}") };
            assert!(!command.contains("sudo"), "{command}");
        }
    }

    #[test]
    fn codelldb_is_released_per_platform() {
        assert_eq!(codelldb_platform("linux", "x86_64").ok(), Some("linux-x64"));
        assert_eq!(codelldb_platform("macos", "aarch64").ok(), Some("darwin-arm64"));
        assert!(codelldb_platform("freebsd", "x86_64").is_err());
    }

    #[test]
    fn a_failed_download_leaves_the_installation_as_it_was() {
        let root = std::env::temp_dir().join(format!("tuitor-install-{}", std::process::id()));
        let dir = root.join("vscode-php-debug");
        std::fs::create_dir_all(dir.join("extension/out")).expect("dir");
        std::fs::write(dir.join("extension/out/phpDebug.js"), "// works").expect("script");
        let release = Release {
            package: Package::PhpDebug,
            version: "0.0.0".into(),
            // Nothing listens there: curl fails at once.
            url: "https://127.0.0.1:9/php-debug-0.0.0.vsix".into(),
            dir: dir.clone(),
            archive: Archive::Zip,
            runs: "extension/out/phpDebug.js",
        };
        if release.missing_tools().is_empty() {
            let mut steps = Vec::new();
            assert!(release.install(Output::Quiet, &mut |step| steps.push(step)).is_err());
            assert_eq!(steps.len(), 1, "{steps:?}");
            assert!(dir.join("extension/out/phpDebug.js").is_file());
            let partial = format!(".vscode-php-debug.partial-{}", std::process::id());
            assert!(!root.join(partial).exists());
        }
        std::fs::remove_dir_all(root).ok();
    }
}
