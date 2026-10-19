# C example

`orders.c` totals three orders: a loop whose values change at every stop,
nested structs (order → customer → address) and a call chain
`main` → `order_total` → `line_total`.

Needs a C compiler and gdb 14 or newer built with Python; see
[Installing adapters](../../README.md#installing-adapters).

```sh
./run.sh    # checks those, builds orders and tuitor, starts tuitor
```

tuitor stops at the start of main; quitting it (`Q`) ends the script.
`TUITOR=/path/to/tuitor ./run.sh` uses that binary instead of building tuitor
from this repository, and arguments go to tuitor (`./run.sh --script demo.tsc`).

By hand:

```sh
make        # -g -O0, so every variable is visible
tuitor      # starts .vscode/launch.json and stops at the start of main
```

Put a breakpoint on line 48, `total += line_total(...)`: press `f`, type
`orders.c:48`, press `ctrl-b`, then `esc` to close the finder. Then try:

- `c` runs to the breakpoint, `c` again runs one more iteration: `i` and
  `total` turn green with `▲` and their previous value.
- `tab` moves to LOCALS; `j` to `order`, `l` expands it: `value` →
  `customer` → `address`.
- `s` steps into `line_total` (STACK shows three frames), `o` steps out.
- `w` on `total` in LOCALS watches it; `g h` shows it at every stop, `[` and
  `]` go back and forth through the stops, `g o` returns.
- `R` runs the program again (the breakpoint is remembered), `Q` quits.
