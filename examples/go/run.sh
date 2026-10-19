#!/usr/bin/env bash
# Debugs the Go example in tuitor through Delve, which builds it when the session starts.
# Arguments go to tuitor (./run.sh --script demo.tsc). TUITOR=/path/to/tuitor uses that
# binary instead of building tuitor from this repository.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
install_help="see \"Installing adapters\" in $repo/README.md"
tuitor_pid=

step() { echo "==> $*"; }

die() {
    echo "run.sh: $1" >&2
    shift
    for line in "$@"; do echo "    $line" >&2; done
    exit 1
}

# Every process below $1: dlv and the program it built.
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
        # dlv removes the program it built when its session ends, not when it is killed.
        rm -f "$here"/__debug_bin*
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

# Where `go install` puts programs: GOBIN, which defaults to bin in GOPATH (go help
# install). `go env GOBIN` may print nothing for the default.
go_bin() {
    local bin
    bin=$(go env GOBIN)
    if [[ -z $bin ]]; then
        bin="$(go env GOPATH)"
        bin="${bin%%:*}/bin"
    fi
    echo "$bin"
}

# 1. Prerequisites.
step "checking Go and Delve"
command -v go >/dev/null || die "Go is not installed; Delve builds the example with it." \
    "Install it from https://go.dev/dl or your distribution (pacman -S go, apt-get install golang-go," \
    "dnf install golang); $install_help."
if ! command -v dlv >/dev/null; then
    gobin=$(go_bin)
    if [[ ! -x $gobin/dlv ]]; then
        echo "Delve (dlv) is not installed."
        echo "README.md (\"Installing adapters\") installs it with Go, into $gobin:"
        echo "    go install github.com/go-delve/delve/cmd/dlv@latest"
        read -r -p "Install it now? [y/N] " answer || answer=
        [[ $answer == [yY]* ]] || die "Delve is not installed; nothing was changed." \
            "To install it yourself, $install_help; then run ./run.sh again."
        step "installing Delve (go install)"
        # The command from README.md, "Installing adapters".
        go install github.com/go-delve/delve/cmd/dlv@latest
        [[ -x $gobin/dlv ]] || die "go install did not put dlv in $gobin."
    fi
    # tuitor runs dlv from PATH only. This run gets Go's bin directory; a shell of yours
    # needs it too to debug Go without this script.
    if ! command -v dlv >/dev/null; then
        echo "dlv is in $gobin, which is not on PATH; this run adds it. To debug Go without"
        echo "this script, add it to PATH yourself: export PATH=\"\$PATH:$gobin\""
        export PATH="$gobin:$PATH"
    fi
fi
if [[ -n ${TUITOR:-} ]]; then
    tuitor=$(command -v "$TUITOR") || die "TUITOR=$TUITOR is not an executable."
else
    command -v cargo >/dev/null || die "cargo is not installed; it builds tuitor from this repository." \
        "Install Rust from https://rustup.rs, or point TUITOR at a tuitor binary: TUITOR=/path/to/tuitor ./run.sh"
fi

# 2. The example: dlv builds it without optimisations when the session starts.

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
