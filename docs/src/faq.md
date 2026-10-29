# FAQ

## How do I debug Python in the terminal with tuitor?

There is no built-in Python adapter yet. You can add debugpy yourself in
`~/.config/tuitor/adapters.toml` (untested; reports welcome):

```toml
[debugpy]
command = "python3"
args = ["-m", "debugpy.adapter"]   # pip install debugpy
extensions = ["py"]
```

Then a `.vscode/launch.json` entry with `"type": "debugpy"`, `"request": "launch"`,
`"program": "${workspaceFolder}/main.py"` and `"console": "internalConsole"`.

## Can tuitor use my VS Code launch.json?

Yes. It reads `.vscode/launch.json` as VS Code does (comments, `${workspaceFolder}`) and never
writes it. Not supported: `preLaunchTask` and `"console": "integratedTerminal"`.

## Does it work over SSH and in tmux?

Yes. It needs only a terminal with truecolour and Unicode. `y` copies through OSC 52, which works over
SSH. Browser debugging runs headless.

## Does it work on macOS or Windows?

It is developed and tested on Linux. macOS is untested. On Windows, use WSL.

## Is it a time-travel debugger?

No. tuitor records what it read at every stop (stack, variables, watches), so you can go back through
stops and compare runs. It does not reverse execution and cannot show a value it did not read.

## How is it different from rr?

[rr](https://rr-project.org/) records a whole native program's execution and replays it, so you can
run backwards to any instruction. tuitor records only the stops, but works with any DAP adapter (PHP,
JS, Go…), keeps runs across sessions and commits, and compares them.

## Which editor does `E` open?

The `editor` command in `config.toml`, else the editor tuitor runs inside, else `$VISUAL` or
`$EDITOR`. See [Editors](editors.md).

## Where are my breakpoints stored?

In `~/.local/state/tuitor/`, per project, outside the repository. Share them with
`/bp export` and `/bp import`.

## How do I see what the adapter says?

`TUITOR_LOG=dap=trace tuitor` logs the protocol to `~/.local/state/tuitor/tuitor.log`.
