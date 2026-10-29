# tuitor vs gdb's TUI and gdb-dashboard

gdb's TUI (`gdb -tui`) and [gdb-dashboard](https://github.com/cyrus-and/gdb-dashboard) add panes to
gdb's own command line. tuitor drives gdb through its DAP server (gdb 14+), and other debuggers too.

| | tuitor | gdb TUI / gdb-dashboard |
|---|---|---|
| Languages | C, C++, Rust, Zig, plus PHP, JS/TS, Go via other adapters | what gdb debugs |
| Interface | panes and keys; `launch.json` configurations | gdb commands |
| Changed values highlighted with the previous value | yes | dashboard: yes; TUI: no |
| Recorded stops, HISTORY, TIMELINE, COMPARE | yes | no |
| Reverse execution (`record`, `reverse-step`) | no | yes |
| Watchpoints, disassembly, memory, registers | no | yes |
| Full gdb command line, Python scripting | no (`x` evaluates expressions) | yes |

**Choose gdb directly** for low-level work: watchpoints, disassembly, memory, reverse execution.

**Choose tuitor** for application debugging across languages with one interface, and to see how
values changed over a run without re-running it.
