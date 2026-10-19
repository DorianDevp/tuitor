# PHP example

`orders.php` totals three orders: a loop whose values change at every stop,
nested objects (`Order` → `Customer` → `Address`) and a call chain
`orderTotal()` → `lineTotal()`.

Needs PHP with Xdebug 3, Node.js and vscode-php-debug; see
[Installing adapters](../../README.md#installing-adapters).

Xdebug connects to the debugger, so tuitor starts first and waits:

1. Terminal 1, in this directory: `tuitor "Listen for Xdebug"`. It listens
   on port 9003 (`.vscode/launch.json`).
2. Put a breakpoint on line 68, `$total += lineTotal($line);`: press `f`,
   type `orders.php:68`, press `ctrl-b`, then `esc` to close the finder.
3. Terminal 2, in this directory:

   ```sh
   XDEBUG_MODE=debug XDEBUG_SESSION=1 php orders.php
   ```

   `XDEBUG_MODE=debug` turns on step debugging and `XDEBUG_SESSION` makes
   Xdebug connect for this run
   ([Xdebug docs](https://xdebug.org/docs/step_debug#activate_debugger)).

PHP stops at the breakpoint. Then try:

- `c` runs one more iteration: `$total` turns green with `▲` and its previous
  value.
- `tab` moves to LOCALS; `j` to `$order`, `l` expands it: `customer` →
  `address`.
- `s` steps into `lineTotal()` (STACK shows three frames). `o` first stops
  on its return value (`$__RETURN_VALUE`), `o` again is back in `orderTotal()`.
- `w` on `$total` in LOCALS watches it; `g h` shows it at every stop, `[` and
  `]` go back and forth through the stops, `g o` returns.
- `c` until PHP finishes. tuitor keeps listening: run step 3 again and it
  stops at the same breakpoint. `Q` quits.

## In a container

`compose.yaml` runs the same script in the official `php:8.4-cli` image, with
Xdebug installed by [PIE](https://github.com/php/pie) as Xdebug's install docs
describe. Xdebug connects out to tuitor on the host through
`host.docker.internal`, and `pathMappings` in `.vscode/launch.json` maps the
container's `/app` back to this directory. Step 1 starts the other configuration,
`tuitor "Listen for Xdebug in Docker"`, step 2 stays the same, and step 3
becomes:

```sh
docker compose run --rm php
```
