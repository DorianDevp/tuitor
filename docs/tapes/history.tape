# HISTORY on the C example through gdb: a breakpoint in order_total's loop hit five times, every
# variable at every stop, TIMELINE of what changed, and a recorded stop opened again.
# From the repository's root: vhs docs/tapes/history.tape (TUITOR=/path/to/tuitor).
Source docs/tapes/settings.tape
Output assets/demo/history.webm
Output assets/demo/history.mp4
Require gdb
Require make

Hide
Type "source docs/tapes/home.sh && cd c && clear" Enter
Sleep 500ms
Type "./run.sh" Enter
Wait+Screen@120s /stop #1/
Show
Sleep 1.5s

# A breakpoint on the line that adds each line of an order.
Type ":"
Sleep 500ms
Type "bp toggle orders.c:48"
Sleep 700ms
Enter
Sleep 800ms
Type "c"
Wait+Screen@30s /stop #2/
Sleep 1s
Type "c"
Wait+Screen@30s /stop #3/
Sleep 1s
Type "c"
Wait+Screen@30s /stop #4/
Sleep 1s
Type "c"
Wait+Screen@30s /stop #5/
Sleep 1s
Type "c"
Wait+Screen@30s /stop #6/
Sleep 1.5s

# Every variable at every stop; k goes back a stop, showing what was recorded there.
Type "gh"
Sleep 2.5s
Type "k"
Sleep 900ms
Type "k"
Sleep 2s
Screenshot assets/demo/history.png

# TIMELINE: a value, and its next change.
Tab
Sleep 900ms
Type "j"
Sleep 1.2s
Type "l"
Sleep 2s

# The stop on view, opened in OVERVIEW as it was recorded.
Enter
Sleep 3s

Hide
Type "Q"
Sleep 1s
