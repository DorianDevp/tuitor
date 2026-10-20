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
tuitor doctor                  # what debugging needs here, and what is missing
tuitor install php             # an adapter from its official release, after asking
```

`-C <dir>` sets the project directory (default: the current directory) and
`--open <dir>`, repeatable, opens more projects, each in a tab of its own. With
neither, the current directory gets a tab only if it is a project (it has a
launch configuration or a kind tuitor knows, see below); otherwise tuitor starts
with an empty tab, and `O` opens a project in it.

### Examples

[`examples/`](examples) has one small program, which totals three orders, for
each kind of project. Each example has a `.vscode/launch.json`, a README with a
first session and the steps by hand, and a `run.sh` that checks what the
example needs (offering to install js-debug, vscode-php-debug or Delve as
[Installing adapters](#installing-adapters) does), builds tuitor from this
repository and starts it on the example.

| example | debugged through | run |
|---|---|---|
| [`php`](examples/php) | Xdebug and vscode-php-debug, on the host or in Docker | `./run.sh`, `PHP=/path/to/php ./run.sh`, `TRACE=1 ./run.sh` |
| [`js`](examples/js) | js-debug, in Node.js | `./run.sh` |
| [`typescript`](examples/typescript) | js-debug, in Node.js, through `tsc`'s source map | `./run.sh` |
| [`web`](examples/web) | js-debug, a page from disk in Chromium | `./run.sh`, `HEADLESS=1 ./run.sh` |
| [`angular`](examples/angular) | js-debug, `ng serve`'s page in Chromium | `./run.sh`, `HEADLESS=1 ./run.sh` |
| [`go`](examples/go) | Delve | `./run.sh` |
| [`c`](examples/c) | gdb | `./run.sh` |
| [`rust`](examples/rust) | gdb | `./run.sh` |

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

`/config mappings-from-compose` (`M` in CONFIG) takes them from the compose
file of the project, or of the nearest directory above it that has one: it
asks `docker compose config` for the bind mounts of the service that runs PHP
(by its name, image or Dockerfile), or of the one named, and maps each to
where it is here. Without docker it reads `compose.yaml` and its override
itself, with `${VAR}`s from the environment and `.env`; a project that uses
`include`, `extends` or `COMPOSE_FILE` needs docker. Named volumes and mounts
of single files are left out. Mounts that share a root here and in the
container (`./api/src:/var/www/app/src`, `./api/config:/var/www/app/config`, …)
become one mapping of the roots, `/var/www/app=./api`, but only when nothing
else under `./api` holds code: a `vendor` here that is not mounted is not the
one in the container, and then every mount keeps a mapping of its own. The
mappings go into the selected configuration, which has to be tuitor's own.

### Angular

[`examples/angular`](examples/angular) is an app made by Angular CLI 22, with
a breakpoint in its component that stops while the page loads and on a click.
This is its configuration for `ng serve`; an Angular project opened without
configurations gets one like it in CONFIG, named `Launch Chrome`:

```jsonc
{
  "name": "ng serve",
  "type": "pwa-chrome",                      // or "chrome", as `ng new` writes it
  "request": "launch",
  "url": "http://localhost:4200",
  "webRoot": "${workspaceFolder}",           // the project root, where src/app/… is
  "runtimeExecutable": "/usr/bin/chromium"   // without Google Chrome
}
```

tuitor does not run VS Code's `preLaunchTask`, so start the dev server first
(`ng serve`), then `tuitor "ng serve"`. Breakpoints go in the `.ts` files under
`src/app`, and stops, STACK and stepping show those files, down to the line of
the template's `(click)` that called the component; Angular's own frames fold
into one line, `@angular/core · @angular/platform-browser`. No `sourceMapPathOverrides` are needed: the dev
server of Angular's application builder (`@angular/build:dev-server`) sends the
page's `main.js` with a source map that names `src/app/app.ts` and the others
relative to the page, and `webRoot` puts them in the project.

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
| Go | `go.mod` | Delve builds and launches the package in the directory |
| C | `Makefile`, `CMakeLists.txt`, `*.c` | gdb launch of an executable built in the directory |
| JS/TS | `package.json`, `angular.json`, `tsconfig.json` | `pwa-chrome` on `http://localhost:4200` with `webRoot`, in the browser found on `PATH` |

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
/config mappings-from-compose [service]   pathMappings from a compose service's bind mounts (M)
/config add rust                     a kind, or a type: php, gdb, lldb, go, pwa-chrome, pwa-node…
/config delete                       the selected configuration (d)
/config start                        the selected configuration (S)
```

Variables such as `${workspaceFolder}` are expanded when a configuration starts;
its fields then go to the adapter as they are.

## Screen

Views: **overview** (everything), **code** (source + full stack + breakpoints,
with FLOW and OUTLINE beside them), **history** (every variable at every stop),
**flow** (the files of the run, below), **project** (every breakpoint, and
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

LOCALS shows the variables of the frame selected in STACK, and SOURCE their
values at its line; LOCALS names a frame other than the top one. The top
frame's variables are read at every stop, another frame's once it is selected
while the program is stopped there, so a recorded stop has those of the frames
looked at then. `▲` and `△` compare a frame with the same call at the previous
stop. `x` evaluates in the selected frame; watches are evaluated in the top
frame, so that each is one series from stop to stop.

A frame whose source the adapter has itself, such as Node's internals or code
generated at run time, shows the adapter's text in SOURCE, marked `from
adapter`. It is read once per session, and is read-only: breakpoints go to
adapters by file path, and it has none here. A path that is not on this
machine, as a program in a container reports one, comes with the `/config map`
that maps it to the project.

### FLOW

FLOW (`g f`) lists the files of a run in the order execution first reached them,
from its first stop, with the functions seen in each: `✓` ran, `▶` executes at
the stop on view, `▷` is further down its stack, `◇` is predicted. Each file has
a bar over stop numbers, or over time when a trace has it, with the stop on view
marked `▼` and breakpoints where the evidence reaches their lines. Library files
of one package that follow one another are one line (`v` lists them). SELECTED
shows what is known of the file under the cursor, and its code; EVIDENCE where
the rows come from:

- **This run**: every file on the stack of every recorded stop, for any adapter.
- **An earlier run**: tuitor keeps the files of recent runs in
  `~/.local/state/tuitor/flows/`. A run that started at the same first stop
  (and, once this run has hit a breakpoint, stopped there too) predicts the
  files it reached after the point this run is at, lined up at the stop where it
  was where this run is. While vscode-php-debug listens, each request is a run.
- **An Xdebug function trace** (PHP): the newest finished trace in the
  configuration's `xdebugTraceDir` that passes through the stack of the stop on
  view. It times every file and shows what the request does after the stop.
  Times count from the request's first call, without debugger stops (gaps of
  over 0.1 s outside PHP's own functions).

A file is predicted only when a trace or an earlier run reached it after this
point. `enter` opens the file in CODE where the stack is in it, else at its
first function; `b` toggles a breakpoint on that function's first line.

In CODE the list is on the left, above OUTLINE: the functions of the file in
SOURCE, found by reading it (PHP, JavaScript and TypeScript, Rust, C, Go),
whether each ran, where the one at the cursor was called from and what it calls.
SOURCE marks with `│` the lines a stop or the trace reached.

Xdebug writes traces in `trace` mode, in the computerized format tuitor reads:

```ini
xdebug.mode = debug,trace
xdebug.start_with_request = trigger    ; XDEBUG_TRIGGER (cookie, query, environment) starts both
xdebug.trace_format = 1
xdebug.output_dir = /tmp/xdebug
xdebug.trace_output_name = trace.%u.%R ; a file per request: the time and the URI
```

`/config set xdebugTraceDir /tmp/xdebug` tells tuitor where they are (a field of
tuitor's own; the adapter ignores it). A trace is complete once its request has
finished: trace a request, then debug the next one that takes the same path. In
a container, mount the directory and map it like the code:

```yaml
services:
  php:
    volumes: [".:/var/www/app", "./var/xdebug:/tmp/xdebug"]
    environment:
      XDEBUG_MODE: debug,trace
      XDEBUG_CONFIG: client_host=host.docker.internal client_port=9003 output_dir=/tmp/xdebug
