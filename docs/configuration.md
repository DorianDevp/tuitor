# Configuration

## Launch configurations

CONFIG (`g p`, beside BREAKPOINTS) lists the project's launch configurations from two places:
`.vscode/launch.json`, read as VS Code reads it and never written, and tuitor's own, kept
outside the repository in `~/.config/tuitor/projects.toml` by project path. `enter` starts
one, or edits the field under the cursor; tuitor's own change with these commands and are
saved at once:

```
/config set port 9010                JSON when it reads as JSON, text otherwise
/config unset stopOnEntry            (u)
/config map /var/www/app=./api       pathMappings; /var/www/app= removes it (m)
/config mappings-from-compose [svc]  pathMappings from a compose service's bind mounts (M)
/config add rust                     a kind, or a type: php, gdb, lldb, go, pwa-chrome, pwa-node… (a)
/config delete                       (d)
/config start                        (S)
```

A project with no configuration gets one for its kind when it is opened, and opens on
CONFIG to adjust what a default cannot know:

| kind | found by | configuration |
|---|---|---|
| PHP | `composer.json`, `*.php` | listen for Xdebug on port 9003 |
| Rust | `Cargo.toml` | gdb launch of `target/debug/<package>` |
| Go | `go.mod` | Delve builds and launches the package |
| C | `Makefile`, `CMakeLists.txt`, `*.c` | gdb launch of an executable built in the directory |
| JS/TS | `package.json`, `angular.json`, `tsconfig.json` | `pwa-chrome` on `http://localhost:4200`, in the browser found on `PATH` |

## Files

| file | |
|---|---|
| `~/.config/tuitor/keys.toml` | key bindings over the [defaults](../crates/tuitor/src/default_keys.toml) |
| `~/.config/tuitor/adapters.toml` | adapters: replace a built-in one, or add one |
| `~/.config/tuitor/projects.toml` | tuitor's own launch configurations, by project path; written by `/config` |
| `~/.config/tuitor/config.toml` | the editor `E` runs; the language servers `P` asks |
| `~/.local/state/tuitor/` | breakpoints and their sets, watches, recent projects and runs; `tuitor.log` |
| `~/.local/share/tuitor/adapters/` | what `tuitor install` unpacks |

The directories follow `$XDG_CONFIG_HOME`, `$XDG_STATE_HOME` and `$XDG_DATA_HOME` (macOS:
`~/Library/Application Support/tuitor`). `TUITOR_LOG=dap=trace` logs the protocol.

## keys.toml

A binding is a key, or a sequence of keys, and a command line; `""` unbinds. `[global]`
applies everywhere, `[pane.<name>]` while that pane has focus, over `[global]`:

```toml
[global]
m = "continue"
c = ""
"g n" = "find"

[pane.locals]
enter = "open"
```

## adapters.toml

A table replaces the built-in adapter of that name; `transport` is `stdio` (the default),
`server` (the adapter listens on `${port}`) or `tcp` (tuitor connects to `address`).
[adapters.md](adapters.md) lists the built-in ones.

```toml
[php]
command = "node"
args = ["/opt/vscode-php-debug/extension/out/phpDebug.js"]
extensions = ["php"]        # breakpoints in these files go to this adapter
```

## config.toml

```toml
# Split at spaces, {file} and {line} replaced in each word.
editor = "code --goto {file}:{line}"
editor_terminal = false     # it has a window of its own; true (the default) hands it the terminal

# The language server of each language, if not the one on PATH: command, args and the
# server's initializationOptions; a field left out keeps the default.
[lsp.php]                   # intelephense --stdio; PATHS needs no licence, but takes one
initialization_options = { licenceKey = "/home/me/intelephense/licence.txt" }

[lsp.rust]                  # rust-analyzer, with { checkOnSave = false }
command = "mise"
args = ["exec", "--", "rust-analyzer"]

[lsp.typescript]            # typescript-language-server --stdio

[lsp.go]                    # gopls
```

Without `editor`, `E` opens the Neovim tuitor runs in, beside tuitor, else `$VISUAL` or
`$EDITOR` in tuitor's terminal until it exits.
