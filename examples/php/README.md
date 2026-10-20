# PHP example

`orders.php` totals three orders, debugged through Xdebug and
vscode-php-debug, on this machine or in a container: a loop, nested objects
(`Order` → `Customer` → `Address`) and a call chain `orderTotal()` →
`lineTotal()`.

Needs PHP with Xdebug 3, Node.js and vscode-php-debug; see
[Installing adapters](../../README.md#installing-adapters).

```sh
./run.sh                    # with the php on PATH
PHP=/path/to/php ./run.sh   # with another php, one that has Xdebug
```

`run.sh` checks those, builds tuitor, has it offer to install
vscode-php-debug (`tuitor install`) when it is missing, starts it on the configuration `Listen for Xdebug` in `.vscode/launch.json`,
which listens on port 9003 and stops on the first line, and runs `orders.php`
once tuitor listens. PHP's output goes to `run.log` (`tail -f run.log` follows
it), which starts with the command that runs PHP again from another terminal.
Quitting tuitor stops PHP too. `TUITOR=/path/to/tuitor ./run.sh` uses that
binary instead of building tuitor from this repository, and arguments go to
tuitor (`./run.sh --script demo.tsc`).

## A first session

Put a breakpoint on line 68, `$total += lineTotal($line);`: `f`, type
`orders.php:68`, `ctrl-b`, then `esc` to close the finder. tuitor remembers it
for later runs. Then:

- `c` runs to the breakpoint, `c` again runs one more iteration: `$total`
  turns green with `▲` and its previous value.
- `tab` moves to LOCALS; `j` down to `$order`, `l` expands it: `customer` →
  `address`. `j` down to `$total`, `w` watches it.
- `s` steps into `lineTotal()` (STACK shows three frames). `o` first stops
  on its return value (`$__RETURN_VALUE`), `o` again is back in `orderTotal()`.
- `tab` moves on to STACK; `j` selects `{main}`, the script, and LOCALS shows
  its variables.
- `g h` shows every variable at every stop: `[` and `]` go back and forth
  through the stops, `}` back to the live one, `g o` back to OVERVIEW.
- `g p` shows the breakpoints and CONFIG, with both configurations from
  `.vscode/launch.json`. `E` opens the line under the SOURCE cursor in your
  editor (`$VISUAL` or `$EDITOR`).
- `O` opens another project in a new tab, such as `../js`; `<` and `>` switch
  tabs.
- `c` until PHP finishes. tuitor keeps listening: run PHP again (step 2
  below, or the command in `run.log`) and it stops on the first line again,
  `c` runs to the same breakpoint. `Q` quits.

## By hand

Xdebug connects to the debugger, so tuitor starts first and waits.

1. Terminal 1, in this directory: `tuitor "Listen for Xdebug"`.
2. Terminal 2, in this directory:

   ```sh
   XDEBUG_MODE=debug XDEBUG_SESSION=1 php orders.php
   ```

   `XDEBUG_MODE=debug` turns on step debugging and `XDEBUG_SESSION` makes
   Xdebug connect for this run
   ([Xdebug docs](https://xdebug.org/docs/step_debug#activate_debugger)).

## In a container

`compose.yaml` runs the same script in the official `php:8.4-cli` image, with
Xdebug installed by [PIE](https://github.com/php/pie) as Xdebug's install docs
describe. Xdebug connects out to tuitor on the host through
`host.docker.internal`, and `pathMappings` in `.vscode/launch.json` maps the
container's `/app` back to this directory. Step 1 starts the other
configuration, `tuitor "Listen for Xdebug in Docker"`, which stops on the first
line too, and step 2 becomes:

```sh
docker compose run --rm php
```

## Traces for FLOW

`TRACE=1 ./run.sh` has Xdebug also write a function trace of the run, in the
format tuitor reads, to `traces/`, which both configurations name as
`xdebugTraceDir` (the Docker one as `/app/traces`, which `pathMappings` maps
back here). Trace one run; in the next one, traced or not, FLOW (`g f`) times
`orders.php` from the earlier trace and marks where the breakpoint on line 68
comes round again. In the container, after `mkdir -p traces` (Xdebug makes no
directory): `XDEBUG_MODE=debug,trace docker compose run --rm php`.
