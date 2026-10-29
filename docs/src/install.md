# Install

## tuitor

| from | command |
|---|---|
| crates.io | `cargo install tuitor --locked` |
| prebuilt binary | `cargo binstall tuitor`, or download from [GitHub Releases](https://github.com/DorianDevp/tuitor/releases) |
| Homebrew | `brew install DorianDevp/tuitor/tuitor` |
| Arch Linux (AUR) | `yay -S tuitor-bin` (binary) or `yay -S tuitor` (from source) |
| Nix | `nix run github:DorianDevp/tuitor` |
| source | `cargo install --path crates/tuitor` in a clone (stable Rust) |

## Adapters

tuitor runs adapters; it does not bundle them. Check what you have:

```sh
tuitor doctor            # every adapter: found, missing, and the command that fixes it
tuitor install <adapter> # download an adapter's official release (asks first)
```

| language | adapter | install |
|---|---|---|
| PHP | vscode-php-debug 1.40.2 (Node.js 20.18+) + Xdebug 3 | `tuitor install php`; Xdebug from your distribution or `pie install xdebug/xdebug` |
| JavaScript, TypeScript (Node.js, Chrome) | js-debug 1.140.0 | `tuitor install js-debug` |
| Go | Delve | `tuitor install delve` (runs `go install`) |
| C, C++, Rust, Zig | gdb 14+ built with Python | distribution package |
| | lldb-dap | distribution package (LLVM) |
| | CodeLLDB 1.12.3 | `tuitor install codelldb` |
| Dockerfile | BuildKit, `docker buildx` 0.38+ | Docker |

For gdb, LLDB and Xdebug, `tuitor install` names your distribution's package (Arch, Debian,
Ubuntu, Fedora). `--version` picks another release. Manual steps: [Adapters](adapters.md#installing-by-hand).

![DOCTOR: every adapter, lldb-dap missing with the package that provides it](assets/screenshots/doctor.png)
