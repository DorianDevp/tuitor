//! Key bindings: key sequences → command lines, global and per pane.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use anyhow::{Context, Result, bail};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const DEFAULTS: &str = include_str!("default_keys.toml");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    code: KeyCodeOrd,
    ctrl: bool,
    alt: bool,
    /// Only for the keys in `SHIFTED`; a character has it in itself.
    shift: bool,
}

/// `KeyCode` is not `Ord`; this wraps the variants bindings can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum KeyCodeOrd {
    Char(char),
    Named(Named),
    F(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Named {
    Enter,
    Esc,
    Tab,
    BackTab,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    Delete,
}

/// Keys that `shift-` can go with: a shifted character is another character, and the
/// terminal reports shift-tab as `backtab`.
const SHIFTED: &[Named] =
    &[Named::Up, Named::Down, Named::Left, Named::Right, Named::PageUp, Named::PageDown, Named::Home, Named::End];

const NAMES: &[(&str, Named)] = &[
    ("enter", Named::Enter),
    ("esc", Named::Esc),
    ("tab", Named::Tab),
    ("backtab", Named::BackTab),
    ("backspace", Named::Backspace),
    ("up", Named::Up),
    ("down", Named::Down),
    ("left", Named::Left),
    ("right", Named::Right),
    ("pageup", Named::PageUp),
    ("pagedown", Named::PageDown),
    ("home", Named::Home),
    ("end", Named::End),
    ("delete", Named::Delete),
];

impl Key {
    pub fn from_event(event: KeyEvent) -> Option<Key> {
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
        let alt = event.modifiers.contains(KeyModifiers::ALT);
        let shift = event.modifiers.contains(KeyModifiers::SHIFT);
        let code = match event.code {
            // Shift is already in the character ('G'), so it is not tracked separately.
            KeyCode::Char(' ') => KeyCodeOrd::Char(' '),
            KeyCode::Char(c) => KeyCodeOrd::Char(if ctrl { c.to_ascii_lowercase() } else { c }),
            KeyCode::Enter => KeyCodeOrd::Named(Named::Enter),
            KeyCode::Esc => KeyCodeOrd::Named(Named::Esc),
            KeyCode::Tab => KeyCodeOrd::Named(Named::Tab),
            KeyCode::BackTab => KeyCodeOrd::Named(Named::BackTab),
            KeyCode::Backspace => KeyCodeOrd::Named(Named::Backspace),
            KeyCode::Up => KeyCodeOrd::Named(Named::Up),
            KeyCode::Down => KeyCodeOrd::Named(Named::Down),
            KeyCode::Left => KeyCodeOrd::Named(Named::Left),
            KeyCode::Right => KeyCodeOrd::Named(Named::Right),
            KeyCode::PageUp => KeyCodeOrd::Named(Named::PageUp),
            KeyCode::PageDown => KeyCodeOrd::Named(Named::PageDown),
            KeyCode::Home => KeyCodeOrd::Named(Named::Home),
            KeyCode::End => KeyCodeOrd::Named(Named::End),
            KeyCode::Delete => KeyCodeOrd::Named(Named::Delete),
            KeyCode::F(n) => KeyCodeOrd::F(n),
            _ => return None,
        };
        let shift = shift && matches!(code, KeyCodeOrd::Named(named) if SHIFTED.contains(&named));
        Some(Key { code, ctrl, alt, shift })
    }

    pub fn parse(text: &str) -> Result<Key> {
        let mut rest = text;
        let (mut ctrl, mut alt, mut shift) = (false, false, false);
        loop {
            if let Some(r) = rest
                .strip_prefix("ctrl-")
                .or_else(|| rest.strip_prefix("c-"))
                .or_else(|| rest.strip_prefix('^'))
                .filter(|r| !r.is_empty())
            {
                ctrl = true;
                rest = r;
            } else if let Some(r) =
                rest.strip_prefix("alt-").or_else(|| rest.strip_prefix("m-")).filter(|r| !r.is_empty())
            {
                alt = true;
                rest = r;
            } else if let Some(r) = rest.strip_prefix("shift-").filter(|r| !r.is_empty()) {
                shift = true;
                rest = r;
            } else {
                break;
            }
        }
        let mut chars = rest.chars();
        let code = match (chars.next(), chars.next()) {
            (Some(c), None) => KeyCodeOrd::Char(if ctrl { c.to_ascii_lowercase() } else { c }),
            _ if rest == "space" => KeyCodeOrd::Char(' '),
            _ => {
                if let Some(n) = rest.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
                    KeyCodeOrd::F(n)
                } else {
                    let named = NAMES.iter().find(|(name, _)| *name == rest).map(|(_, n)| *n);
                    KeyCodeOrd::Named(named.with_context(|| format!("unknown key `{text}`"))?)
                }
            }
        };
        if shift && !matches!(code, KeyCodeOrd::Named(named) if SHIFTED.contains(&named)) {
            bail!(
                "`{text}`: shift- goes with arrows, pageup, pagedown, home and end; a shifted character is typed as itself (`G`)"
            );
        }
        Ok(Key { code, ctrl, alt, shift })
    }

    /// Printable characters typed into an input field.
    pub fn text_char(&self) -> Option<char> {
        match self.code {
            KeyCodeOrd::Char(c) if !self.ctrl && !self.alt => Some(c),
            _ => None,
        }
    }

    pub fn is(&self, name: &str) -> bool {
        Key::parse(name).is_ok_and(|k| k == *self)
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            f.write_str("^")?;
        }
        if self.alt {
            f.write_str("M-")?;
        }
        if self.shift {
            f.write_str("S-")?;
        }
        match self.code {
            KeyCodeOrd::Char(' ') => f.write_str("space"),
            KeyCodeOrd::Char(c) => write!(f, "{c}"),
            KeyCodeOrd::F(n) => write!(f, "F{n}"),
            KeyCodeOrd::Named(Named::Enter) => f.write_str("⏎"),
            KeyCodeOrd::Named(n) => {
                let name = NAMES.iter().find(|(_, x)| *x == n).map_or("?", |(name, _)| name);
                f.write_str(name)
            }
        }
    }
}

