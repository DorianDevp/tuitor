# Sets, COMPARE and PATHS

The [`php-paths` example](https://github.com/DorianDevp/tuitor/tree/master/examples/php-paths) walks
through all three.

## Sets

<video autoplay loop muted playsinline poster="assets/demo/sets.png" width="100%">
  <source src="assets/demo/sets.webm" type="video/webm">
  <source src="assets/demo/sets.mp4" type="video/mp4">
</video>

Every breakpoint is in one or more sets. A set has a colour, not a name.

- Breakpoints of the **active** set stop the program.
- Breakpoints only in **other** sets record the stop (stack, top frame's variables, watches) and let
  the program go on. HISTORY marks those stops `↷`.
- Sets can share breakpoints, e.g. two entry points into one service.
- Switching the active set works while the program runs.

| key | command | does |
|---|---|---|
| `N` | `/set new [file:line…]` | new set, made active |
| `A` | `/set next`, `/set prev`, `/set active teal` | change the active set |
| `g a` | `/set pick` | list the sets; `enter` activates |
| `M` | `/set toggle [set]` | the selected breakpoint into or out of the active set |
| | `/set add teal orders.php:68`, `/set remove teal` | add or remove breakpoints |
| `D` in BREAKPOINTS | `/set delete [set] [into-set]` | delete a set (press twice to also delete its own breakpoints) |

`b` adds to the active set. A breakpoint is full colour when it stops, dimmer when it only records;
a small `•` shows it is in more sets.

![OVERVIEW at a breakpoint of lavender, the active set, which cyan shares](assets/screenshots/sets.png)

**Cost of a recording hit** (2026-10-20, examples): 4–6 ms with gdb, 3–5 ms with Xdebug, about
260 ms with js-debug.

## COMPARE

COMPARE (`g d`) lines up what two sets recorded, each in its own run, e.g. one set in one request
and another set in the next. The same hit of a shared breakpoint lines up on both sides; the row
counts values that differ, appear or disappear, and VALUES lists them. Hits only one side has are
listed too.

| command | compares |
|---|---|
| `/compare`, `g d` | the active set with the other set that ran last |
| `/compare teal` | the active set with teal |
| `/compare rose teal` | rose with teal |
| `/compare rose@3 rose@5` | rose in FLOW's runs 3 and 5 |

In COMPARE: `h` `l` pick a side, `[` `]` take it from an older or newer run, `enter` opens the
breakpoint in CODE or the stop in HISTORY. Runs saved on disk compare too, across commits:
breakpoints follow their lines, and each side shows its commit.

![COMPARE: lavender in one request against cyan in another](assets/screenshots/compare.png)

## PATHS

`P` on a SOURCE line asks the language server for the call trees through its function: up to where
execution starts (a controller, `main`, a test, a script), and down a few calls past the line.

| key | command | does |
|---|---|---|
| `m` | `/paths mark` | mark the end-to-end path through the stage under the cursor |
| `space` | `/paths stage` | one stage in or out of the mark |
| `b` | `/paths branch` | a stage and all below it in or out |
| `u` | `/paths unmark` | drop the mark |
| `[` `]` | `/paths prev`, `/paths next` | previous, next mark |
| `h` `l` | | fold, unfold |
| `enter` | `/paths take` | each mark becomes a breakpoint set |

`◆` is the line's function, `⋔` where paths part, `⇢` a call seen only in an earlier run's stack.
Under the trees: the lines worth a breakpoint in the stage under the cursor.

![PATHS through a helper: a tree per controller and the test, two marked paths](assets/screenshots/paths.png)

| language | server | blind spots |
|---|---|---|
| PHP | intelephense (no licence needed) | container, router and event-dispatcher calls; `$callable()` |
| TypeScript, JavaScript, Angular | typescript-language-server | template bindings, DI tokens, callbacks |
| Rust | rust-analyzer | trait objects, function pointers, calls inside macro arguments |
| Go | gopls | interfaces, function values |

Run the program and ask again: calls seen on its stacks join the trees. PATHS is bounded (8 calls
up, 3 below the path, 8 trees, 400 stages) and says what a bound cut short. Servers are configured in
[`config.toml`](configuration.md#configtoml).
