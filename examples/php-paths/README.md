# php-paths example

A small PHP application laid out as a Symfony one is: a front controller
(`public/index.php`), a kernel that wires the services and routes a request to
a controller by name, two controllers, `InvoiceController::print` and
`OrderController::show`, that both total an order through
`OrderService::total`, which calls the helper `Money::lineTotal` for each line,
and a listener that the kernel registers for the `OrderTotalled` event, which
calls the helper again. It is made for PATHS (`P`), breakpoint sets, COMPARE
and TIMELINE: two ways into one service, taken in two requests.

Needs PHP with Xdebug 3, Node.js and vscode-php-debug, as
[`../php`](../php) does (see
[Installing adapters](../../README.md#installing-adapters)), and for PATHS
intelephense: `npm install -g intelephense`, or `[lsp.php]` in tuitor's
`config.toml` naming it. PATHS asks intelephense for references, which need no
licence.

```sh
./run.sh                    # with the php on PATH
PHP=/path/to/php ./run.sh   # with another php, one that has Xdebug
```

`run.sh` checks those, builds tuitor, has it offer to install
vscode-php-debug (`tuitor install`) when it is missing, and starts it on the
configuration `Listen for Xdebug` in `.vscode/launch.json`, which listens on
port 9003 and stops on the first line of each request. Once tuitor listens,
PHP handles two requests, one after the other: `invoice.print 1002`, then
`order.show 1001`. Their output goes to `run.log`, which starts with the
command that sends another request from another terminal. Quitting tuitor
stops PHP too. `TUITOR=/path/to/tuitor ./run.sh` uses that binary instead of
building tuitor from this repository, and arguments go to tuitor
(`./run.sh --script demo.tsc`).

## A first session

The first request stops on the first line of `public/index.php`.

- `f`, type `Money.php:16`, `enter`: SOURCE shows the helper at the bulk
  rebate, `$amount = intdiv($amount * 95, 100);`.
- `P` asks intelephense for the paths to that line. PATHS lists four:
  `InvoiceController::print` and `OrderController::show` through
  `OrderService::total`, `LoyaltyListener::onOrderTotalled`, which no code
  calls by name, and the test. `j` moves to a path and shows its lines,
  `space` marks it in the colour of its set to be: `space j space j space`
  marks all but the test, lavender for the invoice, cyan for the order, pink
  for the listener. `enter` makes each path marked a breakpoint set, and
  lavender active.
- `c` runs the invoice request: it stops at the lavender breakpoints, 14
  times, among them those of `OrderService` and `Money`, which cyan and pink
  share.
  The breakpoints of `OrderController` and the listener only record the stop
  and the request goes on.
- Once it has ended, the order request stops on its first line. `A` makes the
  next set, cyan, active; `c` runs it through cyan's 14 stops.
- `:compare lavender cyan` lines up what lavender recorded in the first
  request and cyan in the second: the hits of the breakpoints they share with the number
  of values that differ, and those of each controller alone. `j` down to
  `OrderService.php:23 hit 1` lists the six values that differ, `$order.id`
  1002 against 1001 among them; `enter` opens the breakpoint in CODE. `g d`
  alone would compare cyan with pink, whose last recording is the latest.
- `g h` shows HISTORY; `tab` moves to TIMELINE, a row for each value that
  changed with a mark at each stop where it did, in the colour of the stop's
  set, and `↷` above the stops where the listener's breakpoints recorded and
  the program went on. `j` to `$total`, `h` and `l` go to its changes,
  `enter` opens one in OVERVIEW.
- `f`, `Money.php:16`, `enter` and `P` again: FLOW has now seen the listener
  called from `Dispatcher::dispatch`, which no language server can, so PATHS
  also lists both controllers' paths through it, marked `⇢`.
- `Q` quits.

## By hand

Xdebug connects to the debugger, so tuitor starts first and waits.

1. Terminal 1, in this directory: `tuitor "Listen for Xdebug"`.
2. Terminal 2, in this directory, one request at a time:

   ```sh
   XDEBUG_MODE=debug XDEBUG_SESSION=1 php public/index.php invoice.print 1002
   XDEBUG_MODE=debug XDEBUG_SESSION=1 php public/index.php order.show 1001
   ```

The orders are 1001, 1002 and 1003, in `src/Repository/OrderRepository.php`.
`public/index.php` loads the classes itself, as Composer's autoloader would
for `composer.json`'s PSR-4 rule, so there is nothing to install.
