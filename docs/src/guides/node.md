# Debug Node.js and TypeScript in the terminal

tuitor uses js-debug, the JavaScript debugger built into VS Code.

```sh
tuitor install js-debug
tuitor                      # in a project with .vscode/launch.json
```

A minimal `.vscode/launch.json`:

```jsonc
{
  "version": "0.2.0",
  "configurations": [
    { "name": "orders", "type": "node", "request": "launch", "program": "${workspaceFolder}/orders.js" }
  ]
}
```

- **TypeScript:** compile with source maps (`tsc` with `"sourceMap": true`). Breakpoints, stops and
  stepping then show the `.ts` files.
- **Output** shows in OUTPUT. Use `"console": "internalConsole"` (the default);
  `"integratedTerminal"` is not supported.
- **Child processes:** js-debug runs each Node process in a session of its own. tuitor shows them as
  part of the session that started them. `X` stops the whole program.
- **Library frames** (`node_modules`) fold into one line in STACK; `v` unfolds. `J` skips library
  code when stepping.

## Try it

```sh
cd examples/js && ./run.sh            # JavaScript
cd examples/typescript && ./run.sh    # TypeScript via tsc's source map
```

`preLaunchTask` is not run: build first. Hit counts use js-debug's syntax: `>= 3`.
