# Contributing

Thanks for helping. Bug reports, new adapters and language servers are especially welcome.

## Build and test

```sh
cargo build
cargo test --workspace                          # unit tests and a real gdb session
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

Tests against other real adapters are opt-in through environment variables:

```sh
TUITOR_PHP=/path/to/php-with-xdebug \
TUITOR_PHP_ADAPTER=~/.local/share/tuitor/adapters/vscode-php-debug/extension/out/phpDebug.js \
  cargo test -p tuitor-core --test php -- --ignored
TUITOR_JS_ADAPTER=~/.local/share/tuitor/adapters/js-debug/js-debug/src/dapDebugServer.js \
  cargo test -p tuitor-core --test js -- --ignored
TUITOR_DLV="$(go env GOPATH)/bin/dlv" cargo test -p tuitor-core --test go -- --ignored
TUITOR_GOPLS=gopls TUITOR_RUST_ANALYZER=rust-analyzer TUITOR_INTELEPHENSE=intelephense \
TUITOR_TYPESCRIPT_LANGUAGE_SERVER=typescript-language-server \
  cargo test -p tuitor-code --test servers -- --ignored
```

Try a change by hand with an example: `cd examples/go && ./run.sh`.

## Where things are

| crate | |
|---|---|
| `tuitor-dap` | DAP wire format and async client |
| `tuitor-core` | engine: sessions, breakpoints, snapshots, flows, doctor, install |
| `tuitor-code` | language servers (the only crate that speaks LSP) |
| `tuitor` | the terminal UI |

Every user action is a command (`crates/tuitor/src/command.rs`); keys are bindings in
`crates/tuitor/src/default_keys.toml`; actions are applied in `App::apply`.

## Add an adapter

1. Add a built-in entry in `Adapters::default()` in `crates/tuitor-core/src/config.rs`: command,
   transport, file extensions.
2. If it can be downloaded, add it to `crates/tuitor-core/src/install.rs`; add its checks to
   `crates/tuitor-core/src/doctor.rs`.
3. Add an example under `examples/` with a `.vscode/launch.json`, a `run.sh` and a short README.
4. Add an end-to-end test in `crates/tuitor-core/tests/`, ignored unless its environment variable is set.
5. Document it in `docs/src/adapters.md` and a guide in `docs/src/guides/`.

## Add a language server

`crates/tuitor-code/src/config.rs` lists the languages and their servers. Add the language there, then
a test in `crates/tuitor-code/tests/servers.rs`.

## Screenshots and demos

- Screenshots: `docs/screenshots/render.sh` runs scripted sessions (`tuitor --script`) on the examples
  and renders them to `assets/screenshots/`.
- Demo videos: [VHS](https://github.com/charmbracelet/vhs) tapes in `docs/tapes/`, recorded to
  `assets/demo/`.

CI re-renders both when the views they show change. Run them locally only if you need to.

## Docs

The book is in `docs/src/` ([mdBook](https://rust-lang.github.io/mdBook/)):

```sh
mdbook serve docs
```

Keep pages short: commands first, then one or two sentences per feature.

## Commits and pull requests

- One change per commit. The subject is one full sentence that says what changes and why, e.g.
  *"End an adapter with its whole process group, so that a child it started does not live on holding
  a build."*
- Run fmt, clippy and the tests before opening a pull request.
- Update the docs and `CHANGELOG.md` (under Unreleased) for user-visible changes.

By contributing you agree that your work is licensed under the [MIT License](LICENSE). Please follow
the [Code of Conduct](CODE_OF_CONDUCT.md).
