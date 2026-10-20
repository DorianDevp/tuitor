# C example

`orders.c` totals three orders, debugged through gdb: a loop, nested structs
(order → customer → address) and a call chain `main` → `order_total` →
`line_total`.

Needs a C compiler, make and gdb 14 or newer built with Python; see
[Installing adapters](../../README.md#installing-adapters).

```sh
./run.sh
```

`run.sh` checks those, builds `orders` (`make`) and tuitor, and starts tuitor
on the configuration `orders` in `.vscode/launch.json`, which stops at the
start of `main`. `TUITOR=/path/to/tuitor ./run.sh` uses that binary instead of
building tuitor from this repository, and arguments go to tuitor
(`./run.sh --script demo.tsc`).

## A first session

Put a breakpoint on line 48, `total += line_total(...)`: `f`, type
`orders.c:48`, `ctrl-b`, then `esc` to close the finder. tuitor remembers it
for later runs. Then:

- `c` runs to the breakpoint, `c` again runs one more iteration: `i` and
  `total` turn green with `▲` and their previous value.
- `tab` moves to LOCALS; `j` to `order`, `l` expands it: `value` →
  `customer` → `address`. `j` down to `total`, `w` watches it.
- `s` steps into `line_total` (STACK shows three frames), `o` steps out.
- `tab` moves on to STACK; `j` selects `main`, and LOCALS shows its variables.
- `g h` shows every variable at every stop: `[` and `]` go back and forth
  through the stops, `}` back to the live one, `g o` back to OVERVIEW.
- `g p` shows the breakpoints and CONFIG, with `orders` from
  `.vscode/launch.json`. `E` opens the line under the SOURCE cursor in your
  editor (`$VISUAL` or `$EDITOR`).
- `O` opens another project in a new tab, such as `../rust`; `<` and `>`
  switch tabs.
- `R` runs the program again, `Q` quits.

## By hand

```sh
make        # -g -O0, so every variable is visible
tuitor      # starts the only configuration in .vscode/launch.json
```
