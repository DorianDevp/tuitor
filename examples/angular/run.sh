#!/usr/bin/env bash
# Debugs the Angular example in tuitor: ng serve serves it, and js-debug opens it in a
# Chromium of its own. HEADLESS=1 runs that Chromium without a window. Arguments go to
# tuitor (./run.sh --script demo.tsc). TUITOR=/path/to/tuitor uses that binary instead of
# building tuitor from this repository.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
install_help="see \"Installing adapters\" in $repo/README.md"
adapter="${XDG_DATA_HOME:-$HOME/.local/share}/tuitor/adapters/js-debug"
js_debug_version=1.140.0 # the release README.md installs
log="$here/ng-serve.log"
config="ng serve"
if [[ -n ${HEADLESS:-} ]]; then
    config="ng serve (headless)"
fi

step() { echo "==> $*"; }

die() {
    echo "run.sh: $1" >&2
    shift
    for line in "$@"; do echo "    $line" >&2; done
    exit 1
}

# 1. Prerequisites.
step "checking Chromium, Node.js, npm, curl and js-debug"
# The browser of the first configuration, "ng serve".
browser=$(grep -m 1 -o '"runtimeExecutable": *"[^"]*"' "$here/.vscode/launch.json") ||
    die "No runtimeExecutable in .vscode/launch.json."
browser=${browser%\"}
browser=${browser##*\"}
[[ -x $browser ]] || die "$browser, the browser .vscode/launch.json starts, is not installed." \
    "Install Chromium (pacman -S chromium, apt-get install chromium, dnf install chromium)," \
    "or set runtimeExecutable in .vscode/launch.json to your Chromium or Chrome."
command -v node >/dev/null || die "Node.js is not installed; it runs ng serve and js-debug." \
    "Install it: https://nodejs.org/en/download or your distribution's nodejs; $install_help."
command -v npm >/dev/null || die "npm is not installed; it installs Angular for ng serve." \
    "It comes with Node.js from https://nodejs.org/en/download; some distributions package it" \
    "separately (pacman -S npm, apt-get install npm)."
command -v curl >/dev/null || die "curl is not installed; run.sh uses it to see when ng serve serves the page." \
    "Install it, e.g. pacman -S curl or apt-get install curl."
if [[ ! -f $adapter/js-debug/src/dapDebugServer.js ]]; then
    url="https://github.com/microsoft/vscode-js-debug/releases/download/v$js_debug_version/js-debug-dap-v$js_debug_version.tar.gz"
    echo "js-debug is not installed in $adapter."
    echo "README.md (\"Installing adapters\") installs it there from its GitHub release:"
    echo "    $url"
    read -r -p "Download and unpack it now? [y/N] " answer || answer=
    [[ $answer == [yY]* ]] || die "js-debug is not installed; nothing was changed." \
        "To install it yourself, $install_help; then run ./run.sh again."
    step "installing js-debug $js_debug_version"
    # The commands from README.md, "Installing adapters".
    rm -rf "$adapter" && mkdir -p "$adapter"
    curl -fL -o "$adapter/js-debug-dap.tar.gz" "$url"
    tar -xzf "$adapter/js-debug-dap.tar.gz" -C "$adapter" && rm "$adapter/js-debug-dap.tar.gz"
fi
# The page of the first configuration, which ng serve is to serve.
url=$(grep -m 1 -o '"url": *"[^"]*"' "$here/.vscode/launch.json") || die "No url in .vscode/launch.json."
url=${url%\"}
url=${url##*\"}
port=${url##*:}
port=${port%%/*}
# Another server there would be debugged instead, and ng serve would not start.
! curl -s -o /dev/null "$url" || die "Something serves $url already, so ng serve cannot." \
    "Is another ng serve running? Stop it, then run ./run.sh again."
if [[ -n ${TUITOR:-} ]]; then
    tuitor=$(command -v "$TUITOR") || die "TUITOR=$TUITOR is not an executable."
else
    command -v cargo >/dev/null || die "cargo is not installed; it builds tuitor from this repository." \
        "Install Rust from https://rustup.rs, or point TUITOR at a tuitor binary: TUITOR=/path/to/tuitor ./run.sh"
fi

# 2. The example: Angular from package-lock.json into node_modules, again when the lock
# file changed since (npm ci leaves node_modules/.package-lock.json behind).
if [[ ! -f $here/node_modules/.package-lock.json || $here/package-lock.json -nt $here/node_modules/.package-lock.json ]]; then
    step "installing Angular into node_modules (npm ci)"
    npm ci --prefix "$here" --no-audit --no-fund
fi

# 3. tuitor: built from this repository so it matches the examples. The debug build is the
# one `cargo build` and `cargo test` already keep up to date; release would be a second,
# much slower build.
if [[ -z ${TUITOR:-} ]]; then
    step "building tuitor (cargo build -p tuitor)"
    cargo build -p tuitor --manifest-path "$repo/Cargo.toml"
    tuitor="${CARGO_TARGET_DIR:-$repo/target}/debug/tuitor"
fi

# 4. ng serve, in the background; tuitor does not run VS Code's preLaunchTask. It reads
# nothing, so that every key goes to tuitor. A watcher stops it, with the SIGINT of ctrl-c,
# once this script has ended: after the exec below, once tuitor has.
step "starting ng serve on port $port (its output goes to $log)"
(cd "$here" && exec node_modules/.bin/ng serve --port "$port") </dev/null >"$log" 2>&1 &
serve_pid=$!
script_pid=$$ # tuitor's, after the exec below
(
    while kill -0 "$script_pid" 2>/dev/null; do sleep 0.5; done
    kill -INT "$serve_pid" 2>/dev/null
) </dev/null >/dev/null 2>&1 &
until curl -s -o /dev/null "$url"; do
    if ! kill -0 "$serve_pid" 2>/dev/null; then
        tail -n 20 "$log" >&2
        die "ng serve stopped; above is the end of $log."
    fi
    sleep 0.3
done

# 5. Debug. However tuitor ends, by Q or by SIGINT, SIGTERM or SIGHUP, it ends js-debug and
# Chromium and puts the terminal back; the watcher then stops ng serve.
step "starting tuitor on configuration \"$config\" (Q quits, closes Chromium and stops ng serve)"
exec "$tuitor" -C "$here" "$config" "$@"
