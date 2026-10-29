# The screen, CODE and tabs

## OVERVIEW (`g o`)

SOURCE, STACK, LOCALS, WATCH, BREAKPOINTS, OUTPUT and EVENTS on one screen. `z` zooms a pane,
`tab` moves between them.

![OVERVIEW stopped at a breakpoint: SOURCE with values inline, LOCALS with deltas](../assets/screenshots/overview.png)

| you see | means |
|---|---|
| amber | where execution is |
| green, `▲` and the old value | changed since the previous stop |
| `△` | same text, changed inside (an object's field, a pointer's target) |
| red | an error |
| `◉` | the breakpoint this stop hit |

- SOURCE shows values beside the lines that use them.
- LOCALS shows the frame selected in STACK; `x` evaluates in it.
- WATCH (`w`) evaluates in the top frame at every stop, with recent values and a trend.

## CODE (`g c`)

SOURCE with the whole stack and the breakpoints, plus FLOW and OUTLINE on the left. OUTLINE lists
the file's functions (PHP, JS/TS, Rust, C, Go): whether each ran, its callers and callees. `│` marks
lines a stop or a trace reached.

![CODE in the Angular example: FLOW, OUTLINE and the stack](../assets/screenshots/code.png)

## Tabs

One tab = one project, with its own configurations, sessions, views and history.

| key | does |
|---|---|
| `O` | open a project (recent ones are offered) |
| `T` | new tab |
| `<` `>` | switch tabs |
| `ctrl-w` | close a tab (asks while a session is live) |

The top bar shows each tab's state: `■` stopped, `▶` running, `⧗` starting, `✕` ended. js-debug's
child sessions show as part of the session that started them.

![The Angular example with the PHP example in a second tab](../assets/screenshots/tabs.png)
