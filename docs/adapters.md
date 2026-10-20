# Adapters

tuitor starts the adapter that a launch configuration's `type` names, talks the Debug
Adapter Protocol to it, and ends it with the session. Programs are looked up on `PATH`
only. Adapters released as VS Code extensions are unpacked into `<adapters>`:
`${XDG_DATA_HOME:-~/.local/share}/tuitor/adapters` (macOS:
`~/Library/Application Support/tuitor/adapters`).

| type | runs | debugs |
|---|---|---|
| `php` | `node <adapters>/vscode-php-debug/extension/out/phpDebug.js` | PHP with Xdebug |
| `gdb` | `gdb --interpreter=dap --quiet` | C, C++, Rust, Zig |
| `lldb-dap` | `lldb-dap` | C, C++, Rust, Zig |
| `lldb` | `<adapters>/codelldb/extension/adapter/codelldb --port ${port}` (CodeLLDB) | C, C++, Rust, Zig |
| `pwa-node`, `node` | `node <adapters>/js-debug/js-debug/src/dapDebugServer.js ${port} 127.0.0.1` | JavaScript, TypeScript in Node.js |
| `pwa-chrome`, `chrome` | the same js-debug | JavaScript, TypeScript in Chrome or Chromium |
| `go` | `dlv dap --listen 127.0.0.1:${port}` (Delve) | Go |

`tuitor configs` prints this list as it resolves on your machine, and
`~/.config/tuitor/adapters.toml` overrides or adds entries (see the README).

## js-debug

js-debug runs each Node process and each browser page in a session of its own, which it
asks tuitor for (`startDebugging`); the program stops there, and breakpoints go there.
tuitor shows it as part of the session that launched it: the context line names the
configuration and the target (`orders › orders.js [4242]`), and the tab's state, `g s` and
the count of live sessions take the two as one. `X` stops the whole program.

`node` and `chrome` configurations go to js-debug as `pwa-node` and `pwa-chrome`, as VS
Code sends them. `"console": "integratedTerminal"` asks for a terminal that tuitor cannot
give the program, so tuitor ends the session and says so; with `"internalConsole"`, the
default, the program's output shows in OUTPUT.

Browser configurations need Chrome or Chromium. Without `runtimeExecutable`, js-debug
looks for Google Chrome only; a configuration tuitor makes names the first of
`google-chrome`, `chromium`, `chromium-browser` and `microsoft-edge` it finds on `PATH`,
and CONFIG points out one that names none.

tuitor does not run VS Code's `preLaunchTask`: start a dev server such as `ng serve`
yourself. For Angular's application builder no `sourceMapPathOverrides` are needed;
`"webRoot": "${workspaceFolder}"` puts `src/app/…` in the project
([`examples/angular`](../examples/angular)).

## Delve

Delve builds the program itself, without optimisations, and runs it:

```jsonc
{
  "name": "Launch Package",
  "type": "go",
  "request": "launch",
  "mode": "debug",                    // build and run the package; "test" runs its tests
  "program": "${workspaceFolder}"     // the package's directory
}
```

dlv itself does not know `"mode": "auto"`, which VS Code's snippets write: tuitor makes it
`test` for a `program` that is a `_test.go` file and `debug` for anything else.
`${fileDirname}` (the file open in VS Code) means nothing to tuitor; name the directory.
dlv writes the program's output to its own standard output, so tuitor asks it to send the
output as the program's (`"outputMode": "remote"`) unless the configuration sets
`outputMode`, `stdoutTo` or `stderrTo`. `"stopOnEntry": true` stops before Go has started
the program, where dlv has no stack to show; a breakpoint on `func main() {` stops at the
start of `main`.

## Installing by hand

`tuitor install <adapter>` does what this section describes. Each adapter comes from where
its authors publish it; on macOS use `~/Library/Application Support/tuitor/adapters/…` as
`dir`.

