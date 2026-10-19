<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/logo_white.svg">
  <source media="(prefers-color-scheme: light)" srcset="assets/logo_black.svg">
  <img alt="tuitor" src="assets/logo_black.svg" width="442">
</picture>

# tuitor

A terminal debugger built like a trading terminal: the whole state of a stopped
program on one screen, what changed since the last step highlighted, and every
stop recorded so you can go back and see when a value went wrong.

It speaks the Debug Adapter Protocol, so it works with the adapters VS Code and
Neovim use. Your editor stays an editor; tuitor runs next to it.

```
cargo install --path crates/tuitor
```

## Start

```sh
tuitor                         # uses .vscode/launch.json (starts it if there is one config)
tuitor "Listen for Xdebug"     # a named configuration
tuitor listen --port 9003 --map /var/www/app=./api   # PHP, no launch.json needed
tuitor exec ./target/debug/app -- --flag              # native program through gdb
tuitor configs                 # what tuitor sees: configurations and adapters
```

`-C <dir>` sets the workspace (default: current directory).

[`examples/`](examples) has a small program for PHP, Rust and C, each with a
launch.json, a `run.sh` that sets it up and starts tuitor on it, and a walk
through a first session.

### PHP in a container

Xdebug connects *out* of the container to tuitor on the host:

```yaml
# compose.yaml
services:
  php:
    extra_hosts: ["host.docker.internal:host-gateway"]   # needed on Linux
    environment:
      XDEBUG_MODE: ${XDEBUG_MODE:-off}                    # XDEBUG_MODE=debug docker compose up -d
      XDEBUG_CONFIG: client_host=host.docker.internal client_port=9003
```

Then `tuitor listen --map /var/www/app=./api` (container path = local path) and
trigger a request with the Xdebug cookie, or `XDEBUG_TRIGGER=1` for CLI scripts.

## Screen

Views: **overview** (everything), **code** (source + full stack + breakpoints),
**history** (every variable at every stop), **breakpoints** (full list).
Any pane can be maximised with `z`.

Bright colour means something: amber is where execution is, green is what
changed in this step (`▲`, with the previous value), red is a hit breakpoint or
an error. Everything else is grey. A dim green `△` marks a variable that reads
the same but changed inside, such as an object whose field changed or a pointer
whose target did; expand it to see what.

`[` and `]` step through recorded stops. An older stop is shown read-only with
the values captured at that moment; nothing is re-read from the program.

## Keys

Every key runs a command; `/` opens the command line with the same commands and
`?` lists them with their keys.

| key | command | | key | command |
|---|---|---|---|---|
| `c` | continue | | `b` | toggle breakpoint on the SOURCE cursor |
| `n` | next (step over) | | `f` | find files and breakpoints (`name:line`) |
| `s` | step in | | `w` | watch an expression (selected variable in LOCALS) |
| `o` | step out | | `x` | evaluate |
| `p` / `X` / `R` | pause / stop / restart | | `y` | copy the selection (OSC 52) |
| `[` `]` `}` | older stop / newer / live | | `.` | SOURCE follows execution again |
| `g o` `g c` `g h` `g b` | views | | `tab` | next pane |
| `j` `k` `h` `l`, arrows | move, collapse, expand | | `z` | zoom pane |
| `<` `>` | previous / next session | | `q` / `esc` | back out (zoom → history → browsing) |

In BREAKPOINTS: `a` session/all, `e` condition, `l` logpoint, `space` on/off, `d` delete.
In STACK: `v` library frames. In OUTPUT: `C` clear.

### Changing keys

Copy `crates/tuitor/src/default_keys.toml` to `~/.config/tuitor/keys.toml`. A
binding is a key and a command line; `""` unbinds:

```toml
[global]
m = "continue"
c = ""
"g f" = "find"

[pane.locals]
enter = "open"
```

## Adapters

| type | runs | debugs |
|---|---|---|
| `php` | `node <adapters>/vscode-php-debug/extension/out/phpDebug.js` | PHP with Xdebug |
| `gdb` | `gdb --interpreter=dap` | C, C++, Rust, Zig |
| `lldb-dap` | `lldb-dap` | C, C++, Rust, Zig |
| `lldb` | `<adapters>/codelldb/extension/adapter/codelldb --port ${port}` (CodeLLDB) | C, C++, Rust, Zig |

Programs are looked up on `PATH` only. Adapters released as VS Code extensions
are unpacked from their official `.vsix` into `<adapters>`:
`${XDG_DATA_HOME:-~/.local/share}/tuitor/adapters` (macOS:
`~/Library/Application Support/tuitor/adapters`). `tuitor configs` shows what
is installed and what is missing.

