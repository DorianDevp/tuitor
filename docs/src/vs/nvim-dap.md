# tuitor vs nvim-dap

[nvim-dap](https://github.com/mfussenegger/nvim-dap) is a DAP client inside Neovim, usually with
nvim-dap-ui for panes. tuitor is a standalone DAP client that runs next to any editor.

| | tuitor | nvim-dap (+ nvim-dap-ui) |
|---|---|---|
| Runs in | its own terminal (pane, tab, tmux split) | Neovim |
| Setup | `.vscode/launch.json` or a default per project; `tuitor install` fetches adapters | Lua config per adapter; can load `launch.json` |
| Adapters | PHP, JS/TS, Go, C/C++/Rust/Zig, Dockerfile built in; others via `adapters.toml` | any DAP adapter, large plugin ecosystem (e.g. Python via nvim-dap-python) |
| Editing while debugging | in your editor; `E` opens the line, Neovim is detected | in the same Neovim |
| Recorded stops, step back through them | yes | no |
| History of every variable, TIMELINE | yes | no |
| Breakpoint sets that record without stopping, COMPARE of runs | yes | no |
| Program in its own terminal (`runInTerminal`) | no | yes |
| Thread list | no (the stopped thread) | yes |

**Choose nvim-dap** if you live in Neovim and want debugging inside it, or need an adapter
tuitor doesn't ship (Python, Java, .NET).

**Choose tuitor** if you want recorded stops and run comparison, use another editor, or want one
debugger that works the same in every project. Both can be used side by side: run tuitor in a Neovim
terminal and `E` opens files in that Neovim.
