# Keys

`?` lists every command with its key. Rebind any key in `~/.config/tuitor/keys.toml`
([defaults](https://github.com/DorianDevp/tuitor/blob/master/crates/tuitor/src/default_keys.toml)).

## Run

| key | does | key | does |
|---|---|---|---|
| `c` | continue | `p` | pause |
| `n` | step over | `X` | stop |
| `s` | step in | `R` | restart |
| `o` | step out | `S` | start the configuration selected in CONFIG |
| `[` `]` | older / newer recorded stop | `}` | back to live |
| `J` | my code only, on/off | `.` | SOURCE follows execution again |

## Breakpoints and values

| key | does | key | does |
|---|---|---|---|
| `b` | toggle a breakpoint | `w` | watch |
| `N` | new breakpoint set | `x` | evaluate |
| `A` | next set active | `v` | value in full |
| `g a` | list sets | `y` | copy (OSC 52) |
| `M` | breakpoint in/out of the active set | `F` | filter OUTPUT or LOCALS |
| `P` | paths through the SOURCE line | | |

## Views and navigation

| key | does | key | does |
|---|---|---|---|
| `g o` | OVERVIEW | `tab` | next pane |
| `g c` | CODE | `z` | zoom the pane |
| `g h` | HISTORY | `j` `k` `h` `l`, arrows | move, collapse, expand |
| `g f` | FLOW | `shift-down` `shift-up` | scroll SOURCE a page |
| `g p` | PROJECT | `:42` | line 42 of SOURCE |
| `g d` | COMPARE | `f` | find files and breakpoints |
| `g r` | REBASE | `ctrl-f`, `/` in SOURCE | search SOURCE |
| `E` | open in your editor | `enter` | expand, open, start |

## Tabs, sessions, app

| key | does | key | does |
|---|---|---|---|
| `<` `>` | previous / next tab | `/` `:` | command line |
| `O` `T` | open a project, new tab | `?` | all commands |
| `ctrl-w` | close tab | `!` | doctor |
| `g s` `g S` | next / previous session | `q` `esc` | back out of a popup, zoom, history |
| `ctrl-z` | back to the shell, until `fg` | `Q` `ctrl-c` | quit |

## In a pane

| pane | keys |
|---|---|
| BREAKPOINTS | `a` session/all, `e` condition, `h` hit count, `l` log, `space` on/off, `d` delete, `D` delete its set |
| CONFIG | `a` add, `m` map a path, `M` mappings from compose, `u` unset, `d` delete, `i` install the adapter |
| STACK, FLOW | `v` library frames or files |
| WATCH | `d` delete |
| OUTPUT | `C` clear |
| SOURCE | `/` search, `ctrl-n` `ctrl-p` next/previous match, `q` clear the search |
| COMPARE | `[` `]` older, newer run |
| REBASE | `enter` read / open, `h` `l` fold, `b` a set of its decisions |
| PATHS | `m` mark, `space` stage, `b` branch, `u` unmark, `[` `]` marks, `h` `l` fold, `enter` make sets |
