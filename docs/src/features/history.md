# Recorded stops: HISTORY and TIMELINE

Every stop is recorded: the stack, the variables read, the watches. You can go back to any of them.

| key | does |
|---|---|
| `[` `]` | older / newer stop (read-only, nothing is asked of the program) |
| `}` | back to the live stop |
| `g h` | HISTORY: every variable at every stop |

<video autoplay loop muted playsinline poster="../assets/demo/history.png" width="100%">
  <source src="../assets/demo/history.webm" type="video/webm">
  <source src="../assets/demo/history.mp4" type="video/mp4">
</video>

## HISTORY

A table of stops × variables, a chart of the value under the cursor, and the full snapshot of the
stop.

![HISTORY of seven stops: values per stop, TIMELINE, a bar chart, a snapshot](../assets/screenshots/history.png)

The `ran` column is how long the program itself ran since the previous stop, without the time the
debugger held it. With breakpoints that only record (see [sets](../sets.md)), that is a rough profile
of each stage, with any adapter.

## TIMELINE

A row per value that changed, a mark at each stop where it changed, in the colour of the breakpoint
set. `●` the program waited, `↷` it went on, `·` no breakpoint.

| key | does |
|---|---|
| `j` `k` | pick a value |
| `h` `l` | its previous / next change, with old and new value |
| `enter` | open that stop in OVERVIEW |

![TIMELINE: two requests' stops, the change of $rows at stop #9](../assets/screenshots/timeline.png)

## Kept across runs

Runs are saved to disk with the commit the code was at (the newest twelve per project). The next
tuitor loads them, so HISTORY, TIMELINE and [COMPARE](../sets.md#compare) work on them too.
