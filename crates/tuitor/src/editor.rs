//! Opening a file at a line in the user's editor, as `edit` asks. What runs: the `editor`
//! command of config.toml; else, in the terminal of an editor (Neovim, Vim, Kakoune, Emacs,
//! Zed, VS Code), that editor, so that the file opens beside tuitor; else `$VISUAL` or
//! `$EDITOR`. An editor named without `{file}` gets the file and the line as it takes them.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// How long an editor that runs beside tuitor gets to say it failed. One that is still
/// running then opened the file and waits for it to be closed, as `code --wait` does.
const BESIDE_WAIT: Duration = Duration::from_secs(3);

/// A Lua expression that the Neovim tuitor runs in evaluates for `--remote-expr`, with the
/// file and the line as `_A`. The file opens in a window that shows it already, else in the
/// window used before tuitor's terminal, else in a split beside that: it never takes the
/// place of tuitor. Kept free of single quotes, as it goes in a Vim string that way.
const NEOVIM_OPEN: &str = r#"(function(file, line)
  local shown = false
  for _, win in ipairs(vim.api.nvim_list_wins()) do
    shown = shown or vim.api.nvim_buf_get_name(vim.api.nvim_win_get_buf(win)) == file
  end
  if not shown and vim.bo.buftype == "terminal" then
    vim.cmd("silent! wincmd p")
    if vim.bo.buftype == "terminal" then vim.cmd("vsplit") end
  end
  vim.cmd("drop +" .. line .. " " .. vim.fn.fnameescape(file))
  vim.cmd("normal! zz")
end)(_A[1], _A[2])"#;

/// tuitor's own settings, from `config.toml` in its config directory.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Settings {
    /// The command `edit` runs: split at spaces, with `{file}` and `{line}` replaced in each word;
    /// without either, the file and the line are added as the editor takes them (`code`, `hx`).
    pub editor: Option<String>,
    /// Whether that command runs in tuitor's terminal, which tuitor hands it until it exits.
    /// `false` for an editor with a window of its own, which tuitor stays on screen beside.
    /// Unset, a command with `{file}` runs in the terminal, and a named editor as it is known to.
    pub editor_terminal: Option<bool>,
    /// The language servers `paths` asks, by language: `[lsp.php]`, `[lsp.typescript]`,
    /// `[lsp.rust]`, `[lsp.go]`.
    pub lsp: tuitor_code::Servers,
}

impl Settings {
    /// The settings in `file`, over the defaults; without the file, the defaults.
    pub fn load(file: Option<&Path>) -> Result<Settings> {
        let Some(path) = file.filter(|p| p.exists()) else { return Ok(Settings::default()) };
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }
}

/// A command that opens a file at a line.
#[derive(Debug)]
pub struct Editor {
    pub program: String,
    pub args: Vec<String>,
    /// It runs in tuitor's terminal, which tuitor hands it until it exits. Otherwise it runs
    /// beside tuitor, which stays on screen.
    pub terminal: bool,
    /// `file:line`, for telling what was opened.
    pub place: String,
    /// What to write to its standard input, for an editor that reads commands there (Kakoune).
    pub stdin: Option<String>,
}

impl Editor {
    /// The editor to open `file` at `line` in, `env` reading the environment.
    pub fn new(
        settings: &Settings,
        env: impl Fn(&str) -> Option<String>,
        file: &Path,
        line: u32,
    ) -> Result<Editor, String> {
        let place = format!("{}:{line}", file.file_name().unwrap_or(file.as_os_str()).to_string_lossy());
        let path = file.to_string_lossy();
        // An empty variable counts as none.
        let set = |name: &str| env(name).filter(|value: &String| !value.trim().is_empty());
        let mut stdin = None;
        let (words, terminal) = match (&settings.editor, host(&set)) {
            (Some(template), _) if template.contains("{file}") || template.contains("{line}") => {
                let line = line.to_string();
                let words = template.split_whitespace().map(|word| fill(word, &path, &line)).collect();
                (words, settings.editor_terminal.unwrap_or(true))
            }
            (Some(named), _) if named.trim().is_empty() => {
                return Err("`editor` in ~/.config/tuitor/config.toml is empty".into());
            }
            (Some(named), _) => {
                let (words, terminal) = named_editor(named, &path, line);
                (words, settings.editor_terminal.unwrap_or(terminal))
            }
            (None, Some(Host::Neovim(server))) => {
                let expr = format!("luaeval({}, [{}, {line}])", vim_string(NEOVIM_OPEN), vim_string(&path));
                (vec!["nvim".into(), "--server".into(), server, "--remote-expr".into(), expr], false)
            }
            (None, Some(Host::Vim(server))) => {
                let words = ["vim", "--servername", &server, "--remote-silent", &format!("+{line}"), &path];
                (words.map(str::to_owned).to_vec(), false)
            }
            (None, Some(Host::Kakoune { session, client })) => {
                // `-verbatim` runs the words as one command, so they are quoted once.
                let command = format!(
                    "evaluate-commands -client {} -verbatim -- edit -existing {} {line}\n",
                    kak_string(&client),
                    kak_string(&path)
                );
                stdin = Some(command);
                (vec!["kak".into(), "-p".into(), session], false)
            }
            (None, Some(Host::Command(program))) => named_editor(&program, &path, line),
            (None, None) => {
                let editor = set("VISUAL")
                    .or_else(|| set("EDITOR"))
                    .ok_or("no editor: set $VISUAL or $EDITOR, or `editor` in ~/.config/tuitor/config.toml")?;
                named_editor(&editor, &path, line)
            }
        };
        let mut words = words.into_iter();
        let program = words.next().ok_or("`editor` in ~/.config/tuitor/config.toml is empty")?;
        Ok(Editor { program, args: words.collect(), terminal, place, stdin })
    }

