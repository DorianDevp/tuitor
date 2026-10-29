# tuitor vs the VS Code debugger

tuitor uses the same adapters as VS Code (js-debug, vscode-php-debug, Delve, CodeLLDB) and reads
the same `.vscode/launch.json`.

| | tuitor | VS Code |
|---|---|---|
| Needs | a terminal | a desktop (or Remote extensions) |
| Size | ~9 MB binary | ~1 GB unpacked |
| Over SSH, in tmux, in a container | yes | via Remote Development |
| `preLaunchTask`, `integratedTerminal` | no | yes |
| Threads, data breakpoints, memory view, set a variable | no | yes (adapter permitting) |
| Recorded stops, step back through them | yes | no |
| Breakpoints that record the stack and variables without stopping | yes | no (logpoints log text) |
| COMPARE two runs, across commits | yes | no |
| Dockerfile build debugging | yes, with failure explanations | via Docker's extension |
| Editor | any ([Editors](../editors.md)) | VS Code |

**Choose VS Code** if you already work in it and need threads, memory or tasks.

**Choose tuitor** to debug from a terminal or a remote machine, or to keep and compare what each
stop saw. Your `launch.json` works in both, so you can switch per task.
