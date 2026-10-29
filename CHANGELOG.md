# Changelog

All notable changes to tuitor. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `tuitor send ‹command line›` runs a command in the running tuitor, so editors can toggle
  breakpoints and show lines in it; ready-made bindings in `editors/` for Vim, Neovim, Emacs, Helix,
  VS Code, Zed, Kakoune and Sublime Text, and a JetBrains recipe.
- `/show file:line` shows a place in CODE.
- `E` detects the editor whose terminal tuitor runs in: Vim, Kakoune, Emacs, Zed, VS Code and its
  forks, besides Neovim. `editor` in `config.toml` can be just an editor's name.
- Documentation site built with mdBook, with guides per language, comparisons and an FAQ.
- Contributing guide, code of conduct, security policy, and issue and pull request templates.

## [0.1.0]

First version.

### Added

- DAP client with a terminal UI: OVERVIEW, CODE, HISTORY, FLOW, PROJECT, COMPARE and REBASE views.
- Built-in adapters: vscode-php-debug with Xdebug, js-debug (Node.js, Chrome), Delve, gdb, lldb-dap,
  CodeLLDB, and BuildKit for Dockerfile builds (`tuitor build`). Others through `adapters.toml`.
- `.vscode/launch.json` support, tuitor's own configurations, and a default per project kind.
- `tuitor doctor` and `tuitor install` for adapters.
- Project tabs, each with its own engine; js-debug child sessions shown as one session.
- Recorded stops: step back with `[` and `]`, HISTORY of every variable, TIMELINE of changes, the
  time the program ran between stops; runs kept on disk with their commit.
- LOCALS deltas (`▲`, `△`), values inline in SOURCE, watches with trends.
- Breakpoint conditions, hit counts and logpoints; breakpoints that follow their lines when files
  change; export and import.
- Breakpoint sets: the active set stops, the others record and go on; COMPARE of two sets' runs.
- PATHS: call trees through a line from intelephense, typescript-language-server, rust-analyzer or
  gopls, turned into breakpoint sets.
- FLOW: files of a run in execution order, predicted from earlier runs and Xdebug traces.
- REBASE: decisions made by hand in rebases, as breakpoint sets.
- Which runs (program or tests) reached each breakpoint, and `/coverage`.
- Command line with completion, rebindable keys, value popup, filters, "my code only" stepping.
- `E` opens the line in your editor (Neovim terminal, `$VISUAL`/`$EDITOR`, or a configured command).
- Examples for PHP, PHP in Docker, Node.js, TypeScript, a web page, Angular, Go, C and Rust.

[Unreleased]: https://github.com/DorianDevp/tuitor/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/DorianDevp/tuitor/releases/tag/v0.1.0