    /// Runs it in tuitor's terminal, which the caller has put back as the shell had it, and
    /// waits for it to exit.
    pub async fn run_in_terminal(&self) -> Result<String, String> {
        let status = Command::new(&self.program).args(&self.args).status().await.map_err(|e| self.cannot_run(&e))?;
        if !status.success() {
            return Err(format!("{} ended with {status}", self.name()));
        }
        Ok(format!("back from {} ({})", self.name(), self.place))
    }

    /// Runs it beside tuitor, out of its terminal, and waits a moment for it to fail.
    pub async fn run_beside(self) -> Result<String, String> {
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .stdin(if self.stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| self.cannot_run(&e))?;
        if let (Some(text), Some(mut stdin)) = (&self.stdin, child.stdin.take()) {
            stdin.write_all(text.as_bytes()).await.map_err(|e| format!("{}: {e}", self.name()))?;
            // Closed, so that the editor reads to its end.
            drop(stdin);
        }
        let output = match tokio::time::timeout(BESIDE_WAIT, child.wait_with_output()).await {
            Ok(output) => output.map_err(|e| format!("{}: {e}", self.name()))?,
            Err(_) => return Ok(format!("opened {} in {}", self.place, self.name())),
        };
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let said = stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or_default().trim().to_owned();
            let why = if said.is_empty() { output.status.to_string() } else { said };
            return Err(format!("{}: {why}", self.name()));
        }
        Ok(format!("opened {} in {}", self.place, self.name()))
    }

    /// The program's name, without the directory it is in.
    pub fn name(&self) -> String {
        Path::new(&self.program).file_name().map_or_else(|| self.program.clone(), |n| n.to_string_lossy().into_owned())
    }

    fn cannot_run(&self, err: &std::io::Error) -> String {
        format!("cannot run {}: {err}", self.program)
    }
}

/// The editor whose terminal tuitor runs in, as its environment says.
enum Host {
    /// Neovim's server, `$NVIM`.
    Neovim(String),
    /// Vim's server, `$VIM_SERVERNAME`, which a Vim built with `+clientserver` sets.
    Vim(String),
    /// A Kakoune session and the client whose terminal it is.
    Kakoune { session: String, client: String },
    /// An editor that opens files in its running window from its command: Emacs, Zed, VS Code.
    Command(String),
}

/// The editor whose terminal tuitor runs in, `set` reading a variable that is set and not empty.
fn host(set: &impl Fn(&str) -> Option<String>) -> Option<Host> {
    if let Some(server) = set("NVIM") {
        return Some(Host::Neovim(server));
    }
    if let Some(server) = set("VIM_SERVERNAME") {
        return Some(Host::Vim(server));
    }
    if let (Some(session), Some(client)) = (set("KAKOUNE_SESSION"), set("KAKOUNE_CLIENT")) {
        return Some(Host::Kakoune { session, client });
    }
    if set("INSIDE_EMACS").is_some() {
        return Some(Host::Command("emacsclient -n".into()));
    }
    let term = set("TERM_PROGRAM").unwrap_or_default();
    if set("ZED_TERM").is_some() || term == "zed" {
        return Some(Host::Command("zed".into()));
    }
    if term == "vscode" {
        // Forks of VS Code say they are it; the path of their git helper names them.
        let askpass = set("VSCODE_GIT_ASKPASS_MAIN").unwrap_or_default().to_lowercase();
        let cli = ["cursor", "windsurf", "codium"].into_iter().find(|fork| askpass.contains(fork)).unwrap_or("code");
        return Some(Host::Command(cli.into()));
    }
    None
}

