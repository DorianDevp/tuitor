# Breakpoint sets, COMPARE and PATHS

[`examples/php-paths`](../examples/php-paths) walks through all three: PATHS to a helper,
its paths taken as sets, one request with each set active, and COMPARE of the two.

## Sets

Every breakpoint is in one set or more. Sets have no names, only a colour each: rose, teal,
orchid, sky and gold, so there are five at most. One set is active, and the breakpoints in it
stop the program. A breakpoint only in other sets records the stop as every stop is recorded,
with the stack, the top frame's variables read ahead and the watches, and the program goes on
by itself. Sets share breakpoints: two ways into one service, a controller and a command, can
each have a set, both with the service's breakpoints. Every enabled breakpoint stays with the
adapter, so a new active set takes effect at its next hit, also while the program runs. A step
that ends on a breakpoint of another set stops there as asked.

| key | command | |
|---|---|---|
| `N` | `/set new [file:line…]` | a set in the first free colour, active from then on, with the breakpoints at those places, made where there are none, else the one selected in BREAKPOINTS or on the SOURCE line, if there is one |
| `A` | `/set next`, `/set prev`, `/set active teal` | the next, previous or a given set active |
| `M` | `/set toggle [set]` | the breakpoint selected, or on the SOURCE line, in the active set (or the one named), or out of it |
| | `/set add teal orders.php:68`, `/set remove teal` | breakpoints into a set, made where there are none, or out of it; without places, the selected one |
| `D` in BREAKPOINTS | `/set delete [set] [set]` | the set the selected breakpoint is shown in, else the active one |

Breakpoints added with `b` go to the active set. A breakpoint stays in one set at least: taken
out of its last set, it is not. Deleting a set deletes its breakpoints that are in no other set
once `D` is pressed again; `/set delete teal rose` puts them in rose instead. Sets are kept with
the breakpoints in `~/.local/state/tuitor/`; breakpoints from before there were sets are in
rose.

A breakpoint has the colour of the active set when it is in it, else of the first set it
joined, in SOURCE, BREAKPOINTS, FLOW, OUTLINE, HISTORY and the context line: full when it stops
the program, dimmer when it only records. A smaller `•` after it, in another set's colour, says
it is in more sets, and BREAKPOINTS has a dot for each set it is in. The shape is what it does:
`●` stops, `◐` has a condition or a hit count, `◆` logs, `○` is disabled, `◌` was not taken by
the adapter, and `◉` is the one the stop on view hit. HISTORY marks the stops the program went
on from with `↷`, and EVENTS logs them as `passed`, with the time the stop took to read.

A breakpoint of another set costs what reading a stop costs. tuitor asks for the variables of
a stop together, a level at a time. Measured on 2026-10-20 between consecutive hits of such a
breakpoint in the C, PHP and JavaScript examples, three runs each: 4 to 6 ms with gdb, 3 to 5 ms
with Xdebug and vscode-php-debug, and about 260 ms with js-debug, which takes 45 to 130 ms to
answer each `variables` request.

## COMPARE

COMPARE (`g d`, `/compare`) lines up what two sets recorded, each the stops at its breakpoints
in one run: set A in one request, set B in another. A stop lines up with the stop at the same
breakpoint that is the same hit of it on the other path: the first with the first, the second
with the second. These are hits of breakpoints the sets share, and for them the values each
stop read, the watches and the top frame's variables at every level read, are compared: the
row counts those that differ, appear and disappear, and VALUES below lists them for the hit
under the cursor, those that differ first. A hit only one path has, at a breakpoint of one set
or one the other path did not reach, is listed with what it read. Hits on both keep the order
of the first path, and the others follow the last of those before them by time, so two sets of
one run are in the order things happened.

Each set is compared in its own run: the latest in which it was the active set and stopped the
program, else the latest that recorded it.

| command | compares |
|---|---|
| `/compare`, `g d` | the active set with the other set whose own run is the latest; with no other, the active set's latest run with the run before |
| `/compare teal` | the active set with teal |
| `/compare rose teal` | rose with teal |
| `/compare rose@3 rose@5` | rose in FLOW's runs 3 and 5 |

In COMPARE, `h` and `l` pick the breakpoint or a side, `[` and `]` take that side, or on the
breakpoint both, from an older or newer run, and `enter` opens the breakpoint in CODE or the
side's stop in HISTORY. Only the runs of the tuitor that is running can be compared: FLOW
remembers earlier ones by their stacks, without values.

## PATHS

`P` (`/paths`) asks the project's language server who calls the function the SOURCE line is
in, and who calls those, up to where execution starts: a controller, `main`, a test, a script's
top level, or a function no code calls, which a framework, a container or an event does. PATHS
lists each chain, entry point first and tests last, with the lines on it worth a breakpoint:
the entry of each function, its call into the next one, before that call the last write of
each variable and the last call into the project's other functions (three at most), and the
line asked about. `j` and `k` pick a path and show its lines, `space` marks it or unmarks it.
As many paths are marked as there are sets for, a set with no breakpoint or a colour no set
has. `enter` makes each path marked a breakpoint set of its own, in the colour its mark has,
and the first of them the active set: a line several paths share is one breakpoint in each of
their sets, and COMPARE lines up two paths at the lines they share.

Calls no server can see, such as a container's, an event's or one through an interface, come
from the stacks of the runs FLOW remembers, and show as `⇢`. tuitor runs intelephense (PHP),
typescript-language-server (TypeScript, JavaScript, Angular), rust-analyzer (Rust) and gopls
(Go) from `PATH`, or as the `[lsp]` table of `config.toml` says
([configuration](configuration.md#configtoml)), once per project when first asked, and ends
them with the tab; PATHS shows what a server reports while it reads the project. intelephense
answers no call hierarchy, so for PHP the callers are its references that call; references
need no intelephense licence. Other language servers are welcome as contributions: the
`tuitor-code` crate is the only one that speaks LSP.