pub fn label(keys: &[Key]) -> String {
    keys.iter().map(Key::to_string).collect::<Vec<_>>().join(" ")
}

pub enum Lookup<'a> {
    Command(&'a str),
    /// More keys are needed to complete a sequence.
    Prefix,
    None,
}

type Table = BTreeMap<Vec<Key>, String>;

#[derive(Debug, Default)]
pub struct Keymap {
    global: Table,
    panes: BTreeMap<String, Table>,
}

impl Keymap {
    pub fn defaults() -> Keymap {
        let mut map = Keymap::default();
        #[allow(clippy::expect_used)]
        map.merge(DEFAULTS).expect("built-in key bindings are valid");
        map
    }

    /// Defaults with the user's file merged on top.
    pub fn load(user_file: Option<&Path>) -> Result<Keymap> {
        let mut map = Keymap::defaults();
        if let Some(path) = user_file.filter(|p| p.exists()) {
            let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
            map.merge(&text).with_context(|| format!("in {}", path.display()))?;
        }
        Ok(map)
    }

    fn merge(&mut self, text: &str) -> Result<()> {
        let doc: toml::Table = toml::from_str(text)?;
        for (section, value) in doc {
            let toml::Value::Table(entries) = value else { bail!("`{section}` must be a table") };
            if section == "global" {
                merge_table(&mut self.global, &entries)?;
            } else if section == "pane" {
                for (pane, value) in entries {
                    let toml::Value::Table(entries) = value else { bail!("`pane.{pane}` must be a table") };
                    merge_table(self.panes.entry(pane).or_default(), &entries)?;
                }
            } else {
                bail!("unknown section `{section}` (use [global] or [pane.<name>])");
            }
        }
        Ok(())
    }

    pub fn lookup(&self, pane: &str, keys: &[Key]) -> Lookup<'_> {
        let tables = self.panes.get(pane).into_iter().chain(std::iter::once(&self.global));
        let mut prefix = false;
        for table in tables {
            if let Some(command) = table.get(keys) {
                return Lookup::Command(command);
            }
            prefix |= table.range(keys.to_vec()..).next().is_some_and(|(k, _)| k.starts_with(keys));
        }
        if prefix { Lookup::Prefix } else { Lookup::None }
    }

    /// Shortest key sequence bound to exactly this command line in `pane`: a global key
    /// the pane binds to something else does not count.
    pub fn key_for(&self, pane: &str, command: &str) -> Option<String> {
        let own = self.panes.get(pane);
        let global = self.global.iter().filter(|(keys, _)| !own.is_some_and(|t| t.contains_key(*keys)));
        own.into_iter()
            .flat_map(|t| t.iter())
            .chain(global)
            .filter(|(_, c)| c.as_str() == command)
            .min_by_key(|(k, _)| (k.len(), label(k).len()))
            .map(|(k, _)| label(k))
    }

    /// Every key for a command in any table, for the help listing.
    pub fn any_key_for(&self, command: &str) -> Option<String> {
        self.key_for("", command).or_else(|| {
            self.panes.values().flat_map(|t| t.iter()).find(|(_, c)| c.as_str() == command).map(|(k, _)| label(k))
        })
    }
}

