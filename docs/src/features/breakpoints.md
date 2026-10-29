# Breakpoints and configurations

PROJECT (`g p`) shows BREAKPOINTS and CONFIG side by side.

![PROJECT in the Go example: a condition, a hit count and a logpoint](../assets/screenshots/project.png)

## Breakpoints

| key | does |
|---|---|
| `b` | toggle on the SOURCE line |
| `e` | condition |
| `h` | hit count, in the adapter's syntax (`>= 3` js-debug, `% 2` PHP, `> 2` Delve, hits to skip for gdb) |
| `l` | log a message instead of stopping (`{expr}` is interpolated) |
| `space` | on/off |
| `a` | this session's or every tab's |

| shape | means |
|---|---|
| `●` | stops |
| `◐` | condition or hit count |
| `◆` | logs |
| `○` | disabled |
| `◌` | not accepted by the adapter |
| `⊘` | its line is gone |

Breakpoints and watches are saved per project in `~/.local/state/tuitor/`, outside the repository.

## Breakpoints follow their lines

When a file changes on disk (an editor save, checkout, pull or rebase), each breakpoint finds its
line again by its text and the lines around it. If the line is gone, it shows `⊘` and goes to no
adapter until the line comes back.

`/bp export [file]` and `/bp import [file]` share breakpoints with a colleague or another branch;
imported ones land in a new set and find their lines the same way.

## What the tests reach

tuitor tells test runs from program runs by their stacks (a frame in `*Test.php`, `*.spec.ts`,
`*_test.go`, `tests/`). BREAKPOINTS shows which runs reached each breakpoint, such as
`program 3 · tests: MoneyTest->testBulkRebate`. `/coverage` lists breakpoints the program reaches and
no test does.

## Configurations

CONFIG lists `.vscode/launch.json` (read-only) and tuitor's own configurations
(`~/.config/tuitor/projects.toml`). `enter` starts one. Commands: [Configuration](../configuration.md).
