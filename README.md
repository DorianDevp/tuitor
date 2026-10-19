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
tuitor                         # this directory's project (starts its configuration if it has just one)
tuitor "Listen for Xdebug"     # a named configuration
tuitor -C ~/shop/api --open ~/shop/web               # two projects, each in a tab
tuitor listen --port 9003 --map /var/www/app=./api   # PHP, no configuration needed
tuitor exec ./target/debug/app -- --flag              # native program through gdb
tuitor configs                 # what tuitor sees: projects, configurations and adapters
```

`-C <dir>` sets the project directory (default: the current directory) and
`--open <dir>`, repeatable, opens more projects, each in a tab of its own. With
neither, the current directory gets a tab only if it is a project (it has a
launch configuration or a kind tuitor knows, see below); otherwise tuitor starts
with an empty tab, and `O` opens a project in it.

[`examples/`](examples) has a small program for PHP, Rust, C, Node.js, a web
page in Chromium and TypeScript, each with a launch.json, a `run.sh` that sets
it up and starts tuitor on it, and a walk through a first session.

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
To keep the mapping, add it to the project's configuration in CONFIG:
`/config map /var/www/app=./api`.

### Angular

Not verified: no Angular project was tried with tuitor. This is the
configuration VS Code uses for `ng serve`; an Angular project opened without
configurations gets one like it in CONFIG, named `Launch Chrome`:

```jsonc
{
  "name": "ng serve",
  "type": "pwa-chrome",               // or "chrome", as `ng new` writes it
  "request": "launch",
  "url": "http://localhost:4200/",
  "webRoot": "${workspaceFolder}"     // the project root, where src/app/… is
  // "runtimeExecutable": "/usr/bin/chromium"   without Google Chrome
}
```

tuitor does not run VS Code's `preLaunchTask`, so start the dev server first
(`ng serve`), then `tuitor "ng serve"`. Breakpoints go in the `.ts` files under
`src/app`; js-debug finds them in the served bundles through the source maps
that `ng serve` writes.

## Projects and tabs

A tab is one project: its directory, launch configurations, debug sessions,
views and history. The debuggers of different tabs never mix, so an adapter that
fails or exits takes only its own tab's sessions with it. The top bar lists the
tabs on the right, each with the state of its active session: `■` stopped (dim
when it waits in another tab), `▶` running, `⧗` starting, `✕` ended. Names
shorten to fit; the active tab is the highlighted one.

`O` opens a project: pick one of those opened recently, matched as you type, or
type a directory (`~/…`, `/…`, or `../…` from the current project), which `tab`
completes. It opens in a new tab, or in the current one if that is empty; `T`
adds an empty tab and asks the same. `/tab open <dir>` does it from the command
line. `<` and `>` (or `g T` and `g t`) switch tabs, `ctrl-w` closes one and stops
its debuggers, asking first while a session is live. Sessions of one tab stay a
`g s` / `g S` away.

tuitor knows these kinds of project by the files in their directory:

| kind | marker files | configuration it gets |
|---|---|---|
| PHP | `composer.json`, `*.php` | listen for Xdebug on port 9003 |
| Rust | `Cargo.toml` | gdb launch of `target/debug/<package>` |
| C | `Makefile`, `CMakeLists.txt`, `*.c` | gdb launch of an executable built in the directory |
| JS/TS | `package.json`, `angular.json`, `tsconfig.json` | `pwa-chrome` on `http://localhost:4200` with `webRoot` |

### Launch configurations

A project's configurations come from two places, both listed in CONFIG with
where they come from:

- `.vscode/launch.json`, when there is one, read as VS Code reads it. tuitor
  never changes it.
- tuitor's own, kept outside the repository in `~/.config/tuitor/projects.toml`,
  a table per project keyed by its absolute path, with the fields of
  launch.json. One file for all projects is one place to read, back up or carry
  to another machine; tuitor reads it again before every change, so what another
  tuitor wrote for other projects stays.

A project with no configuration in either gets one for its kind (table above)
when it is opened, and opens on CONFIG to adjust what a default cannot know:
the program, its arguments, a port, path mappings.

CONFIG sits next to BREAKPOINTS in the PROJECT view (`g p`; `g b` and `B` show
the same view with BREAKPOINTS focused). It shows the project's path and kind,
its configurations with an `S` by the one `S` starts, and that one's fields.
Moving onto a configuration selects it, `enter` starts it; `enter` on a field
opens the command line with `/config set <field> <value>` to edit. tuitor's own
configurations change with these commands and are saved at once:

```
/config set port 9010                JSON when it reads as JSON, text otherwise
/config set args ["--verbose"]
/config unset stopOnEntry            (u on a field)
/config map /var/www/app=./api       pathMappings; /var/www/app= removes it
/config add rust                     a kind, or a type: php, gdb, lldb, pwa-chrome, pwa-node…
/config delete                       the selected configuration (d)
/config start                        the selected configuration (S)
```

Variables such as `${workspaceFolder}` are expanded when a configuration starts;
its fields then go to the adapter as they are.