/// `editor`, a command such as `code --wait`, with the file and the line added as that editor
/// takes them, and whether it runs in tuitor's terminal.
fn named_editor(editor: &str, file: &str, line: u32) -> (Vec<String>, bool) {
    let mut words: Vec<String> = editor.split_whitespace().map(str::to_owned).collect();
    let program = words.first().map(Path::new).and_then(Path::file_name).unwrap_or_default();
    let program = program.to_string_lossy();
    let at = format!("{file}:{line}");
    let (args, terminal) = match program.trim_end_matches(".sh").trim_end_matches(".exe") {
        "hx" | "helix" => (vec![at], true),
        "code" | "code-insiders" | "codium" | "cursor" | "windsurf" => (vec!["--goto".into(), at], false),
        "zed" | "zeditor" | "subl" | "sublime_text" => (vec![at], false),
        "idea" | "pycharm" | "phpstorm" | "goland" | "webstorm" | "clion" | "rustrover" | "rider" | "rubymine" => {
            (vec!["--line".into(), line.to_string(), file.to_owned()], false)
        }
        "emacsclient" => (vec![format!("+{line}"), file.to_owned()], !words.iter().any(|w| w == "-n")),
        // `+line file`, as vi, Vim, Neovim, Emacs, nano, micro and Kakoune take it.
        _ => (vec![format!("+{line}"), file.to_owned()], true),
    };
    words.extend(args);
    (words, terminal)
}

/// `word` with `{file}` and `{line}` replaced; what replaces them is not looked at again.
fn fill(word: &str, file: &str, line: &str) -> String {
    let mut out = String::new();
    let mut rest = word;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let (value, after) = if let Some(after) = rest.strip_prefix("{file}") {
            (file, after)
        } else if let Some(after) = rest.strip_prefix("{line}") {
            (line, after)
        } else {
            ("{", &rest[1..])
        };
        out.push_str(value);
        rest = after;
    }
    out.push_str(rest);
    out
}