```

`XDEBUG_CONFIG` does not take `trace_format` or `trace_output_name`: set them in
an ini file of the image (`/usr/local/etc/php/conf.d/` in the official ones).
With `/config map /tmp/xdebug=./var/xdebug`, tuitor finds the directory and the
files the traces name through `pathMappings`. `TRACE=1 ./run.sh` in
[`examples/php`](examples/php) traces its run.

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
| `g o` `g c` `g h` `g f` `g p` | views (`g b`: PROJECT at BREAKPOINTS) | | `z` | zoom pane |
| `j` `k` `h` `l`, arrows | move, collapse, expand | | `q` / `esc` | back out (zoom → history → browsing) |
| `<` `>` | previous / next tab (`g T` `g t`) | | `O` / `T` | open a project / in a new tab |
| `g s` `g S` | next / previous session of the tab | | `ctrl-w` | close the tab |
| `E` | open the SOURCE line or the selected breakpoint in your editor | | `!` | doctor: what debugging needs here |
| `^f` | search the file in SOURCE as you type | | `^n` `^p` | in SOURCE: next / previous match |

In BREAKPOINTS: `a` session/all, `e` condition, `l` logpoint, `space` on/off, `d` delete.
In CONFIG: `enter` start or edit, `a` add, `m` map a path, `M` map a compose service's mounts,
`u` unset a field, `d` delete, `i` install the selected configuration's adapter.
In SOURCE: `^n` `^p` next / previous match of a search, `q` clears its highlights.
In STACK: `v` library frames. In OUTPUT: `enter` opens the file:line of the bottom line, `C` clear.
In EVENTS: `enter` opens the file:line of the selected line.
In FLOW: `enter` open in CODE, `b` breakpoint where the file is entered, `v` library files.
In OUTLINE: `enter` go to the function, `b` breakpoint on its first line.

A search (`^f`) goes to the first match as it is typed; it ignores case unless it has a
capital letter, and `enter` on an empty search repeats the last one.

`enter` on a line of OUTPUT or EVENTS that names a file and a line opens it in CODE: PHP's
`in /app/src/Order.php:12`, `Order.php(12)` and `on line 12`, Node's `at f (/w/a.js:3:9)`,
Rust's `panicked at src/main.rs:12:5`, Go's `/w/main.go:58`. EVENTS has such a line, `at`,
for every stop. A path of a program that runs elsewhere goes through its session's
`pathMappings` the other way; a relative one is taken from the project, else from the
configuration's `cwd`. In OUTPUT `k` and `j` scroll, and the bottom line is the one taken.

### Changing keys

Copy `crates/tuitor/src/default_keys.toml` to `~/.config/tuitor/keys.toml`. A
binding is a key and a command line; `""` unbinds:

```toml
[global]
m = "continue"
c = ""
"g n" = "find"

