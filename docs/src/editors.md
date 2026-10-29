# Editors

tuitor works with your editor both ways:

- **tuitor → editor:** `E` opens the SOURCE line, or the selected breakpoint, in your editor.
- **editor → tuitor:** a key in your editor toggles a breakpoint on the cursor's line, or shows that line in tuitor.

## Open in your editor (`E`)

When tuitor runs in an editor's terminal, the file opens in that editor's window:

| tuitor runs in | detected by | runs |
|---|---|---|
| Neovim `:terminal` | `$NVIM` | that Neovim, in a split beside tuitor |
| Vim `:terminal` (`+clientserver`) | `$VIM_SERVERNAME` | `vim --remote-silent` |
| Kakoune terminal | `$KAKOUNE_SESSION`, `$KAKOUNE_CLIENT` | `edit` in that client |
| Emacs (vterm, eat, term) | `$INSIDE_EMACS` | `emacsclient -n` (needs `server-start`) |
| Zed terminal | `$ZED_TERM` | `zed file:line` |
| VS Code, Cursor, Windsurf, VSCodium terminal | `$TERM_PROGRAM=vscode` | `code --goto file:line` (or that fork's CLI) |

Anywhere else, tuitor uses `$VISUAL` or `$EDITOR`.

To pick an editor yourself, set `editor` in `~/.config/tuitor/config.toml`. You can give just the editor's name, and tuitor passes the file and line the way that editor expects:

```toml
editor = "hx"          # Helix: hx file:line, in tuitor's terminal
# editor = "code"      # VS Code: code --goto file:line, in its own window
# editor = "subl"      # Sublime Text
# editor = "phpstorm"  # any JetBrains IDE: --line N file
```

Or give a full command, using `{file}` and `{line}`:

```toml
editor = "kitty @ launch --type=tab nvim +{line} {file}"
editor_terminal = false   # true: tuitor hands its terminal to the editor until it exits
```

When `editor` is a full command, `editor_terminal` defaults to `true`. When it is just a name, tuitor picks the right value for that editor.

## Breakpoints from your editor

`tuitor send` runs a tuitor command line in the running tuitor:

```sh
tuitor send bp toggle src/main.rs:12   # toggle a breakpoint
tuitor send show src/main.rs:12        # show the line in CODE
tuitor send continue                   # any command works
```

It goes to the tuitor tab whose project contains your current directory. If no tab does, it goes to the tuitor started last. Set `$TUITOR_SOCKET` to pick one tuitor yourself. `tuitor send` prints tuitor's reply and exits with 1 if the command was refused.

Ready-made bindings are in [`editors/`](https://github.com/DorianDevp/tuitor/tree/master/editors). With all of them, F9 (or Alt-F9) toggles a breakpoint and F8 (or Alt-F8) shows the line:

| editor | file | install |
|---|---|---|
| Vim, Neovim | `vim/tuitor.vim` | `source` it from your vimrc / init.vim |
| Emacs | `emacs/tuitor.el` | `(load "…/tuitor.el")` |
| Helix 25.01+ | `helix/config.toml` | merge into `~/.config/helix/config.toml` |
| VS Code, Cursor, VSCodium | `vscode/tasks.json`, `keybindings.json` | copy into your tasks and keybindings |
| Zed | `zed/tasks.json`, `keymap.json` | copy into `~/.config/zed/` |
| Kakoune | `kakoune/tuitor.kak` | `source` it from your kakrc |
| Sublime Text | `sublime/tuitor.py`, `Default.sublime-keymap` | copy into `Packages/User/` |

**JetBrains IDEs:** add an External Tool (Settings → Tools → External Tools) with
Program `tuitor`, Arguments `send bp toggle $FilePath$:$LineNumber$`, and
Working directory `$ProjectFileDir$`. Then give it a key under Keymap → External Tools.

`tuitor send` needs Unix sockets, so it works on Linux and macOS.
