# Go example

`main.go` totals three orders, built and debugged by Delve (`dlv`): a loop,
nested structs (`Order` → `Customer` → `Address`), a slice of `Line`s and a
call chain `main` → `orderTotal` → `lineTotal`.

Needs Go and Delve; see
[Installing adapters](../../README.md#installing-adapters).

```sh
./run.sh
```

`run.sh` checks those (and offers to install Delve with `go install`, putting
Go's bin directory on `PATH` for this run if it is not), builds tuitor, and
starts it on the configuration `orders` in `.vscode/launch.json`.
`TUITOR=/path/to/tuitor ./run.sh` uses that binary instead of building tuitor
from this repository, and arguments go to tuitor (`./run.sh --script demo.tsc`).

dlv builds the package without optimisations, so that every variable is
visible, into a `__debug_bin…` next to `main.go`, and removes it when the
session ends. Delve's `stopOnEntry` stops before Go has started the program,
where dlv has no stack and no variables to show, so `.vscode/launch.json` does
not ask for it: the first run goes through to the end, and OUTPUT shows what
the program printed.

## A first session

Put a breakpoint on line 58, `total += lineTotal(line)`: `f`, type
`main.go:58`, `ctrl-b`, then `esc` to close the finder. tuitor remembers it
for later runs. `R` runs the program again, and it stops there. Then:

- `c` runs one more iteration: `line` and `total` turn green with `▲` and
  their previous value.
- `tab` moves to LOCALS; `j` to `order`, `l` expands it: `Customer` →
  `Address`, and `Lines`. `~r0` is the function's result, as dlv names it.
  `j` down to `total`, `w` watches it.
- `s` steps into `lineTotal` (STACK shows it above `main.orderTotal` and
  `main.main`), `o` steps out.
- `tab` moves on to STACK; `j` selects `main.main`, and LOCALS shows its
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
tuitor      # with dlv on PATH; starts the only configuration in .vscode/launch.json
```