## Screen

Views: **overview** (everything), **code** (source + full stack + breakpoints),
**history** (every variable at every stop), **project** (every breakpoint, and
CONFIG). Any pane can be maximised with `z`. BREAKPOINTS lists those of the
active session or, after `a`, all of them: of every session and every tab,
grouped by tab.

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
| `S` | start the configuration selected in CONFIG | | `.` | SOURCE follows execution again |
| `[` `]` `}` | older stop / newer / live | | `tab` | next pane |
| `g o` `g c` `g h` `g p` | views (`g b`: PROJECT at BREAKPOINTS) | | `z` | zoom pane |
| `j` `k` `h` `l`, arrows | move, collapse, expand | | `q` / `esc` | back out (zoom → history → browsing) |
| `<` `>` | previous / next tab (`g T` `g t`) | | `O` / `T` | open a project / in a new tab |
| `g s` `g S` | next / previous session of the tab | | `ctrl-w` | close the tab |

In BREAKPOINTS: `a` session/all, `e` condition, `l` logpoint, `space` on/off, `d` delete.
In CONFIG: `enter` start or edit, `a` add, `m` map a path, `u` unset a field, `d` delete.
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
| `pwa-node`, `node` | `node <adapters>/js-debug/js-debug/src/dapDebugServer.js ${port} 127.0.0.1` | JavaScript, TypeScript in Node.js |
| `pwa-chrome`, `chrome` | the same js-debug | JavaScript, TypeScript in Chrome or Chromium |

js-debug runs each Node process and each browser page in a session of its own,
which it asks tuitor for (`startDebugging`). It shows next to the launch in
the top bar, named after it and the target (`shop·js › server.js [4242]`);
the program stops there, and breakpoints go there. `X` in either stops the
whole program. `node` and `chrome` configurations go to js-debug as
`pwa-node` and `pwa-chrome`, as VS Code sends them. `"console":
"integratedTerminal"` asks for a terminal that tuitor cannot give the
program; with `"internalConsole"`, the default, its output shows in OUTPUT.

Programs are looked up on `PATH` only. Adapters released as VS Code extensions
are unpacked from their official `.vsix` (js-debug: its DAP server's
`.tar.gz`) into `<adapters>`:
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

**js-debug** (JavaScript and TypeScript) is Microsoft's debugger from VS Code;
it runs on Node.js. Take its standalone DAP server,
`js-debug-dap-v<version>.tar.gz`, from its
[GitHub releases](https://github.com/microsoft/vscode-js-debug/releases):

```sh
v=1.140.0    # or a newer release
dir="${XDG_DATA_HOME:-$HOME/.local/share}/tuitor/adapters/js-debug"
rm -rf "$dir" && mkdir -p "$dir"
curl -fL -o "$dir/js-debug-dap.tar.gz" "https://github.com/microsoft/vscode-js-debug/releases/download/v$v/js-debug-dap-v$v.tar.gz"
tar -xzf "$dir/js-debug-dap.tar.gz" -C "$dir" && rm "$dir/js-debug-dap.tar.gz"
```

Browser configurations also need Chrome or Chromium. js-debug looks for Google
Chrome; for Chromium, set `runtimeExecutable` to its path (`/usr/bin/chromium`).

On macOS, use `~/Library/Application Support/tuitor/adapters/…` as `dir`.

Breakpoints and watches are remembered per project in
`~/.local/state/tuitor/`, outside the repository, and so are the projects opened
recently. Logs: `~/.local/state/tuitor/tuitor.log` (`TUITOR_LOG=dap=trace`
records the protocol).

## Code

```
crates/tuitor-dap    DAP framing, message types, async client. Knows nothing about tuitor.
crates/tuitor-core   Engine: sessions, breakpoints, snapshots, deltas. No terminal.
crates/tuitor        The TUI: tabs, commands, keymap, views.
```

Each tab owns an engine, which owns all of its project's debugger state. The
engines are driven from one loop; DAP responses come back as messages, labelled
with their tab on the way into the loop, so there are no locks. The UI renders
every frame from that state. Keys map to command lines, command lines parse into
actions, and actions are applied in one place (`App::apply`).

```sh
cargo test --workspace                       # includes a real gdb session
TUITOR_PHP=/path/to/php-with-xdebug \
TUITOR_PHP_ADAPTER=~/.local/share/tuitor/adapters/vscode-php-debug/extension/out/phpDebug.js \
cargo test -p tuitor-core --test php -- --ignored
TUITOR_JS_ADAPTER=~/.local/share/tuitor/adapters/js-debug/js-debug/src/dapDebugServer.js \
cargo test -p tuitor-core --test js -- --ignored
```

`tuitor --script file` runs commands headless and saves screens as ANSI
(`cat` them); see `crates/tuitor/src/script.rs`.

## Not there yet

- FLOW (file waterfall from the first breakpoint) from the mockups
- `runInTerminal`: programs that need a terminal of their own (`"console": "integratedTerminal"`)
- Locals of a frame other than the top one; sources without a path
- Opening the current location in Neovim
