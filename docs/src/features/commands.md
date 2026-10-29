# Commands and DOCTOR

Every key runs a command line, so keys, the command line and scripts do the same thing.

| key | does |
|---|---|
| `/` or `:` | command line: matches as you type, `tab` completes, shows arguments and history |
| `?` | every command with its key |
| `:42` | go to line 42 of SOURCE |

![The command line with "set" typed](../assets/screenshots/palette.png)

## Handy commands

| key | command | does |
|---|---|---|
| `v` | `/inspect` | the selected value in full; JSON laid out; `y` copies it |
| `F` | `/filter ‹text›` | filter OUTPUT lines or LOCALS variables |
| `J` | `/my code` | stepping skips library code (`vendor`, `node_modules`) |
| `f` | `/find` | find files and breakpoints (`name:line`) |
| `ctrl-f`, `/` in SOURCE | `/search` | search the file; `ctrl-n` `ctrl-p` next, previous |
| `E` | `/edit` | open the line in your editor ([Editors](../editors.md)) |
| | `/show file:line` | show that place in CODE |
| `enter` in OUTPUT/EVENTS | | open the `file:line` it names (PHP, Node, Rust, Go, C formats) |
| `ctrl-z` | `/suspend` | back to the shell; `fg` returns, the program keeps running |

A `file:line` argument can be the end of a path: `/bp toggle Event/LoyaltyListener.php:16`.

`tuitor send ‹command line›` runs a command in the running tuitor, e.g. from an editor key binding
(`tuitor send bp toggle src/main.rs:12`). See [Editors](../editors.md).

## DOCTOR

`tuitor doctor` (or `!` inside tuitor) checks every adapter and what each configuration needs: Node.js
version, gdb's DAP support, Delve, a browser, a free port for Xdebug, Xdebug in your PHP. Anything
missing comes with the command that fixes it; tuitor runs nothing without asking.

```sh
tuitor doctor              # exit code 1 when something the project needs is missing
tuitor doctor php gdb      # only these adapters
```

![DOCTOR: every adapter and each configuration's checks](../assets/screenshots/doctor.png)
