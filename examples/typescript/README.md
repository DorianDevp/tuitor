# TypeScript example

`orders.ts` totals three orders: a loop whose values change at every stop,
nested objects (order → customer → address) and a call chain `orderTotal()` →
`lineTotal()`. `tsc` compiles it to `dist/orders.js` with a source map, Node.js
runs that, and js-debug follows the source map back: breakpoints go in
`orders.ts`, and stops, STACK and stepping show `orders.ts`.

Needs Node.js with npm and js-debug; see
[Installing adapters](../../README.md#installing-adapters). npm installs
TypeScript (the version in `package.json`) into `node_modules`.

```sh
./run.sh    # checks those (and offers to install js-debug), builds the example and tuitor, starts tuitor
```

tuitor stops on the first line (`stopOnEntry` in `.vscode/launch.json`);
quitting it (`Q`) ends the script. `TUITOR=/path/to/tuitor ./run.sh` uses that
binary instead of building tuitor from this repository, and arguments go to
tuitor (`./run.sh --script demo.tsc`).

By hand:

```sh
npm ci      # TypeScript, once
npx tsc     # dist/orders.js and dist/orders.js.map, after every change
tuitor
```

tuitor does not run VS Code's `preLaunchTask`, so build before starting it.

Put a breakpoint on line 42, `total += lineTotal(line);`: press `f`, type
`orders.ts:42`, press `ctrl-b`, then `esc` to close the finder. Then try:

- `c` runs to the breakpoint, `c` again runs one more iteration: `line` and
  `total` turn green with `▲` and their previous value.
- `tab` moves to LOCALS; `j` to `order`, `l` expands it: `customer` →
  `address`.
- `s` steps into `lineTotal()` (STACK shows it above `orderTotal` and the
  script, all in `orders.ts`), `o` steps out.
- `w` on `total` in LOCALS watches it; `g h` shows it at every stop, `[` and
  `]` go back and forth through the stops, `g o` returns.
- `R` runs the program again (the breakpoint is remembered), `Q` quits.

What the program prints shows in OUTPUT.
