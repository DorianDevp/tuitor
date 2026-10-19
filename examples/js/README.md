# JavaScript example

`orders.js` totals three orders: a loop whose values change at every stop,
nested objects (`Order` → `Customer` → `Address`) and a call chain
`orderTotal()` → `lineTotal()`. Node.js runs it, debugged through js-debug.

Needs Node.js and js-debug; see
[Installing adapters](../../README.md#installing-adapters).

```sh
./run.sh    # checks those (and offers to install js-debug), builds tuitor, starts tuitor
```

tuitor stops on the first line (`stopOnEntry` in `.vscode/launch.json`);
quitting it (`Q`) ends the script. `TUITOR=/path/to/tuitor ./run.sh` uses that
binary instead of building tuitor from this repository, and arguments go to
tuitor (`./run.sh --script demo.tsc`).

By hand: `tuitor` in this directory.

js-debug starts Node and then asks tuitor for a second session, in which the
program runs: stops happen there, and breakpoints go there. tuitor shows the
two as one session, `orders › orders.js [pid]` on the context line: the
configuration and the program.

Put a breakpoint on line 50, `total += lineTotal(line);`: press `f`, type
`orders.js:50`, press `ctrl-b`, then `esc` to close the finder. Then try:

- `c` runs to the breakpoint, `c` again runs one more iteration: `line` and
  `total` turn green with `▲` and their previous value.
- `tab` moves to LOCALS; `j` to `order`, `l` expands it: `customer` →
  `address`.
- `s` steps into `lineTotal()` (STACK shows it above `orderTotal` and the
  script), `o` steps out.
- `w` on `total` in LOCALS watches it; `g h` shows it at every stop, `[` and
  `]` go back and forth through the stops, `g o` returns.
- `R` runs the program again (the breakpoint is remembered), `Q` quits.

What the program prints shows in OUTPUT: `"console": "internalConsole"` in
`.vscode/launch.json` has js-debug send it there. `"integratedTerminal"`, which
VS Code offers too, would ask tuitor for a terminal to run the program in;
tuitor has none to give, so it would end the session and say so.
