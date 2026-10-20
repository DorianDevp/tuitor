# Angular example

An Angular app that totals three orders when the page loads and on **Total
again**, served by `ng serve` and debugged in its `.ts` files through js-debug
in a Chromium of its own: a loop, nested objects (order → customer → address)
and a call chain `ngOnInit()` → `render()` → `orderTotal()` → `lineTotal()`.

The component is `src/app/app.ts`; the orders and the functions it calls are
in `src/app/orders.ts`. Angular CLI 22.2.3 generated the rest:

```sh
ng new orders --minimal --ssr=false --routing=false --style=css --skip-git --ai-config=none --package-manager=npm
```

Needs Chromium, Node.js with npm (Angular 22 wants Node.js 22.22.3 or a later
22, 24.15.0 or a later 24, or 26 and newer), curl and js-debug; see
[Installing adapters](../../README.md#installing-adapters).
`.vscode/launch.json` expects Chromium at `/usr/bin/chromium`, where Arch and
Debian put it; elsewhere, set `runtimeExecutable` there to your Chromium or
Chrome.

```sh
./run.sh               # Chromium with a window
HEADLESS=1 ./run.sh    # without one, e.g. over ssh
```

`run.sh` checks those, installs Angular into `node_modules` the first time
and after `package-lock.json` changed (`npm ci`), builds tuitor, has it offer
to install js-debug (`tuitor install`) when it is missing, starts `ng serve` on http://localhost:4200 (its output goes to
`ng-serve.log`), and starts tuitor on the configuration `ng serve` in
`.vscode/launch.json` (`ng serve (headless)` with `HEADLESS=1`). Quitting
tuitor closes Chromium, and `ng serve` stops after it.
`TUITOR=/path/to/tuitor ./run.sh` uses that binary instead of building tuitor
from this repository, and arguments go to tuitor (`./run.sh --script demo.tsc`).

`ng serve` builds the app with source maps, so breakpoints go in the `.ts`
files, and stops, STACK and stepping show them. js-debug runs the page in a
session of its own, apart from the browser's; tuitor shows the two as one
session: `ng serve › localhost:4200` in the header, the configuration and the
page. What the page logs (`console.log`) shows in OUTPUT.

## A first session

Put a breakpoint on line 67 of `app.ts`, `total += lineTotal(line);`: `f`,
type `app.ts:67`, `ctrl-b`, then `esc` to close the finder. tuitor remembers it
for later runs. Click **Total again** in Chromium, and tuitor stops there:
STACK shows `_App.orderTotal` and `_App.render` in `app.ts`, called from the
button's listener, which the template's `(click)` (line 32) became. Then:

- `c` runs one more iteration: `line` and `total` turn green with `▲` and
  their previous value.
- `tab` moves to LOCALS; `j` down to `order`, `l` expands it: `customer` →
  `address`. `j` down to `total`, `w` watches it.
- `s` steps into `lineTotal()` in `orders.ts` (STACK shows it above
  `_App.orderTotal`), `o` steps out.
- `tab` moves on to STACK; `j` selects `_App.render`, and LOCALS shows its
  variables.
- `g f` shows FLOW: the files of the run in the order execution reached them,
  `app.ts` and `orders.ts` among Angular's, and `g c` shows CODE, with FLOW
  and the functions of the file in SOURCE beside it. `g o` back to OVERVIEW.
- `g p` shows the breakpoints and CONFIG, with both configurations from
  `.vscode/launch.json`. `E` opens the line under the SOURCE cursor in your
  editor (`$VISUAL` or `$EDITOR`).
- `O` opens another project in a new tab, such as `../web`; `<` and `>`
  switch tabs.
- `R` starts Chromium again; the page stops at the breakpoint while it loads,
  with `_App.ngOnInit` in STACK. `Q` quits and closes Chromium.

## By hand

```sh
npm ci              # Angular, once
npx ng serve        # in another terminal; tuitor does not run VS Code's preLaunchTask
tuitor "ng serve"   # or "ng serve (headless)"
```
