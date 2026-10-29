# Limitations

- **No terminal for the program.** An adapter's `runInTerminal` request (as
  `"console": "integratedTerminal"` sends) is refused; use `"internalConsole"`. BuildKit's `/shell`
  is the exception.
- **`preLaunchTask` is not run.** Start a dev server or build yourself.
- **One thread.** STACK shows the thread that stopped; there is no thread list.
- **No** function or data breakpoints, disassembly, memory view, or setting a variable.
- **PATHS** covers PHP, TypeScript/JavaScript, Rust and Go (four language servers); not C/C++. It sees
  only what the server sees (see [PATHS](sets.md#paths)).
- **Terminal:** needs truecolour and Unicode symbols; otherwise colours are approximate and glyphs
  missing.
- **Platforms:** developed and tested on Linux; macOS and Windows are untested.
