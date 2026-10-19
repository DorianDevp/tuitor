# Rust example

`src/main.rs` totals three orders: a loop whose values change at every stop,
nested structs (order → customer → address) and a call chain
`main` → `order_total` → `line_total`.

Needs Rust and gdb 14 or newer built with Python; see
[Installing adapters](../../README.md#installing-adapters).

```sh
./run.sh    # checks those, builds orders and tuitor, starts tuitor
```

tuitor stops at the start of main; quitting it (`Q`) ends the script.
`TUITOR=/path/to/tuitor ./run.sh` uses that binary instead of building tuitor
from this repository, and arguments go to tuitor (`./run.sh --script demo.tsc`).

By hand:

```sh
cargo build
tuitor      # starts .vscode/launch.json and stops at the start of main
```

Put a breakpoint on line 56, `total += line_total(line);`: press `f`, type
`main.rs:56`, press `ctrl-b`, then `esc` to close the finder. Then try:

- `c` runs to the breakpoint, `c` again runs one more iteration: `line` and
  `total` turn green with `▲` and their previous value.
- `tab` moves to LOCALS; `j` to `order`, `l` expands it: `value` →
  `customer` → `address`.
- `s` steps into `line_total` (STACK shows three frames), `o` steps out.
- `w` on `total` in LOCALS watches it; `g h` shows it at every stop, `[` and
  `]` go back and forth through the stops, `g o` returns.
- `R` runs the program again (the breakpoint is remembered), `Q` quits.

gdb shows `&str`, slices and enums readably, which is why the example uses
them. `String` and `Vec` need Rust's pretty printers, which `rust-gdb` (part
of every rustup toolchain) loads. To use it, add to
`~/.config/tuitor/adapters.toml`:

```toml
[gdb]
command = "rust-gdb"
args = ["--interpreter=dap", "--quiet"]
extensions = ["rs"]
```