**vscode-php-debug** (PHP), from its
[GitHub releases](https://github.com/xdebug/vscode-php-debug/releases). It runs on
Node.js 20.18 or newer, and PHP needs Xdebug 3 loaded (`php -v` mentions it): your
distribution's package (`apt-get install php-xdebug`, `dnf install php-xdebug`,
`pacman -S xdebug`) or `pie install xdebug/xdebug`, see
[xdebug.org/docs/install](https://xdebug.org/docs/install). Arch's package installs
`/etc/php/conf.d/xdebug.ini` with the extension commented out; uncomment
`zend_extension=xdebug.so` there.

```sh
v=1.40.2
dir="${XDG_DATA_HOME:-$HOME/.local/share}/tuitor/adapters/vscode-php-debug"
rm -rf "$dir" && mkdir -p "$dir"
curl -fL -o "$dir/php-debug.vsix" "https://github.com/xdebug/vscode-php-debug/releases/download/v$v/php-debug-$v.vsix"
unzip -q "$dir/php-debug.vsix" -d "$dir" && rm "$dir/php-debug.vsix"
```

**js-debug** (JavaScript, TypeScript), Microsoft's debugger from VS Code, on Node.js. Take
its standalone DAP server, `js-debug-dap-v<version>.tar.gz`, from its
[GitHub releases](https://github.com/microsoft/vscode-js-debug/releases):

```sh
v=1.140.0
dir="${XDG_DATA_HOME:-$HOME/.local/share}/tuitor/adapters/js-debug"
rm -rf "$dir" && mkdir -p "$dir"
curl -fL -o "$dir/js-debug-dap.tar.gz" "https://github.com/microsoft/vscode-js-debug/releases/download/v$v/js-debug-dap-v$v.tar.gz"
tar -xzf "$dir/js-debug-dap.tar.gz" -C "$dir" && rm "$dir/js-debug-dap.tar.gz"
```

**CodeLLDB** brings its own LLDB. Take the `.vsix` for your platform
(`codelldb-linux-x64`, `-linux-arm64`, `-darwin-arm64`, …) from its
[GitHub releases](https://github.com/vadimcn/codelldb/releases):

```sh
v=1.12.3
dir="${XDG_DATA_HOME:-$HOME/.local/share}/tuitor/adapters/codelldb"
rm -rf "$dir" && mkdir -p "$dir"
curl -fL -o "$dir/codelldb.vsix" "https://github.com/vadimcn/codelldb/releases/download/v$v/codelldb-linux-x64.vsix"
unzip -q "$dir/codelldb.vsix" -d "$dir" && rm "$dir/codelldb.vsix"
```

**Delve** (`dlv`) is built with Go, from [go.dev/dl](https://go.dev/dl/) or your
distribution (`pacman -S go`, `apt-get install golang-go`, `dnf install golang`); Delve
supports only recent Go releases. As
[its installation guide](https://github.com/go-delve/delve/tree/master/Documentation/installation)
says:

```sh
go install github.com/go-delve/delve/cmd/dlv@latest
```

`go install` puts `dlv` in `$(go env GOPATH)/bin` (`~/go/bin` unless `GOPATH` or `GOBIN`
say otherwise), which must be on `PATH`. tuitor names that directory when `dlv` is there
but not on `PATH`.

**gdb** 14 or newer, built with Python (its DAP server is written in Python): your
distribution's `gdb` (Debian 13, Ubuntu 24.04 and Arch qualify; Debian 12's 13.1 does
not) or the [GNU release](https://www.sourceware.org/gdb/download/).

**lldb-dap** comes with LLDB: `pacman -S lldb`, or the `LLVM-<version>-<platform>` archive
from [LLVM's releases](https://github.com/llvm/llvm-project/releases) (`bin/lldb-dap`).
Debian and Ubuntu install it as `lldb-dap-NN` with `lldb-NN`; link it as `lldb-dap` into a
directory on `PATH`.

## Processes

An adapter tuitor starts runs in a process group of its own, so that ctrl-c at the
terminal and the terminal closing reach tuitor alone, which then ends the adapter in
order. It gets a temporary directory of its own as `TMPDIR`, `tuitor-<pid>-<n>` in
`$XDG_RUNTIME_DIR` or else the system's temporary directory, which the programs it starts
inherit and tuitor removes once the adapter has exited; js-debug would otherwise leave the
profile of every browser it started there. `env` in `adapters.toml` can set another
`TMPDIR`. SIGTERM, SIGHUP and SIGINT end tuitor as `Q` does: it disconnects the adapters,
removes their directories and puts the terminal back, then exits with 128 and the
signal's number.
