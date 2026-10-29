# tuitor

**A terminal debugger for every language: it records every stop, so you can step back and compare runs.**

<video autoplay loop muted playsinline poster="assets/demo/overview.png" width="100%">
  <source src="assets/demo/overview.webm" type="video/webm">
  <source src="assets/demo/overview.mp4" type="video/mp4">
  <img src="assets/screenshots/overview.png" alt="tuitor stopped at a breakpoint">
</video>

tuitor is a debugger client for the [Debug Adapter Protocol](https://microsoft.github.io/debug-adapter-protocol/)
(DAP). One terminal program debugs PHP, JavaScript, TypeScript, Go, C, C++, Rust and Zig, using
the same adapters VS Code uses.

- **Uses your `.vscode/launch.json`.** No new config format.
- **Records every stop.** Step back through stops, see every variable at every stop, find when a
  value went wrong.
- **Breakpoint sets.** One set stops the program; the others record and let it run.
  [COMPARE](sets.md#compare) lines up two runs, also across commits.
- **Debugs Dockerfile builds** and says why a step failed.
- **Works with your editor.** Open the current line in it, set breakpoints from it. See
  [Editors](editors.md).
- **One ~9 MB binary.** No GUI, works over SSH and in tmux.

## Who it is for

- Developers who work in a terminal editor (Neovim, Vim, Helix, Emacs, Kakoune) and want a full
  debugger next to it.
- Anyone debugging on a remote machine or in a container over SSH.
- Anyone who wants to see *when* a value changed, not only what it is now.

## Size

Measured on 2026-10-20, x86_64 Linux, in MB:

| | MB |
|---|---:|
| tuitor 0.1.0, release build, stripped | 8.9 |
| vscode-php-debug 1.40.2 + js-debug 1.140.0 (the adapters) | 8.4 |
| VS Code 1.142.0, unpacked | 1016.5 |

Next: [Install](install.md), then the [Quick start](quick-start.md).
