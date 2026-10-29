# The C example through gdb: a breakpoint in the loop, a watch, LOCALS with what changed since
# the stop before (▲), back through the recorded stops, and HISTORY with its TIMELINE.
# From the repository's root: vhs docs/tapes/overview.tape (TUITOR=/path/to/tuitor).
Source docs/tapes/settings.tape
Output assets/demo/overview.webm
Output assets/demo/overview.mp4
Require gdb
Require make

Hide
Type "source docs/tapes/home.sh && cd c && clear" Enter
Sleep 500ms
Type "./run.sh" Enter
# launch.json stops at the start of main.
Wait+Screen@120s /stop #1/
Show
Sleep 2s

# A breakpoint where each order's total is added, from the command line.
Type ":"
Sleep 500ms
Type "bp toggle orders.c:69"
Sleep 700ms
Enter
Sleep 1s
Type "c"
Wait+Screen@30s /stop #2/
Sleep 1.5s

# A watch on the running total.
Type "w"
Sleep 400ms
Type "revenue"
Sleep 500ms
Enter
Sleep 1s

# Two more orders: LOCALS marks what changed, the watch keeps its trend.
Type "c"
Wait+Screen@30s /stop #3/
Sleep 2s
Type "c"
Wait+Screen@30s /stop #4/
Sleep 2.5s
Screenshot assets/demo/overview.png

# Back through the recorded stops, then live again.
Type "["
Sleep 1.2s
Type "["
Sleep 2s
Type "}"
Sleep 1.2s

# HISTORY: every variable at every stop, and TIMELINE of what changed.
Type "gh"
Sleep 3s
Type "j"
Sleep 600ms
Type "j"
Sleep 2s
Type "go"
Sleep 1.5s

Hide
Type "Q"
Sleep 1s
