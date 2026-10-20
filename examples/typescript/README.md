# TypeScript example

`orders.ts` totals three orders, compiled by `tsc` with a source map, run by
Node.js and debugged in `orders.ts` through js-debug: a loop, nested objects
(order → customer → address) and a call chain `orderTotal()` → `lineTotal()`.

Needs Node.js with npm and js-debug; see
[Installing adapters](../../README.md#installing-adapters). npm installs
TypeScript (the version in `package.json`) into `node_modules`.

```sh
./run.sh
```

`run.sh` checks those (and offers to install js-debug), installs TypeScript
the first time (`npm ci`), compiles `dist/orders.js` and `dist/orders.js.map`
(`tsc`), builds tuitor, and starts it on the configuration `orders` in
`.vscode/launch.json`, which stops on the first line.
`TUITOR=/path/to/tuitor ./run.sh` uses that binary instead of building tuitor
from this repository, and arguments go to tuitor (`./run.sh --script demo.tsc`).

js-debug follows the source map: breakpoints go in `orders.ts`, and stops,
STACK and stepping show `orders.ts`. As in the [JavaScript example](../js),
the program runs in a second session that tuitor shows as one with the first,
and what it prints shows in OUTPUT (`"console": "internalConsole"`;
`"integratedTerminal"` would ask for a terminal tuitor cannot give, and tuitor
would end the session and say so).

## A first session

Put a breakpoint on line 42, `total += lineTotal(line);`: `f`, type
`orders.ts:42`, `ctrl-b`, then `esc` to close the finder. tuitor remembers it
for later runs. Then:

- `c` runs to the breakpoint, `c` again runs one more iteration: `line` and
  `total` turn green with `▲` and their previous value.
- `tab` moves to LOCALS; `j` down to `order`, `l` expands it: `customer` →
  `address`. `j` down to `total`, `w` watches it.
- `s` steps into `lineTotal()` (STACK shows it above `orderTotal` and the
  script, `<anonymous>`, all in `orders.ts`), `o` steps out.
- `tab` moves on to STACK; `j` selects `<anonymous>`, and LOCALS shows the
  script's variables.
- `g h` shows every variable at every stop: `[` and `]` go back and forth
  through the stops, `}` back to the live one, `g o` back to OVERVIEW.
- `g p` shows the breakpoints and CONFIG, with `orders` from
  `.vscode/launch.json`. `E` opens the line under the SOURCE cursor, in
  `orders.ts`, in your editor (`$VISUAL` or `$EDITOR`).
- `O` opens another project in a new tab, such as `../js`; `<` and `>` switch
  tabs.
- `R` runs the program again, `Q` quits.

## By hand

tuitor does not run VS Code's `preLaunchTask`, so build before starting it:

```sh
npm ci      # TypeScript, once
npx tsc     # dist/orders.js and dist/orders.js.map, after every change
tuitor      # starts the only configuration in .vscode/launch.json
```
