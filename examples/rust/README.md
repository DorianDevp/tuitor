# Rust example

`src/main.rs` totals three orders, debugged through gdb: a loop, nested
structs (order → customer → address) and a call chain `main` → `order_total`
→ `line_total`.

Needs Rust and gdb 14 or newer built with Python; see
[Installing adapters](../../README.md#installing-adapters).

```sh
./run.sh
```

`run.sh` checks those, builds the example (`cargo build`, into `target/` here)
and tuitor, and starts tuitor on the configuration `orders` in
`.vscode/launch.json`, which stops at the start of `main`.
`TUITOR=/path/to/tuitor ./run.sh` uses that binary instead of building tuitor
from this repository, and arguments go to tuitor (`./run.sh --script demo.tsc`).

## A first session

Put a breakpoint on line 56, `total += line_total(line);`: `f`, type
`main.rs:56`, `ctrl-b`, then `esc` to close the finder. tuitor remembers it
for later runs. Then:

- `c` runs to the breakpoint, `c` again runs one more iteration: `line` and
  `total` turn green with `▲` and their previous value.
- `tab` moves to LOCALS; `j` to `order`, `l` expands it: `value` →
  `customer` → `address`. `j` down to `total`, `w` watches it.
- `s` steps into `line_total` (STACK shows three frames), `o` steps out.
- `tab` moves on to STACK; `j` selects `orders::main`, and LOCALS shows its
  variables.
- `g h` shows every variable at every stop: `[` and `]` go back and forth
  through the stops, `}` back to the live one, `g o` back to OVERVIEW.
- `g p` shows the breakpoints and CONFIG, with `orders` from
  `.vscode/launch.json`. `E` opens the line under the SOURCE cursor in your
  editor (`$VISUAL` or `$EDITOR`).
- `O` opens another project in a new tab, such as `../c`; `<` and `>` switch
  tabs.
- `R` runs the program again, `Q` quits.

## By hand

```sh
cargo build
tuitor      # starts the only configuration in .vscode/launch.json
```

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
