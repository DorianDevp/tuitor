#!/usr/bin/env bash
# Debugs the Node example in tuitor through js-debug, which stops on the first line of
# orders.js. Arguments go to tuitor (./run.sh --script demo.tsc). TUITOR=/path/to/tuitor
# uses that binary instead of building tuitor from this repository.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
install_help="see \"Installing adapters\" in $repo/README.md"
adapter="${XDG_DATA_HOME:-$HOME/.local/share}/tuitor/adapters/js-debug"
js_debug_version=1.140.0 # the release README.md installs
tuitor_pid=

step() { echo "==> $*"; }

die() {
    echo "run.sh: $1" >&2
    shift
    for line in "$@"; do echo "    $line" >&2; done
    exit 1
}

# Every process below $1: js-debug and the Node it started.
descendants() {
    local child
    for child in $(pgrep -P "$1"); do
        echo "$child"
        descendants "$child"
    done
}

cleanup() {
    # Interrupted while tuitor runs: stop it and everything below it. Listed first, since
    # a process whose parent was killed can no longer be found from tuitor.
    if [[ -n $tuitor_pid ]] && kill -0 "$tuitor_pid" 2>/dev/null; then
        local pids
        pids=$(descendants "$tuitor_pid")
        # shellcheck disable=SC2086 # one pid per word
        kill $pids "$tuitor_pid" 2>/dev/null || true
        wait "$tuitor_pid" 2>/dev/null || true
    fi
    # tuitor puts the terminal back when it quits; killed, it leaves it raw and on the
    # alternate screen.
    if [[ -t 0 && $(stty -a) == *-icanon* ]]; then
        printf '\e[?1049l\e[?25h'
        stty sane
    fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

# 1. Prerequisites.
step "checking Node.js and js-debug"
command -v node >/dev/null || die "Node.js is not installed; it runs the example and js-debug." \
    "Install it: https://nodejs.org/en/download or your distribution's nodejs; $install_help."
if [[ ! -f $adapter/js-debug/src/dapDebugServer.js ]]; then
    url="https://github.com/microsoft/vscode-js-debug/releases/download/v$js_debug_version/js-debug-dap-v$js_debug_version.tar.gz"
    echo "js-debug is not installed in $adapter."
    echo "README.md (\"Installing adapters\") installs it there from its GitHub release:"
    echo "    $url"
    read -r -p "Download and unpack it now? [y/N] " answer || answer=
    [[ $answer == [yY]* ]] || die "js-debug is not installed; nothing was changed." \
        "To install it yourself, $install_help; then run ./run.sh again."
    command -v curl >/dev/null || die "Installing it needs curl."
    step "installing js-debug $js_debug_version"
    # The commands from README.md, "Installing adapters".
    rm -rf "$adapter" && mkdir -p "$adapter"
    curl -fL -o "$adapter/js-debug-dap.tar.gz" "$url"
    tar -xzf "$adapter/js-debug-dap.tar.gz" -C "$adapter" && rm "$adapter/js-debug-dap.tar.gz"
fi
if [[ -n ${TUITOR:-} ]]; then
    tuitor=$(command -v "$TUITOR") || die "TUITOR=$TUITOR is not an executable."
else
    command -v cargo >/dev/null || die "cargo is not installed; it builds tuitor from this repository." \
        "Install Rust from https://rustup.rs, or point TUITOR at a tuitor binary: TUITOR=/path/to/tuitor ./run.sh"
fi

# 2. The example: orders.js needs no build.

# 3. tuitor: built from this repository so it matches the examples. The debug build is the
# one `cargo build` and `cargo test` already keep up to date; release would be a second,
# much slower build.
if [[ -z ${TUITOR:-} ]]; then
    step "building tuitor (cargo build -p tuitor)"
    cargo build -p tuitor --manifest-path "$repo/Cargo.toml"
    tuitor="${CARGO_TARGET_DIR:-$repo/target}/debug/tuitor"
fi

# 4. Debug. tuitor runs in the background so that a signal to this script reaches the trap
# at once; <&0 keeps the terminal as its input.
step "starting tuitor on configuration \"orders\" (Q quits)"
"$tuitor" -C "$here" orders "$@" <&0 &
tuitor_pid=$!
wait "$tuitor_pid"