[pane.locals]
enter = "open"
```

### Your editor

`E` (`/edit`) opens the line under the SOURCE cursor, or in BREAKPOINTS the
selected breakpoint, in your editor:

- In a Neovim terminal (`$NVIM` is set), in that Neovim, and tuitor stays on
  screen: in a window that shows the file already, else in the one used before
  tuitor's, else in a split beside tuitor's.
- Otherwise in `$VISUAL` or `$EDITOR`, with `+line file`, in tuitor's terminal,
  which tuitor gives it until it exits.

Any other editor is a command in `~/.config/tuitor/config.toml`, split at
spaces, with `{file}` and `{line}` replaced in each word:

```toml
editor = "code --goto {file}:{line}"
editor_terminal = false     # it has a window of its own: tuitor stays on screen
```

`editor_terminal` is `true` by default, for an editor that runs in the
terminal, such as `hx {file}:{line}`.

## Adapters

| type | runs | debugs |
|---|---|---|
| `php` | `node <adapters>/vscode-php-debug/extension/out/phpDebug.js` | PHP with Xdebug |
| `gdb` | `gdb --interpreter=dap` | C, C++, Rust, Zig |
| `lldb-dap` | `lldb-dap` | C, C++, Rust, Zig |
| `lldb` | `<adapters>/codelldb/extension/adapter/codelldb --port ${port}` (CodeLLDB) | C, C++, Rust, Zig |
| `pwa-node`, `node` | `node <adapters>/js-debug/js-debug/src/dapDebugServer.js ${port} 127.0.0.1` | JavaScript, TypeScript in Node.js |
| `pwa-chrome`, `chrome` | the same js-debug | JavaScript, TypeScript in Chrome or Chromium |
| `go` | `dlv dap --listen 127.0.0.1:${port}` (Delve) | Go |

js-debug runs each Node process and each browser page in a session of its own,
which it asks tuitor for (`startDebugging`); the program stops there, and
breakpoints go there. tuitor shows it as part of the session that launched
it: the context line names the configuration and the target
(`server › server.js [4242]`), and the tab's state, `g s` and the count of
live sessions take the two as one. `X` stops the whole program. `node` and
`chrome` configurations go to js-debug as `pwa-node` and `pwa-chrome`, as VS
Code sends them. `"console": "integratedTerminal"` asks for a terminal that
tuitor cannot give the program, so tuitor ends the session and says so; with
`"internalConsole"`, the default, the program's output shows in OUTPUT.

Delve builds the program itself, without optimisations, and runs it. A `go`
configuration, as in VS Code:

```jsonc
{
  "name": "Launch Package",
  "type": "go",
  "request": "launch",
  "mode": "debug",                    // build and run the package; "test" runs its tests
  "program": "${workspaceFolder}"     // the package's directory
}
```

dlv itself does not know `"mode": "auto"`, which VS Code's snippets write: tuitor
makes it `test` for a `program` that is a `_test.go` file and `debug` for
anything else, as VS Code does when the editor shows no test file.
`${fileDirname}` (the file open in the editor) means nothing to tuitor; name the
directory. dlv writes the program's output to its own standard output, which
tuitor shows as the adapter's, so tuitor asks dlv to send it as the program's
(`"outputMode": "remote"`) unless the configuration sets `outputMode`,
`stdoutTo` or `stderrTo`. `"stopOnEntry": true` stops before Go has started
the program, where dlv has no stack and no variables to show; a breakpoint on
`func main() {` stops at the start of `main`.

Programs are looked up on `PATH` only. Adapters released as VS Code extensions
are unpacked from their official `.vsix` (js-debug: its DAP server's
`.tar.gz`) into `<adapters>`:
`${XDG_DATA_HOME:-~/.local/share}/tuitor/adapters` (macOS:
`~/Library/Application Support/tuitor/adapters`). `tuitor doctor` shows what
is installed and what is missing.

### Doctor

`tuitor doctor`, and `/doctor` (`!`) in tuitor, checks what debugging needs on
this machine. For each adapter: the program it runs, the script of one that
runs on Node.js and that Node.js (vscode-php-debug needs 20.18 or newer), that
gdb answers as a DAP server, that Delve is on `PATH`, a browser for
`pwa-chrome`, and Xdebug in the `php` on `PATH`. For each configuration of the
project, and in tuitor of every open project: that its adapter is ready, that
the port to listen for Xdebug on is free, that the PHP vscode-php-debug starts
for a `program` loads Xdebug 3 (for PHP that something else starts, php-fpm or
a container, it notes what the `php` here lacks, and one that maps paths runs
elsewhere, where tuitor cannot look), the browser it starts and the program it
debugs. Whatever is missing comes with
the command that fixes it; in DOCTOR, `enter` puts a tuitor command on the
command line and copies a shell command, which tuitor never runs itself.

```sh
tuitor doctor                  # every adapter, and this directory's project
tuitor doctor php gdb          # these adapters, and this directory's project
tuitor doctor --php /opt/php/bin/php -C ~/shop/api
```

It exits with 1 when something the project, or an adapter named, needs is
missing, so that scripts can check before they start tuitor.

Override or add in `~/.config/tuitor/adapters.toml`:

```toml
[php]
command = "node"
args = ["/opt/vscode-php-debug/extension/out/phpDebug.js"]
extensions = ["php"]        # breakpoints in these files go to this adapter
```

`transport` is `stdio` (default), `server` (adapter listens on `${port}`) or
`tcp` (connect to `address`).

An adapter tuitor starts gets a temporary directory of its own as `TMPDIR`,
`tuitor-<pid>-<n>` in `$XDG_RUNTIME_DIR` or else the system's temporary
directory, and the programs it starts inherit it. tuitor removes it once the
adapter has exited, when its session ends or tuitor quits; js-debug would
otherwise leave a directory there at every start, with the profile of the
browser it started. `env` in `adapters.toml` can set another `TMPDIR`.
SIGTERM, SIGHUP (its terminal closed) and SIGINT end tuitor as quitting does:
it disconnects the adapters, removes their directories and puts the terminal
back, then exits with 128 and the signal's number. Adapters run in a process
group of their own, so that ctrl-c at the terminal and the terminal closing
signal tuitor alone, which then ends them in that order.

### Installing adapters

Each from where its authors publish it. `tuitor install <adapter>` (`/install`,
or `i` on a configuration in CONFIG) does what this section describes: it says
what it downloads from where into which directory, asks, downloads the release
with curl, unpacks it beside the installation and puts it in its place only
once it is complete, then checks that the adapter finds what it runs; closed
while it runs, INSTALL lets it go on and the status line says how it went. The
adapter is a type (`php`, `pwa-node`, `pwa-chrome`, `lldb`, `go`) or a name
(`vscode-php-debug`, `js-debug`, `codelldb`, `delve`); `--version` installs
another release than the one below. Delve it installs with the `go install`
below, after asking. gdb, LLDB and Xdebug come with your distribution: tuitor
names the package and does not install it, as that takes root.

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
v=1.12.3    # or a newer release
dir="${XDG_DATA_HOME:-$HOME/.local/share}/tuitor/adapters/codelldb"
rm -rf "$dir" && mkdir -p "$dir"
curl -fL -o "$dir/codelldb.vsix" "https://github.com/vadimcn/codelldb/releases/download/v$v/codelldb-linux-x64.vsix"
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

Browser configurations also need Chrome or Chromium. Without
`runtimeExecutable` js-debug looks for Google Chrome only; a configuration
tuitor makes names the first of `google-chrome`, `chromium`,
`chromium-browser` and `microsoft-edge` it finds on `PATH`, and CONFIG points
out one that names none. In launch.json, or for a browser off `PATH`, set
`runtimeExecutable` to its path (`/usr/bin/chromium`).

On macOS, use `~/Library/Application Support/tuitor/adapters/…` as `dir`.

**Delve** (`dlv`), the Go debugger, is built with Go. Go comes from
[go.dev/dl](https://go.dev/dl/) or your distribution (`pacman -S go`,
`apt-get install golang-go`, `dnf install golang`); Delve supports only recent
Go releases (1.27.2: Go 1.25 to 1.27), so an older distribution's Go may be too
old. Then, as [Delve's installation guide](https://github.com/go-delve/delve/tree/master/Documentation/installation)
says:

```sh
go install github.com/go-delve/delve/cmd/dlv@latest
```

`go install` puts `dlv` in `$(go env GOPATH)/bin` (`~/go/bin`, unless `GOPATH`
or `GOBIN` say otherwise), which must be on `PATH`, e.g. in `~/.bashrc` or
`~/.zshrc`:

```sh
export PATH="$PATH:$(go env GOPATH)/bin"
```

tuitor names that directory when `dlv` is there but not on `PATH`.

Breakpoints and watches are remembered per project in `~/.local/state/tuitor/`,
outside the repository, and so are the projects opened recently and the files of
recent runs, for FLOW. Logs: `~/.local/state/tuitor/tuitor.log`
(`TUITOR_LOG=dap=trace` records the protocol).

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
TUITOR_DLV="$(go env GOPATH)/bin/dlv" cargo test -p tuitor-core --test go -- --ignored
```

`tuitor --script file` runs commands headless and saves screens as ANSI
(`cat` them); see `crates/tuitor/src/script.rs`.

## Not there yet

- `runInTerminal`: programs that need a terminal of their own (`"console": "integratedTerminal"`)