fn merge_table(table: &mut Table, entries: &toml::Table) -> Result<()> {
    for (keys, command) in entries {
        let toml::Value::String(command) = command else { bail!("binding `{keys}` must be a string") };
        let sequence = keys.split_whitespace().map(Key::parse).collect::<Result<Vec<_>>>()?;
        if command.is_empty() {
            table.remove(&sequence);
        } else {
            if crate::command::split(command).is_none() {
                bail!("binding `{keys}`: unknown command `{command}`");
            }
            table.insert(sequence, command.clone());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(text: &str) -> Vec<Key> {
        text.split_whitespace().map(|k| Key::parse(k).expect("key")).collect()
    }

    #[test]
    fn defaults_parse_and_resolve() {
        let map = Keymap::defaults();
        assert!(matches!(map.lookup("source", &keys("c")), Lookup::Command("continue")));
        assert!(matches!(map.lookup("source", &keys("g")), Lookup::Prefix));
        assert!(matches!(map.lookup("source", &keys("g c")), Lookup::Command("view code")));
        assert!(matches!(map.lookup("source", &keys("ctrl-c")), Lookup::Command("quit")));
    }

    #[test]
    fn pane_bindings_win_over_global() {
        let map = Keymap::defaults();
        assert!(matches!(map.lookup("breakpoints", &keys("l")), Lookup::Command("bp log")));
        assert!(matches!(map.lookup("locals", &keys("l")), Lookup::Command("right")));
    }

    #[test]
    fn paths_has_keys_of_its_own_and_moves_as_everywhere() {
        let map = Keymap::defaults();
        for (key, command) in [
            ("m", "paths mark"),
            ("space", "paths stage"),
            ("b", "paths branch"),
            ("u", "paths unmark"),
            ("]", "paths next"),
            ("[", "paths prev"),
            ("enter", "paths take"),
            ("j", "down"),
            ("h", "left"),
            ("l", "right"),
            ("q", "close"),
        ] {
            assert!(matches!(map.lookup("paths", &keys(key)), Lookup::Command(c) if c == command), "{key}");
        }
        // Elsewhere those keys keep what they do.
        assert!(matches!(map.lookup("source", &keys("b")), Lookup::Command("bp toggle")));
        assert!(matches!(map.lookup("source", &keys("[")), Lookup::Command("history prev")));
        assert_eq!(map.key_for("paths", "paths take").as_deref(), Some("⏎"));
    }

    #[test]
    fn user_file_overrides_and_unbinds() {
        let mut map = Keymap::defaults();
        map.merge("[global]\nc = \"\"\nm = \"continue\"\n").expect("merge");
        assert!(matches!(map.lookup("source", &keys("c")), Lookup::None));
        assert_eq!(map.key_for("source", "continue").as_deref(), Some("m"));
        assert!(map.merge("[global]\nm = \"frobnicate\"\n").is_err());
    }

    #[test]
    fn key_notation() {
        assert_eq!(Key::parse("^n").expect("^n"), Key::parse("ctrl-n").expect("ctrl-n"));
        assert_eq!(Key::parse("space").expect("space").to_string(), "space");
        assert_eq!(Key::parse("enter").expect("enter").to_string(), "⏎");
        assert!(Key::parse("hyper-x").is_err());
    }

    #[test]
    fn shift_counts_with_arrows_and_paging_keys_only() {
        let event = |code, modifiers| Key::from_event(KeyEvent::new(code, modifiers));
        let shift_down = Key::parse("shift-down").expect("shift-down");
        assert_eq!(event(KeyCode::Down, KeyModifiers::SHIFT), Some(shift_down));
        assert_ne!(shift_down, Key::parse("down").expect("down"));
        assert_eq!(shift_down.to_string(), "S-down");
        // Shift-tab arrives as backtab, and a shifted character is another one.
        assert_eq!(event(KeyCode::BackTab, KeyModifiers::SHIFT), Key::parse("backtab").ok());
        assert_eq!(event(KeyCode::Char('G'), KeyModifiers::SHIFT), Key::parse("G").ok());
        assert_eq!(event(KeyCode::Backspace, KeyModifiers::SHIFT), Key::parse("backspace").ok());
        assert!(Key::parse("shift-g").is_err());
        assert!(Key::parse("shift-enter").is_err());
    }

    #[test]
    fn source_searches_on_slash_as_in_vim() {
        let map = Keymap::defaults();
        assert!(matches!(map.lookup("source", &keys("/")), Lookup::Command("search")));
        assert!(matches!(map.lookup("source", &keys(":")), Lookup::Command("palette")));
        assert!(matches!(map.lookup("locals", &keys("/")), Lookup::Command("palette")));
        assert!(matches!(map.lookup("source", &keys("shift-down")), Lookup::Command("scroll down")));
        // The hints name the key that does it in the pane: not `/`, which SOURCE takes.
        assert_eq!(map.key_for("source", "palette").as_deref(), Some(":"));
        assert_eq!(map.key_for("locals", "palette").as_deref(), Some("/"));
        assert_eq!(map.key_for("source", "search").as_deref(), Some("/"));
    }
}
