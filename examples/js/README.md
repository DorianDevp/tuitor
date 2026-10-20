# JavaScript example

`orders.js` totals three orders in Node.js, debugged through js-debug: a loop,
nested objects (`Order` → `Customer` → `Address`) and a call chain
`orderTotal()` → `lineTotal()`.

Needs Node.js and js-debug; see
[Installing adapters](../../README.md#installing-adapters).

```sh
./run.sh
```

`run.sh` checks those, builds tuitor, has it offer to install js-debug
(`tuitor install`) when it is missing, and starts it on the configuration `orders` in `.vscode/launch.json`, which stops
on the first line. `TUITOR=/path/to/tuitor ./run.sh` uses that binary instead
of building tuitor from this repository, and arguments go to tuitor
(`./run.sh --script demo.tsc`).

js-debug starts Node, then asks tuitor for a second session to run the program
in, where it stops and where breakpoints go. tuitor shows the two as one
session: `orders › orders.js [pid]` in the header, the configuration and the
program. What the program prints shows in OUTPUT (`"console":
"internalConsole"`); `"integratedTerminal"` would ask for a terminal tuitor
cannot give, and tuitor would end the session and say so.

## A first session

Put a breakpoint on line 50, `total += lineTotal(line);`: `f`, type
`orders.js:50`, `ctrl-b`, then `esc` to close the finder. tuitor remembers it
for later runs. Then:

- `c` runs to the breakpoint, `c` again runs one more iteration: `line` and
  `total` turn green with `▲` and their previous value.
- `tab` moves to LOCALS; `j` down to `order`, `l` expands it: `customer` →
  `address`. `j` down to `total`, `w` watches it.
- `s` steps into `lineTotal()` (STACK shows it above `orderTotal` and the
  script, `<anonymous>`), `o` steps out.
- `tab` moves on to STACK; `j` selects `<anonymous>`, and LOCALS shows the
  script's variables.
- `g h` shows every variable at every stop: `[` and `]` go back and forth
  through the stops, `}` back to the live one, `g o` back to OVERVIEW.
- `g p` shows the breakpoints and CONFIG, with `orders` from
  `.vscode/launch.json`. `E` opens the line under the SOURCE cursor in your
  editor (`$VISUAL` or `$EDITOR`).
- `O` opens another project in a new tab, such as `../typescript`; `<` and
  `>` switch tabs.
- `R` runs the program again, `Q` quits.

## By hand

```sh
tuitor      # starts the only configuration in .vscode/launch.json
```