Override or add in `~/.config/tuitor/adapters.toml`:

```toml
[php]
command = "node"
args = ["/opt/vscode-php-debug/extension/out/phpDebug.js"]
extensions = ["php"]        # breakpoints in these files go to this adapter
```

`transport` is `stdio` (default), `server` (adapter listens on `${port}`) or
`tcp` (connect to `address`).

### Installing adapters

Each from where its authors publish it.

**PHP** needs three things:

- Xdebug 3, loaded by PHP (`php -v` mentions it): your distribution's package
  (`apt-get install php-xdebug`, `dnf install php-xdebug`, `pacman -S xdebug`)
  or `pie install xdebug/xdebug`; see [xdebug.org/docs/install](https://xdebug.org/docs/install).
  Arch's package installs `/etc/php/conf.d/xdebug.ini` with the extension commented
  out; uncomment `zend_extension=xdebug.so` there.
- Node.js 20.18 or newer: [nodejs.org/en/download](https://nodejs.org/en/download)
  or your distribution's `nodejs`.
- vscode-php-debug, from its [GitHub releases](https://github.com/xdebug/vscode-php-debug/releases):

  ```sh
  v=1.40.2    # or a newer release
  dir="${XDG_DATA_HOME:-$HOME/.local/share}/tuitor/adapters/vscode-php-debug"
  rm -rf "$dir" && mkdir -p "$dir"
  curl -fL -o "$dir/php-debug.vsix" "https://github.com/xdebug/vscode-php-debug/releases/download/v$v/php-debug-$v.vsix"
  unzip -q "$dir/php-debug.vsix" -d "$dir" && rm "$dir/php-debug.vsix"
  ```

**gdb** 14 or newer, built with Python (its DAP server is written in Python):
your distribution's `gdb` (Debian 13, Ubuntu 24.04 and Arch qualify; Debian 12's
13.1 does not) or the [GNU release](https://www.sourceware.org/gdb/download/).

**lldb-dap** comes with LLDB: `pacman -S lldb`, or the `LLVM-<version>-<platform>`
archive from [LLVM's releases](https://github.com/llvm/llvm-project/releases)
(`bin/lldb-dap`). Debian and Ubuntu install it as `lldb-dap-NN` with
`lldb-NN`; link it as `lldb-dap` into a directory on `PATH`.

**CodeLLDB** brings its own LLDB. Take the `.vsix` for your platform
(`codelldb-linux-x64`, `-linux-arm64`, `-darwin-arm64`, …) from its
[GitHub releases](https://github.com/vadimcn/codelldb/releases):

```sh
dir="${XDG_DATA_HOME:-$HOME/.local/share}/tuitor/adapters/codelldb"
rm -rf "$dir" && mkdir -p "$dir"
curl -fL -o "$dir/codelldb.vsix" https://github.com/vadimcn/codelldb/releases/latest/download/codelldb-linux-x64.vsix
unzip -q "$dir/codelldb.vsix" -d "$dir" && rm "$dir/codelldb.vsix"
```

On macOS, use `~/Library/Application Support/tuitor/adapters/…` as `dir`.

Breakpoints and watches are remembered per workspace in
`~/.local/state/tuitor/`, outside the repository. Logs:
`~/.local/state/tuitor/tuitor.log` (`TUITOR_LOG=dap=trace` records the protocol).

## Code

```
crates/tuitor-dap    DAP framing, message types, async client. Knows nothing about tuitor.
crates/tuitor-core   Engine: sessions, breakpoints, snapshots, deltas. No terminal.
crates/tuitor        The TUI: commands, keymap, views.
```

The engine owns all state and is driven from one loop; DAP responses come back
as messages, so there are no locks. The UI renders every frame from that state.
Keys map to command lines, command lines parse into actions, and actions are
applied in one place (`App::apply`).

```sh
cargo test --workspace                       # includes a real gdb session
TUITOR_PHP=/path/to/php-with-xdebug \
TUITOR_PHP_ADAPTER=~/.local/share/tuitor/adapters/vscode-php-debug/extension/out/phpDebug.js \
cargo test -p tuitor-core --test php -- --ignored
```

`tuitor --script file` runs commands headless and saves screens as ANSI
(`cat` them); see `crates/tuitor/src/script.rs`.

## Not there yet

- FLOW (file waterfall from the first breakpoint) and LINKED (front end and back end of one request) from the mockups
- JavaScript/TypeScript through js-debug (needs child sessions, `startDebugging`)
- Locals of a frame other than the top one; sources without a path
- Opening the current location in Neovim
