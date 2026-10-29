# Debug PHP with Xdebug in the terminal

tuitor debugs PHP through Xdebug 3 and vscode-php-debug, the same adapter as VS Code's PHP Debug.

```sh
tuitor install php                 # vscode-php-debug (needs Node.js 20.18+)
php -v                             # must mention Xdebug; else install it (pie install xdebug/xdebug)
tuitor "Listen for Xdebug"         # or, without a launch.json: tuitor listen
```

In another terminal:

```sh
XDEBUG_MODE=debug XDEBUG_SESSION=1 php orders.php
```

Xdebug connects to tuitor on port 9003. For web requests, send the Xdebug trigger (cookie or query)
instead of `XDEBUG_SESSION`. tuitor keeps listening: each request is a new run.

![OVERVIEW stopped in the PHP example: values inline, LOCALS with deltas, two watches](../assets/screenshots/overview.png)

## Try it

```sh
cd examples/php && ./run.sh        # PHP=/path/to/php ./run.sh for another PHP
```

Then `f`, `orders.php:68`, `ctrl-b`, `esc`, and `c`.

## PHP extras

- **FLOW with timings.** With Xdebug function traces, [FLOW](../features/flow.md) times every file
  and shows what the request does after the current stop. `TRACE=1 ./run.sh` in the example.
- **Call paths.** `P` asks intelephense for the call trees through a line; see [PATHS](../sets.md#paths).
- **Hit counts** use vscode-php-debug's syntax: `% 2`, `>= 3`.
- **Docker:** see [Debug PHP in Docker](php-docker.md).
