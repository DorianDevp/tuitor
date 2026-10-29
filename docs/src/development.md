# Development

## Architecture

```
crates/tuitor-dap    DAP framing, message types, async client. Knows nothing about tuitor.
crates/tuitor-core   Engine: sessions, breakpoints, snapshots, deltas, flows, traces, doctor,
                     install. No terminal.
crates/tuitor-code   Language servers: call paths to a line. The only crate that speaks LSP.
crates/tuitor        The TUI: tabs, commands, keymap, views.
```

- Each tab owns an engine with all of its project's debugger state.
- One loop drives every engine; DAP responses arrive as messages tagged with their tab, so no locks.
- Keys map to command lines, command lines to actions, and every action is applied in
  `App::apply`. A key, the command line and a script do the same thing.

## Build and test

```sh
cargo build
cargo test --workspace        # includes a real gdb session
```

Tests against the other real adapters and language servers are opt-in through environment
variables; see [CONTRIBUTING.md](https://github.com/DorianDevp/tuitor/blob/master/CONTRIBUTING.md).

## Scripts and screenshots

`tuitor --script file` runs command lines and `@` directives without a terminal and saves screens as
ANSI. Directives are documented at the top of `crates/tuitor/src/script.rs`. Every example's `run.sh`
passes its arguments on (`./run.sh --script demo.tsc`). The screenshots are made this way by
`docs/screenshots/render.sh`; the demo videos are recorded with [VHS](https://github.com/charmbracelet/vhs)
from the tapes in `docs/tapes/`.
