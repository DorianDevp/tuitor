//! The language servers tuitor knows, and how to run each: the `[lsp]` table of config.toml
//! over what PATH has.

use std::path::Path;

use serde::Deserialize;
use serde_json::{Value, json};

/// A language with a server tuitor runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Php,
    /// TypeScript and JavaScript, Angular's included.
    TypeScript,
    Rust,
    Go,
}

impl Language {
    pub fn of(path: &Path) -> Option<Language> {
        Some(match path.extension()?.to_str()? {
            "php" => Language::Php,
            "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" => Language::TypeScript,
            "rs" => Language::Rust,
            "go" => Language::Go,
            _ => return None,
        })
    }

    /// The table of `[lsp]` that configures its server.
    pub fn key(self) -> &'static str {
        match self {
            Language::Php => "php",
            Language::TypeScript => "typescript",
            Language::Rust => "rust",
            Language::Go => "go",
        }
    }

    /// The `languageId` a document of `path` is opened with.
    pub fn document(path: &Path) -> &'static str {
        match path.extension().and_then(|e| e.to_str()).unwrap_or_default() {
            "php" => "php",
            "tsx" => "typescriptreact",
            "js" | "mjs" | "cjs" => "javascript",
            "jsx" => "javascriptreact",
            "rs" => "rust",
            "go" => "go",
            _ => "typescript",
        }
    }

    /// The server, as its authors tell to run it.
    fn default_server(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Language::Php => ("intelephense", &["--stdio"]),
            Language::TypeScript => ("typescript-language-server", &["--stdio"]),
            Language::Rust => ("rust-analyzer", &[]),
            Language::Go => ("gopls", &[]),
        }
    }
}

/// One server: what runs it and what it is initialized with. A field left out is the default.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// A program on PATH or a path to one.
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    /// `initializationOptions`, as the server documents them.
    pub initialization_options: Option<Value>,
}

/// The `[lsp]` table: a table per language, and no other languages.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Servers {
    pub php: Option<ServerConfig>,
    pub typescript: Option<ServerConfig>,
    pub rust: Option<ServerConfig>,
    pub go: Option<ServerConfig>,
}

/// How to run a server, defaults filled in.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    pub initialization_options: Value,
}

impl Servers {
    pub fn command(&self, language: Language) -> Command {
        let given = match language {
            Language::Php => &self.php,
            Language::TypeScript => &self.typescript,
            Language::Rust => &self.rust,
            Language::Go => &self.go,
        };
        let given = given.clone().unwrap_or_default();
        let (program, args) = language.default_server();
        Command {
            program: given.command.unwrap_or_else(|| program.to_owned()),
            args: given.args.unwrap_or_else(|| args.iter().map(|a| (*a).to_owned()).collect()),
            initialization_options: given.initialization_options.unwrap_or_else(|| default_options(language)),
        }
    }
}

fn default_options(language: Language) -> Value {
    match language {
        // tuitor shows no diagnostics, so `cargo check` on start would only take time and
        // write to target/.
        Language::Rust => json!({ "checkOnSave": false }),
        Language::Php | Language::TypeScript | Language::Go => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_overrides_the_fields_it_names() {
        let servers: Servers = toml::from_str(
            r#"
            [php]
            initialization_options = { licenceKey = "/home/me/intelephense/licence.txt", storagePath = "/tmp/i" }
            [rust]
            command = "mise"
            args = ["exec", "--", "rust-analyzer"]
            "#,
        )
        .expect("servers");
        let php = servers.command(Language::Php);
        assert_eq!((php.program.as_str(), php.args.as_slice()), ("intelephense", &["--stdio".to_owned()][..]));
        assert_eq!(php.initialization_options["licenceKey"], "/home/me/intelephense/licence.txt");
        let rust = servers.command(Language::Rust);
        assert_eq!((rust.program.as_str(), rust.args.join(" ")), ("mise", "exec -- rust-analyzer".to_owned()));
        assert_eq!(rust.initialization_options, json!({ "checkOnSave": false }));
        assert_eq!(servers.command(Language::Go).program, "gopls");
    }

    #[test]
    fn only_the_four_languages_and_their_fields() {
        assert!(toml::from_str::<Servers>("[python]\ncommand = \"pyright\"").is_err());
        assert!(toml::from_str::<Servers>("[go]\ncomand = \"gopls\"").is_err());
    }

    #[test]
    fn files_go_to_their_language() {
        assert_eq!(Language::of(Path::new("src/app/app.ts")), Some(Language::TypeScript));
        assert_eq!(Language::of(Path::new("orders.php")), Some(Language::Php));
        assert_eq!(Language::of(Path::new("main.c")), None);
        assert_eq!(Language::document(Path::new("a.tsx")), "typescriptreact");
    }
}
