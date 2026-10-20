# Angular example

An Angular app that totals three orders when the page loads and again on
**Total again**: a loop whose values change at every stop, nested objects
(order → customer → address) and a call chain `ngOnInit()` → `render()` →
`orderTotal()` → `lineTotal()`. The component is `src/app/app.ts`; the orders
and the functions it calls are in `src/app/orders.ts`. Angular CLI 22.2.3
generated the rest:

```sh
ng new orders --minimal --ssr=false --routing=false --style=css --skip-git --ai-config=none --package-manager=npm
```

`ng serve` builds the app with source maps and serves it on
http://localhost:4200, and js-debug opens it in a Chromium of its own:
breakpoints go in the `.ts` files, and stops, STACK and stepping show them.

Needs Chromium, Node.js with npm (Angular 22 wants Node.js 22.22.3, 24.15.0,
26 or newer), curl and js-debug; see
[Installing adapters](../../README.md#installing-adapters).
`.vscode/launch.json` expects Chromium at `/usr/bin/chromium`, where Arch and
Debian put it; elsewhere, set `runtimeExecutable` there to your Chromium or
Chrome.

```sh
./run.sh               # checks those (and offers to install js-debug), installs Angular, starts ng serve and tuitor
HEADLESS=1 ./run.sh    # the same with Chromium without a window, e.g. over ssh
```

`ng serve` writes to `ng-serve.log`. Quitting tuitor (`Q`) closes Chromium,
and the script stops `ng serve`. `TUITOR=/path/to/tuitor ./run.sh` uses that
binary instead of building tuitor from this repository, and arguments go to
tuitor (`./run.sh --script demo.tsc`).

By hand:

```sh
npm ci              # Angular, once
npx ng serve        # in another terminal; tuitor does not run VS Code's preLaunchTask
tuitor "ng serve"
```

js-debug runs the page in a session of its own, apart from the browser's:
stops happen there, and breakpoints go there. tuitor shows the two as one
session, `ng serve › localhost:4200` on the context line: the configuration and
the page.

Put a breakpoint on line 67, `total += lineTotal(line);`: press `f`, type
`app.ts:67`, press `ctrl-b`, then `esc` to close the finder. Click **Total
again** in Chromium, and tuitor stops there: STACK shows `_App.orderTotal` and
`_App.render` in `app.ts`, called from the button's listener, which the
template's `(click)` (line 32) became. Then try:

- `c` runs one more iteration: `line` and `total` turn green with `▲` and
  their previous value.
- `tab` moves to LOCALS; `j` to `order`, `l` expands it: `customer` →
  `address`.
- `s` steps into `lineTotal()` in `orders.ts` (STACK shows it above
  `_App.orderTotal`), `o` steps out.
- `R` starts Chromium again; the page stops at the breakpoint while it loads,
  with `_App.ngOnInit` in STACK.
- `Q` quits and closes Chromium.

What the page logs (`console.log`) shows in OUTPUT.
