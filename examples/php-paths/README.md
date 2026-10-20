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
`config.toml` naming it. PATHS asks intelephense for references and
definitions, which need no licence.

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
- `P` asks intelephense for the paths through that line. PATHS shows a tree
  for each of `InvoiceController::print` and `OrderController::show`, for
  `LoyaltyListener::onOrderTotalled`, which no code calls by name, and for the
  test. Each controller loads the order from the repository, totals it in
  `OrderService::total`, which calls the helper, and formats the total with
  `Money::format`. The cursor is on the helper under the invoice controller,
  and nothing is marked.
- `m` marks the path from `InvoiceController::print` through the service to
  the helper, in lavender. `k` `k` goes up to `Money::format` and `space`
  puts it in lavender too, `k` `space` the repository: the request goes on
  past the helper.
- `j` eight times to the helper under `OrderController::show`, `m`: cyan,
  the order's path; `j` `j` `space` puts its `Money::format` in cyan.
  `enter` makes lavender and cyan breakpoint sets, lavender active, with the
  lines of the service and of both helpers in both.
- `c` runs the invoice request: it stops at lavender's breakpoints 19 times.
- Once it has ended, the order request stops on its first line. `A` makes the
  next set, cyan, active; `c` runs it through cyan's 16 stops. The
  repository's breakpoint is in lavender alone: it records the stop and the
  request goes on.
- `:compare lavender cyan` lines up what lavender recorded in the first
  request and cyan in the second: the hits of the breakpoints they share with
  the number of values that differ, those of each controller alone and the
  repository's on lavender's side. `j` down to `OrderService.php:23 hit 1`
  lists the six values that differ, `$order.id` 1002 against 1001 among
  them; `enter` opens the breakpoint in CODE.
- `g h` shows HISTORY; `tab` moves to TIMELINE, a row for each value that
  changed with a mark at each stop where it did, in the colour of the stop's
  set, and `↷` above the stop where the repository's breakpoint recorded and
  the program went on. `j` to `$total`, `h` and `l` go to its changes,
  `enter` opens one in OVERVIEW.
- `f`, `Money.php:16`, `enter` and `P` again: FLOW has now seen the listener
  called from `Dispatcher::dispatch`, which no language server can, so the
  service's paths part two ways (`⋔`), the second through the dispatcher to the
  listener, marked `⇢`.
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
