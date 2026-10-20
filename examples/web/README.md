# Web example

`index.html` loads `orders.js`, which totals three orders when the page loads
and on **Total again**, debugged through js-debug in a Chromium of its own: a
loop, nested objects (`Order` → `Customer` → `Address`) and a call chain
`render()` → `orderTotal()` → `lineTotal()`.

Needs Chromium, Node.js and js-debug; see
[Installing adapters](../../README.md#installing-adapters).
`.vscode/launch.json` expects Chromium at `/usr/bin/chromium`, where Arch and
Debian put it; elsewhere, set `runtimeExecutable` there to your Chromium or
Chrome.

```sh
./run.sh               # Chromium with a window
HEADLESS=1 ./run.sh    # without one, e.g. over ssh
```

`run.sh` checks those, builds tuitor, has it offer to install js-debug
(`tuitor install`) when it is missing, and starts it on the configuration `orders` in `.vscode/launch.json` (`orders
(headless)` with `HEADLESS=1`). js-debug opens the page straight from disk
(`file://`), so no web server is needed, in a Chromium with a fresh profile
that quitting tuitor closes. `TUITOR=/path/to/tuitor ./run.sh` uses that
binary instead of building tuitor from this repository, and arguments go to
tuitor (`./run.sh --script demo.tsc`).

js-debug runs the page in a session of its own, apart from the browser's,
where it stops and where breakpoints go. tuitor shows the two as one session:
`orders › index.html` in the header, the configuration and the page. What the
page logs (`console.log`) shows in OUTPUT.

## A first session

Put a breakpoint on line 51, `total += lineTotal(line);`: `f`, type
`orders.js:51`, `ctrl-b`, then `esc` to close the finder. tuitor remembers it
for later runs. Click **Total again** in Chromium, and tuitor stops there.
Then:

- `c` runs one more iteration: `line` and `total` turn green with `▲` and
  their previous value.
- `tab` moves to LOCALS; `j` down to `order`, `l` expands it: `customer` →
  `address`. `j` down to `total`, `w` watches it.
- `s` steps into `lineTotal()` (STACK shows it above `orderTotal` and
  `button#again.render`: `render`, called by the button), `o` steps out.
- `tab` moves on to STACK; `j` selects `button#again.render`, and LOCALS shows
  its variables.
- `g h` shows every variable at every stop: `[` and `]` go back and forth
  through the stops, `}` back to the live one, `g o` back to OVERVIEW.
- `g p` shows the breakpoints and CONFIG, with both configurations from
  `.vscode/launch.json`. `E` opens the line under the SOURCE cursor in your
  editor (`$VISUAL` or `$EDITOR`).
- `O` opens another project in a new tab, such as `../js`; `<` and `>` switch
  tabs.
- `R` starts Chromium again; the page stops at the breakpoint while it loads.
  `Q` quits and closes Chromium.

## By hand

```sh
tuitor orders               # or "orders (headless)"
```
