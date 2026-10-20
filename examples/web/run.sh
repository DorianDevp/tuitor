#!/usr/bin/env bash
# Debugs the web example in tuitor: js-debug opens index.html in a Chromium of its own.
# HEADLESS=1 runs that Chromium without a window. Arguments go to tuitor
# (./run.sh --script demo.tsc). TUITOR=/path/to/tuitor uses that binary instead of
# building tuitor from this repository.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
install_help="see \"Installing adapters\" in $repo/README.md"
adapter="${XDG_DATA_HOME:-$HOME/.local/share}/tuitor/adapters/js-debug"
js_debug_version=1.140.0 # the release README.md installs
config="orders"
if [[ -n ${HEADLESS:-} ]]; then
    config="orders (headless)"
fi

step() { echo "==> $*"; }

die() {
    echo "run.sh: $1" >&2
    shift
    for line in "$@"; do echo "    $line" >&2; done
    exit 1
}

# 1. Prerequisites.
step "checking Chromium and Node.js"
# The browser of the first configuration, "orders".
browser=$(grep -m 1 -o '"runtimeExecutable": *"[^"]*"' "$here/.vscode/launch.json") ||
    die "No runtimeExecutable in .vscode/launch.json."
browser=${browser%\"}
browser=${browser##*\"}
[[ -x $browser ]] || die "$browser, the browser .vscode/launch.json starts, is not installed." \
    "Install Chromium (pacman -S chromium, apt-get install chromium, dnf install chromium)," \
    "or set runtimeExecutable in .vscode/launch.json to your Chromium or Chrome."
command -v node >/dev/null || die "Node.js is not installed; js-debug runs on it." \
    "Install it: https://nodejs.org/en/download or your distribution's nodejs; $install_help."
if [[ -n ${TUITOR:-} ]]; then
    tuitor=$(command -v "$TUITOR") || die "TUITOR=$TUITOR is not an executable."
else
    command -v cargo >/dev/null || die "cargo is not installed; it builds tuitor from this repository." \
        "Install Rust from https://rustup.rs, or point TUITOR at a tuitor binary: TUITOR=/path/to/tuitor ./run.sh"
fi

# 2. The example: index.html and orders.js need no build, and Chromium opens them from disk.

# 3. tuitor: built from this repository so it matches the examples. The debug build is the
# one `cargo build` and `cargo test` already keep up to date; release would be a second,
# much slower build.
if [[ -z ${TUITOR:-} ]]; then
    step "building tuitor (cargo build -p tuitor)"
    cargo build -p tuitor --manifest-path "$repo/Cargo.toml"
    tuitor="${CARGO_TARGET_DIR:-$repo/target}/debug/tuitor"
fi

# js-debug, which tuitor installs from its GitHub release as README.md ("Installing
# adapters") does, once you say so.
if [[ ! -f $adapter/js-debug/src/dapDebugServer.js ]]; then
    step "installing js-debug $js_debug_version"
    "$tuitor" install js-debug --version "$js_debug_version" ||
        die "js-debug is not installed." "To install it yourself, $install_help; then run ./run.sh again."
fi

# 4. Debug. However tuitor ends, by Q or by SIGINT, SIGTERM or SIGHUP, it ends js-debug and
# Chromium and puts the terminal back.
step "starting tuitor on configuration \"$config\" (Q quits and closes Chromium)"
exec "$tuitor" -C "$here" "$config" "$@"
