# Debug Go with Delve in the terminal

```sh
tuitor install delve        # runs go install github.com/go-delve/delve/cmd/dlv@latest
tuitor                      # in a module: tuitor creates a Delve configuration
```

Or in `.vscode/launch.json`, as VS Code's Go extension writes it:

```jsonc
{
  "name": "Launch Package",
  "type": "go",
  "request": "launch",
  "mode": "debug",                 // "test" runs the package's tests
  "program": "${workspaceFolder}"
}
```

- Delve builds without optimisations, so every variable is visible.
- `"mode": "auto"` works: `test` for a `_test.go` file, `debug` otherwise.
- `go install` puts `dlv` in `$(go env GOPATH)/bin`, which must be on `PATH`. `tuitor doctor` says
  so when it isn't.
- Put the first breakpoint on `func main() {`. `stopOnEntry` stops before Go starts, where there is
  no stack yet.
- `${fileDirname}` means nothing outside VS Code: name the directory.

![PROJECT in the Go example: a condition, a hit count and a logpoint](../assets/screenshots/project.png)

## Try it

```sh
cd examples/go && ./run.sh
```

Then `f`, `main.go:58`, `ctrl-b`, `esc`, `c`.
