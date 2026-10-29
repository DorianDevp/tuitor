# Debug C, C++, Rust and Zig in the terminal

Three adapters work; pick one:

| adapter | type in launch.json | install |
|---|---|---|
| gdb 14+ built with Python | `gdb` | distribution package (Debian 13, Ubuntu 24.04, Arch qualify; Debian 12 does not) |
| lldb-dap | `lldb-dap` | LLVM; on Debian/Ubuntu link `lldb-dap-NN` as `lldb-dap` |
| CodeLLDB | `lldb` | `tuitor install codelldb` |

Debug a binary without any configuration:

```sh
tuitor exec ./target/debug/app -- --flag          # gdb
tuitor exec --adapter lldb-dap ./app              # another adapter
```

Or start in the project: a `Cargo.toml` gets a gdb launch of `target/debug/<package>`; a
`Makefile`, `CMakeLists.txt` or `*.c` gets a gdb launch of an executable built there.

- Build with debug info (`cargo build`, `-g`).
- gdb's hit counts: the number of hits to pass over.

## Try it

```sh
cd examples/c && ./run.sh
cd examples/rust && ./run.sh
```

Then `f`, `orders.c:48` (or `main.rs:56`), `ctrl-b`, `esc`, `c`.