/// A Vim string literal: in single quotes, where only a single quote is special, doubled.
fn vim_string(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// A Kakoune string, quoted as Vim's is.
fn kak_string(text: &str) -> String {
    vim_string(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| (*v).to_owned())
    }

    fn editor(settings: &Settings, vars: &[(&str, &str)]) -> Result<Editor, String> {
        Editor::new(settings, env(vars), Path::new("/src/my app/orders.c"), 48)
    }

    #[test]
    fn the_configured_command_wins_and_takes_the_file_and_line_in_any_word() {
        let settings = Settings {
            editor: Some("code --goto {file}:{line}".into()),
            editor_terminal: Some(false),
            lsp: tuitor_code::Servers::default(),
        };
        let code = editor(&settings, &[("NVIM", "/run/nvim.sock"), ("EDITOR", "vim")]).expect("editor");
        assert_eq!((code.program.as_str(), code.terminal), ("code", false));
        assert_eq!(code.args, ["--goto", "/src/my app/orders.c:48"]);
        assert_eq!(code.place, "orders.c:48");
        assert!(editor(&Settings { editor: Some("  ".into()), ..Settings::default() }, &[]).is_err());
    }

    #[test]
    fn inside_neovim_the_file_goes_to_that_neovim() {
        let nvim = editor(&Settings::default(), &[("NVIM", "/run/nvim.sock"), ("EDITOR", "vim")]).expect("editor");
        assert_eq!((nvim.program.as_str(), nvim.terminal), ("nvim", false));
        assert_eq!(nvim.args[..3], ["--server", "/run/nvim.sock", "--remote-expr"]);
        assert!(nvim.args[3].ends_with(", ['/src/my app/orders.c', 48])"), "{}", nvim.args[3]);
        assert!(!NEOVIM_OPEN.contains('\''), "the Lua goes in a single-quoted Vim string");
    }

    #[test]
    fn otherwise_visual_or_editor_runs_in_the_terminal() {
        let vim = editor(&Settings::default(), &[("EDITOR", "vim"), ("VISUAL", "emacs -nw"), ("NVIM", "")]);
        let vim = vim.expect("editor");
        assert_eq!((vim.program.as_str(), vim.terminal), ("emacs", true));
        assert_eq!(vim.args, ["-nw", "+48", "/src/my app/orders.c"]);
        assert_eq!(editor(&Settings::default(), &[("EDITOR", "nano")]).expect("editor").program, "nano");
        let empty = editor(&Settings::default(), &[("VISUAL", ""), ("EDITOR", "nano")]);
        assert_eq!(empty.expect("editor").program, "nano");
        assert!(editor(&Settings::default(), &[]).is_err());
    }

    #[test]
    fn a_named_editor_gets_the_file_and_line_as_it_takes_them() {
        let named = |name: &str, terminal: Option<bool>| {
            let settings = Settings { editor: Some(name.into()), editor_terminal: terminal, ..Settings::default() };
            let editor = editor(&settings, &[("NVIM", "/run/nvim.sock")]).expect("editor");
            (editor.program, editor.args, editor.terminal)
        };
        let code = named("code --wait", None);
        assert_eq!(
            code,
            ("code".into(), vec!["--wait".into(), "--goto".into(), "/src/my app/orders.c:48".into()], false)
        );
        assert_eq!(named("hx", None).1, ["/src/my app/orders.c:48"]);
        assert!(named("hx", None).2, "Helix runs in tuitor's terminal");
        assert_eq!(named("/opt/zed/bin/zed", None).1, ["/src/my app/orders.c:48"]);
        assert_eq!(named("phpstorm.sh", None).1, ["--line", "48", "/src/my app/orders.c"]);
        assert!(!named("emacsclient -n", None).2);
        assert!(named("emacsclient -t", None).2);
        assert_eq!(named("micro", None).1, ["+48", "/src/my app/orders.c"]);
        assert!(named("code", Some(true)).2, "editor_terminal still decides");
    }

    #[test]
    fn the_terminal_of_an_editor_says_which_one_opens_the_file() {
        let host = |vars: &[(&str, &str)]| {
            let editor = editor(&Settings::default(), vars).expect("editor");
            (editor.program, editor.args, editor.terminal, editor.stdin)
        };
        let vim = host(&[("VIM_SERVERNAME", "VIM"), ("EDITOR", "nano")]);
        assert_eq!(vim.1, ["--servername", "VIM", "--remote-silent", "+48", "/src/my app/orders.c"]);
        assert_eq!((vim.0.as_str(), vim.2), ("vim", false));
        let kak = host(&[("KAKOUNE_SESSION", "4242"), ("KAKOUNE_CLIENT", "client0")]);
        assert_eq!(
            (kak.0.as_str(), kak.1.as_slice(), kak.2),
            ("kak", &["-p".to_owned(), "4242".to_owned()][..], false)
        );
        assert_eq!(
            kak.3.as_deref(),
            Some("evaluate-commands -client 'client0' -verbatim -- edit -existing '/src/my app/orders.c' 48\n")
        );
        let emacs = host(&[("INSIDE_EMACS", "29.4,vterm")]);
        assert_eq!(
            (emacs.0.as_str(), emacs.1.as_slice()),
            ("emacsclient", &["-n", "+48", "/src/my app/orders.c"].map(String::from)[..])
        );
        assert!(!emacs.2);
        assert_eq!(host(&[("ZED_TERM", "true")]).0, "zed");
        assert_eq!(host(&[("TERM_PROGRAM", "vscode")]).1, ["--goto", "/src/my app/orders.c:48"]);
        let cursor = host(&[
            ("TERM_PROGRAM", "vscode"),
            ("VSCODE_GIT_ASKPASS_MAIN", "/usr/share/cursor/resources/app/extensions/git/dist/askpass-main.js"),
        ]);
        assert_eq!(cursor.0, "cursor");
        // A Kakoune session alone, without its client, is no Kakoune terminal.
        assert_eq!(host(&[("KAKOUNE_SESSION", "4242"), ("EDITOR", "nano")]).0, "nano");
    }

    #[test]
    fn filling_in_leaves_what_it_put_in_alone() {
        assert_eq!(fill("{file}:{line}", "/a/{line}.c", "7"), "/a/{line}.c:7");
        assert_eq!(fill("+{line}", "f", "7"), "+7");
        assert_eq!(fill("{x}{", "f", "7"), "{x}{");
        assert_eq!(vim_string("it's"), "'it''s'");
    }

    #[test]
    fn settings_come_from_config_toml() {
        let settings: Settings =
            toml::from_str("editor = \"subl {file}:{line}\"\neditor_terminal = false\n").expect("settings");
        assert_eq!((settings.editor.as_deref(), settings.editor_terminal), (Some("subl {file}:{line}"), Some(false)));
        let settings: Settings = toml::from_str("").expect("defaults");
        assert!(settings.editor.is_none() && settings.editor_terminal.is_none());
        assert!(toml::from_str::<Settings>("edtior = \"vim\"").is_err(), "a misspelt key is an error");
        let settings: Settings =
            toml::from_str("[lsp.rust]\ncommand = \"mise\"\nargs = [\"exec\", \"--\", \"rust-analyzer\"]\n")
                .expect("lsp");
        assert_eq!(settings.lsp.command(tuitor_code::Language::Rust).program, "mise");
    }
}
