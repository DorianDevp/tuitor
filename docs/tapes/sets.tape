# Breakpoint sets on the C example through gdb: lavender stops in main's loop, cyan only
# records in order_total and lets the program go on; HISTORY marks those stops ↷.
# From the repository's root: vhs docs/tapes/sets.tape (TUITOR=/path/to/tuitor).
Source docs/tapes/settings.tape
Output assets/demo/sets.webm
Output assets/demo/sets.mp4
Require gdb
Require make

Hide
Type "source docs/tapes/home.sh && cd c && clear" Enter
Sleep 500ms
Type "./run.sh" Enter
Wait+Screen@120s /stop #1/
Show
Sleep 1.5s

# The first set, lavender: a breakpoint where each order's total is added.
Type ":"
Sleep 500ms
Type "bp toggle orders.c:69"
Sleep 700ms
Enter
Sleep 1s

# N makes a second set, cyan, and makes it the active one: its breakpoint goes in it.
Type "N"
Sleep 1.2s
Type ":"
Sleep 500ms
Type "bp toggle orders.c:48"
Sleep 700ms
Enter
Sleep 1.2s

# A makes lavender active again: cyan's breakpoint now records and the program goes on.
Type "A"
Sleep 2s
Type "c"
Wait+Screen@30s /stop #5/
Sleep 2.5s
Screenshot assets/demo/sets.png
Type "c"
Wait+Screen@30s /stop #8/
Sleep 2s

# HISTORY: ● where the program waited, ↷ where cyan recorded and it went on.
Type "gh"
Sleep 3.5s

# The sets, to pick the active one.
Type "ga"
Sleep 2.5s
Escape
Sleep 1s

Hide
Type "Q"
Sleep 1s
