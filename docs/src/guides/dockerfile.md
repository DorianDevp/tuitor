# Debug a Dockerfile build

tuitor debugs a build through BuildKit's own debugger (`docker buildx dap build`, buildx 0.38+).

```sh
tuitor build                                   # ./Dockerfile
tuitor build api/Dockerfile --target builder   # a file and a stage
tuitor build --context ./api
```

<video autoplay loop muted playsinline poster="../assets/demo/build.png" width="100%">
  <source src="../assets/demo/build.webm" type="video/webm">
  <source src="../assets/demo/build.mp4" type="video/mp4">
</video>

- Breakpoints go on Dockerfile lines; the build stops there.
- STACK is the step; LOCALS are its arguments and file system.
- `/shell` opens a shell in the container of the step it stopped at.
- A `"type": "dockerfile"` configuration (as Docker's VS Code extension writes it) works too.

## When a step fails

The build stops on the failing line, and tuitor says what failed and why, with its source:

| it tells you | example |
|---|---|
| what the command was at | "at `opcache`, 4 of the 5 it names" |
| what the Dockerfile shows | "`$packages` is set by the RUN at Dockerfile:3, a shell of its own: in this RUN it is empty" |
| what the container shows | `php -m` already lists the extension; where a missing header is; whether apt has a package; a variable's value |

For `pecl install`, it also asks PECL for the package's newest release.
