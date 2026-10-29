# Quick start

## Try an example

Each directory in [`examples/`](https://github.com/DorianDevp/tuitor/tree/master/examples) has a
`run.sh` that checks what it needs, offers to install missing adapters, builds tuitor and starts it.

```sh
git clone https://github.com/DorianDevp/tuitor && cd tuitor/examples/go
./run.sh
```

| example | debugged through |
|---|---|
| `php`, `php-paths` | Xdebug and vscode-php-debug (host or Docker) |
| `js`, `typescript` | js-debug in Node.js |
| `web`, `angular` | js-debug in Chromium (`HEADLESS=1 ./run.sh` over SSH) |
| `go` | Delve |
| `c`, `rust` | gdb |

## Your first session

1. `f`, type `main.go:58`, `ctrl-b`, `esc` — a breakpoint.
2. `c` — run to it. `c` again: changed values turn green with `▲` and the old value.
3. `tab` to LOCALS, `l` to expand, `w` to watch a variable.
4. `s` step in, `o` step out, `n` step over.
5. `[` and `]` step back and forth through recorded stops, `}` back to live.
6. `g h` — every variable at every stop.
7. `?` lists every command. `Q` quits.

## Your own code

```sh
tuitor                                    # this directory's project
tuitor "Listen for Xdebug"                # a configuration by name
tuitor -C ~/shop/api --open ~/shop/web    # two projects, two tabs
tuitor exec ./target/debug/app -- --flag  # a native program, through gdb
tuitor listen --map /var/www/app=./api    # PHP on port 9003, no configuration needed
tuitor build api/Dockerfile               # a Dockerfile build
tuitor configs                            # what tuitor sees
```

tuitor reads `.vscode/launch.json`. A project without one gets a default for its kind
(PHP, Rust, Go, C, JS/TS); see [Configuration](configuration.md).
