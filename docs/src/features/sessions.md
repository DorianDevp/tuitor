# Shared sessions

Several tuitors, or a tuitor and a script, debug one program together: the same stops, the same
variables, the same breakpoints. The program moves on only when everyone in the session says yes.

```sh
tuitor --session pair "Listen for Xdebug"     # a terminal in session `pair`
tuitor --session pair --as ana                # another one, under another name
tuitor session pair --as bot join             # a peer on the command line
tuitor session pair --as bot wait stop        # until the program stops, then the stop
```

## How it works

- A session is a directory, `~/.local/state/tuitor/sessions/‹name›/`, with one append-only
  journal. Every tuitor in it appends what it asks for and reads what the others did.
- No tuitor owns the session. The first one that starts a configuration starts the session's
  monitor, `tuitor monitor ‹name›`, which holds the adapters (Xdebug's port, js-debug), does what
  the journal asks, and records what happened. It ends once nothing runs and nobody is in.
- Opened again, a session picks up where it was: its stops, breakpoints, sets, watches and notes
  come back from the journal.

## Votes

Continue, the steps, pause, stop and restart wait until every peer in the session said yes; one
no drops them. Alone in a session, they run at once. Breakpoints, sets, watches, expanding
variables, eval and notes need no vote. A peer that joined with `--su` (`tuitor --session pair --su`)
moves the program on at once, without votes; the others' calls still wait for its yes.

A call that waits shows in a VOTE popup in the middle of every peer's screen until it is decided:

| key | does |
|---|---|
| `y`, `n` | yes or no to another peer's call |
| `w` | withdraw your own call |
| `esc` | hide the popup; the status line still says VOTE, and `/vote` shows it again |

| command | does |
|---|---|
| `/vote yes`, `/vote no` | answer the call that waits for you; `/vote` alone shows its popup |
| `/note ‹text›` | a note every peer sees, at the stop and frame on screen |

## Several debuggers on the same files

Each configuration started is a session of its own, all in one tab and one timeline. `b` sets a
breakpoint for the configuration of the session on screen: a browser debugged twice, once by
hand and once by a test, stops each at its own breakpoints though they are on the same lines.
With no session on screen, `b` sets one for every session.

| command | does |
|---|---|
| `b`, `/bp toggle` | a breakpoint for the session on screen's configuration |
| `/bp for ‹configuration›` | a breakpoint on the SOURCE line for that configuration; without one, for every session |

## On the command line

`tuitor session ‹name› ‹verb›` prints plain text, for scripts and agents. `--as ‹name›` names the
peer.

| verb | does |
|---|---|
| `join`, `leave` | be in the session in between, so that calls wait for this peer's vote |
| `show [--frame n] [--context n]` | the stop: stack with the code around each frame, variables, watches, votes, notes |
| `wait stop\|proposal\|exit` | until that happens, then `show` |
| `do ‹command›` | `continue`, `next`, `step in`, `step out`, `pause`, `stop`, `restart`, `start ‹config›`, `eval ‹expr›`, `expand ‹Scope.var.field›`, `frame ‹n›`, `bp ‹file:line›`, `set new`, `watch ‹expr›` |
| `vote yes\|no`, `note ‹text› [--at stop/frame]` | as in the terminal |
| `history` | the session's stops |
| `paths ‹file:line›` | PATHS as text |
| `compare ‹session›:‹from›-‹to› ‹session›:‹from›-‹to› [--set ‹set›]` | COMPARE as text |
