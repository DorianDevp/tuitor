# FLOW

FLOW (`g f`) lists the files of a run in the order execution first reached them, with the
functions seen in each.

![FLOW of the PHP example over the time of an Xdebug trace](../assets/screenshots/flow.png)

| mark | means |
|---|---|
| `✓` | ran |
| `▶` | executes now |
| `▷` | further down the stack |
| `◇` | still to come |

## Where it comes from

| source | adapters |
|---|---|
| the stack of every stop of this run | any |
| earlier runs from the same starting stop (predict what comes next) | any |
| the newest finished Xdebug function trace through this stop (times every file, shows the rest of the request) | PHP |

| key | does |
|---|---|
| `enter` | open the file in CODE |
| `b` | breakpoint where the file is entered |
| `v` | show library files one by one |

## Xdebug traces

Turn on traces in the computerized format:

```ini
xdebug.mode = debug,trace
xdebug.start_with_request = trigger
xdebug.trace_format = 1
xdebug.output_dir = /tmp/xdebug
xdebug.trace_output_name = trace.%u.%R
```

Then `/config set xdebugTraceDir /tmp/xdebug`. A trace is complete once its request finished:
trace one request, debug the next one on the same path. Times leave out the debugger's stops.
