# Go example

`main.go` totals three orders: a loop whose values change at every stop,
nested structs (`Order` → `Customer` → `Address`), a slice of `Line`s and a
call chain `main` → `orderTotal` → `lineTotal`. Delve (`dlv`) builds and runs
it.

Needs Go and Delve; see
[Installing adapters](../../README.md#installing-adapters).

```sh
./run.sh    # checks those (and offers to install Delve), builds tuitor, starts tuitor
```

Quitting tuitor (`Q`) ends the script. `TUITOR=/path/to/tuitor ./run.sh` uses
that binary instead of building tuitor from this repository, and arguments go
to tuitor (`./run.sh --script demo.tsc`).

By hand: `tuitor` in this directory, with `dlv` on `PATH`.

dlv builds the package without optimisations, so that every variable is
visible, into a `__debug_bin…` next to `main.go`, and removes it when the
session ends.

The first run goes through to the end; OUTPUT shows what the program printed.
Delve's `stopOnEntry` stops before Go has started the program, where it has no
stack and no variables to show (tuitor says `reading the stop failed: Unable to
produce stack trace: unknown goroutine 1`), so `.vscode/launch.json` does not
ask for it. Put a breakpoint on line 58, `total += lineTotal(line)`: press `f`,
type `main.go:58`, press `ctrl-b`, then `esc` to close the finder. `R` runs the
program again, and it stops there. Then try:

- `c` runs one more iteration: `line` and `total` turn green with `▲` and
  their previous value.
- `tab` moves to LOCALS; `j` to `order`, `l` expands it: `Customer` →
  `Address`, and `Lines`. `~r0` is the function's result, as dlv names it.
- `s` steps into `lineTotal` (STACK shows it above `main.orderTotal` and
  `main.main`), `o` steps out.
- `w` on `total` in LOCALS watches it; `g h` shows it at every stop, `[` and
  `]` go back and forth through the stops, `g o` returns.
- `Q` quits. The breakpoint is remembered, so the next run stops there.
