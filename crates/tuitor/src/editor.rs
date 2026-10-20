//! Opening a file at a line in the user's editor, as `edit` asks. What runs: the `editor`
//! command of config.toml; else, in a Neovim terminal, that Neovim, so that the file opens
//! beside tuitor; else `$VISUAL` or `$EDITOR`, in tuitor's terminal.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
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
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Settings {
    /// The command `edit` runs: split at spaces, with `{file}` and `{line}` replaced in each word.
    pub editor: Option<String>,
    /// Whether that command runs in tuitor's terminal, which tuitor hands it until it exits.
    /// `false` for an editor with a window of its own, which tuitor stays on screen beside.
    pub editor_terminal: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { editor: None, editor_terminal: true }
    }
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
        let (words, terminal) = match (&settings.editor, env("NVIM").filter(|s| !s.is_empty())) {
            (Some(template), _) => {
                let line = line.to_string();
                (template.split_whitespace().map(|word| fill(word, &path, &line)).collect(), settings.editor_terminal)
            }
            (None, Some(server)) => {
                let expr = format!("luaeval({}, [{}, {line}])", vim_string(NEOVIM_OPEN), vim_string(&path));
                (vec!["nvim".into(), "--server".into(), server, "--remote-expr".into(), expr], false)
            }
            (None, None) => {
                let editor = env("VISUAL")
                    .or_else(|| env("EDITOR"))
                    .filter(|e| !e.trim().is_empty())
                    .ok_or("no editor: set $VISUAL or $EDITOR, or `editor` in ~/.config/tuitor/config.toml")?;
                // `+line file`, as vi, Vim, Neovim, Emacs, nano, micro and Kakoune take it.
                let mut words: Vec<String> = editor.split_whitespace().map(str::to_owned).collect();
                words.extend([format!("+{line}"), path.into_owned()]);
                (words, true)
            }
        };
        let mut words = words.into_iter();
        let program = words.next().ok_or("`editor` in ~/.config/tuitor/config.toml is empty")?;
        Ok(Editor { program, args: words.collect(), terminal, place })
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
        let child = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| self.cannot_run(&e))?;
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
        let settings = Settings { editor: Some("code --goto {file}:{line}".into()), editor_terminal: false };
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
        assert!(editor(&Settings::default(), &[]).is_err());
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
        assert_eq!((settings.editor.as_deref(), settings.editor_terminal), (Some("subl {file}:{line}"), false));
        let settings: Settings = toml::from_str("").expect("defaults");
        assert!(settings.editor.is_none() && settings.editor_terminal);
        assert!(toml::from_str::<Settings>("edtior = \"vim\"").is_err(), "a misspelt key is an error");
    }
}
