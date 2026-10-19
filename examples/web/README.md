# Web example

`index.html` loads `orders.js`, which totals three orders when the page loads
and again on **Total again**: a loop whose values change at every stop, nested
objects (`Order` → `Customer` → `Address`) and a call chain `render()` →
`orderTotal()` → `lineTotal()`. js-debug opens the page in a Chromium of its
own, straight from disk (`file://`), so no web server is needed.

Needs Chromium, Node.js and js-debug; see
[Installing adapters](../../README.md#installing-adapters).
`.vscode/launch.json` expects Chromium at `/usr/bin/chromium`, where Arch and
Debian put it; elsewhere, set `runtimeExecutable` there to your Chromium or
Chrome.

```sh
./run.sh               # checks those (and offers to install js-debug), builds tuitor, starts tuitor
HEADLESS=1 ./run.sh    # the same with Chromium without a window, e.g. over ssh
```

Chromium starts with a fresh profile, and quitting tuitor (`Q`) closes it and
ends the script. `TUITOR=/path/to/tuitor ./run.sh` uses that binary instead of
building tuitor from this repository, and arguments go to tuitor
(`./run.sh --script demo.tsc`).

By hand: `tuitor orders` in this directory.

js-debug runs the page in a session of its own: the top bar shows `web·js`,
the browser, and `web·js › index.html`, the page. Stops happen in the second
one, and breakpoints go there.

Put a breakpoint on line 51, `total += lineTotal(line);`: press `f`, type
`orders.js:51`, press `ctrl-b`, then `esc` to close the finder. Click
**Total again** in Chromium, and tuitor stops there. Then try:

- `c` runs one more iteration: `line` and `total` turn green with `▲` and
  their previous value.
- `tab` moves to LOCALS; `j` to `order`, `l` expands it: `customer` →
  `address`.
- `s` steps into `lineTotal()` (STACK shows it above `orderTotal` and
  `render`, which the button called), `o` steps out.
- `R` starts Chromium again; the page stops at the breakpoint while it loads.
- `Q` quits and closes Chromium.

What the page logs (`console.log`) shows in OUTPUT.
