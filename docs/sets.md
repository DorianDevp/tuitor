# Breakpoint sets, COMPARE and PATHS

[`examples/php-paths`](../examples/php-paths) walks through all three: PATHS through a helper,
two of its paths taken as sets, one request with each set active, and COMPARE of the two.

## Sets

Every breakpoint is in one set or more. Sets have no names, only a colour each, chosen when
the set is made ([colours](#colours)), and there can be any number of them. One set is active,
and the breakpoints in it stop the program. A breakpoint only in other sets records the stop as every stop is recorded,
with the stack, the top frame's variables read ahead and the watches, and the program goes on
by itself. Sets share breakpoints: two ways into one service, a controller and a command, can
each have a set, both with the service's breakpoints. Every enabled breakpoint stays with the
adapter, so a new active set takes effect at its next hit, also while the program runs. A step
that ends on a breakpoint of another set stops there as asked.

| key | command | |
|---|---|---|
| `N` | `/set new [file:line…]` | a set in a colour of its own, active from then on, with the breakpoints at those places, made where there are none, else the one selected in BREAKPOINTS or on the SOURCE line, if there is one |
| `A` | `/set next`, `/set prev`, `/set active teal` | the next, previous or a given set active |
| `g a` | `/set pick`, `/set active` | the sets in a list, each with its colour, what it does and its breakpoints; `enter` makes the one selected active |
| `M` | `/set toggle [set]` | the breakpoint selected, or on the SOURCE line, in the active set (or the one named), or out of it |
| | `/set add teal orders.php:68`, `/set remove teal` | breakpoints into a set, made where there are none, or out of it; without places, the selected one |
| `D` in BREAKPOINTS | `/set delete [set] [set]` | the set the selected breakpoint is shown in, else the active one |

Breakpoints added with `b` go to the active set. A breakpoint stays in one set at least: taken
out of its last set, it is not. Deleting a set deletes its breakpoints that are in no other set
once `D` is pressed again; `/set delete teal rose` puts them in rose instead. Sets are kept, by
colour, with the breakpoints in `~/.local/state/tuitor/`; sets from before colours were chosen
keep theirs (rose, teal, orchid, sky and gold), and breakpoints from before there were sets are
in rose.

A breakpoint has the colour of the active set when it is in it, else of the first set it
joined, in SOURCE, BREAKPOINTS, FLOW, OUTLINE, HISTORY and the context line: full when it stops
the program, dimmer when it only records. A smaller `•` after it, in another set's colour, says
it is in more sets, and BREAKPOINTS has a dot for each set it is in, four at most, then `+n`
for the rest. Its title shows every set up to seven; past that, the active one between the two
on each side, those `A` and `/set prev` go to, and its place among them all, as `3/12`. The shape is what it does:
`●` stops, `◐` has a condition or a hit count, `◆` logs, `○` is disabled, `◌` was not taken by
the adapter, and `◉` is the one the stop on view hit. HISTORY marks the stops the program went
on from with `↷`, and EVENTS logs them as `passed`, with the time the stop took to read.

### Colours

A new set's colour is chosen in OKLab, where equal distances look equally far apart. The
candidates are hues 2° apart at a lightness and chroma that read well on the dark background
(OKLCH 0.76, 0.13), then the same hues lighter and darker (0.85, 0.10 and 0.66, 0.13), then four
shades between (0.71 and 0.81, at chroma 0.09 and 0.16). A set takes the candidate farthest
from the colours of the other sets and of the states, amber, green, red and the yellow of
warnings, as long as that is ΔE 0.12 at least: from the first candidates if one is, else from
the first and the lighter and darker ones, else from all. When none is, the bar is lowered by
15 % and the candidates looked at again, so the later sets are closer together; below ΔE 0.01
any colour no set has will do. Whatever the bar, a set's colour is ΔE 0.11 at least from each
state's, with lightness counting half, since a darker amber still reads as amber, and 20° of
hue from it, since a paler red still reads as red.

The same sets give the same colour. A new project's sets are lavender, cyan, pink, deep-blue,
pale-sky and deep-orchid, the six that clear ΔE 0.12, then deep-teal, pale-mauve, pale-teal and
deep-pink, each 0.09 to 0.12 from its nearest colour; the fourteenth is 0.067 from its nearest,
and the hundredth about 0.02. A deleted set's colour is free, and the next set made with the
same others takes it.

Commands know a set by the word for its colour: its hue's, such as `cyan`, after `pale-` or
`deep-` when it is lighter or darker than most and `dusty-` or `bright-` when it is duller or
more vivid, with a number after it when a set made before it has the same word (`pink2`), or
the colour itself, `#ee8dbd`.

### Cost

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

`P` (`/paths`) asks the project's language server for the call trees through the function the
SOURCE line is in, the anchor:

- Up, who calls the anchor, and who calls those, up to where execution starts: a controller,
  `main`, a test, a script's top level, or a function no code calls, which a framework, a
  container or an event does. Each such place is the root of a tree; controllers, `main` and
  scripts come first, then the functions no code calls, then tests.
- Down, what every function on a way from a root to the anchor calls, and the anchor itself,
  and what those calls call, as far as three calls below the way, in the project's code only:
  `vendor` and `node_modules` are left out. Calls are asked of `callHierarchy/outgoingCalls`,
  or, from intelephense, which has no call hierarchy, of the `textDocument/definition` of each
  name the text calls, kept when it is a function of the project.
- Calls no server can see, such as a container's, an event dispatcher's or one through an
  interface, come from the stacks of the runs FLOW remembers, in both directions, and show as
  `⇢`.

A stage is a call: a function, under the stage that calls it, in the order of the lines that
make the calls. A chain from a root down to a stage with nothing below it that passes the
anchor is an end-to-end path. The anchor is `◆` wherever it is called, `⋔ 2 ways` marks a stage
where the end-to-end paths through it part, `↻ again` a call of a function above it in the
chain, followed once, and `…` a stage whose calls were not asked for. Branches off the
end-to-end paths start folded.

Below the trees are the lines of the stage under the cursor worth a breakpoint: the entry of
its function, its calls into the stages chosen below it, in the anchor the line asked about,
and before the last of those the last write of each variable and the last call into the
project's other functions (three at most). Chosen means in the current mark when the stage is
in it, else on the end-to-end path through it.

| key | command | |
|---|---|---|
| `m` | `/paths mark` | mark the end-to-end path through the stage under the cursor: its chain from the root and, below it, the first call on an end-to-end path, else the first call, down to a stage with nothing below it |
| `space` | `/paths stage` | the stage under the cursor in the current mark, or out of it; with no mark, a new one |
| `b` | `/paths branch` | the stage under the cursor and every stage below it, in the current mark or out of it |
| `u` | `/paths unmark` | drop the current mark |
| `[` `]` | `/paths prev`, `/paths next` | the mark before or after is current |
| `h` `l` | `/left`, `/right` | fold a stage's calls, or go to its caller; unfold them, or go to the first |
| `enter` | `/paths take` | a breakpoint set for each mark |

Nothing is marked at first. A mark has the colour of the set it is to be from when it is made:
a set with no breakpoint, else a colour chosen as a new set's is ([colours](#colours)), apart
from the sets' and those of the other marks. Unmarking frees the colour, and a mark left with
no stage is unmarked. `enter` makes each mark a breakpoint set on the lines of its stages, made
where there is no breakpoint and joined where there is one, and the first mark's set the
active one: a line several marks share is one breakpoint in each of their sets, and COMPARE
lines up two paths at the lines they share.

PATHS is bounded, and says in its notes what each bound cut short:

| bound | |
|---|---|
| 8 calls up | callers of a function further from the anchor are not asked for; its tree is `cut short` |
| 10 callers | of a function, the first followed |
| 3 calls below the paths | what a stage further below a way to the anchor calls is not asked for (`…`) |
| 12 calls | of a function, the first in its text followed |
| 300 requests up, 300 down | past them no function is asked about, and the graph found so far is the answer |
| 8 trees | the roots nearest the anchor kept |
| 400 stages | in all the trees |
| 600 requests for lines | past them, which lines write comes from the text alone |

What no server sees, it says too: in PHP the calls a container, a router or an event dispatcher
makes and calls of callables (`$listener($event)`, `$controller->$action()`); in TypeScript the
calls from templates, Angular's bindings and events included, and through DI tokens and
callbacks; in Rust calls through trait objects and function pointers, and any call inside a
macro's arguments, which rust-analyzer does not report; in Go calls through interfaces and
function values. Run the program and ask again: the calls its stacks showed join the trees.

tuitor runs intelephense (PHP), typescript-language-server (TypeScript, JavaScript, Angular),
rust-analyzer (Rust) and gopls (Go) from `PATH`, or as the `[lsp]` table of `config.toml` says
([configuration](configuration.md#configtoml)), once per project when first asked, and ends
them with the tab; PATHS shows what a server reports while it reads the project. intelephense
answers no call hierarchy, so for PHP the callers are its references that call, and the calls
are the definitions of what the text calls; neither needs an intelephense licence. Other
language servers are welcome as contributions: the `tuitor-code` crate is the only one that
speaks LSP.
