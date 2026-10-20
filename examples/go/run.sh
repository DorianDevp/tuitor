#!/usr/bin/env bash
# Debugs the Go example in tuitor through Delve, which builds it when the session starts.
# Arguments go to tuitor (./run.sh --script demo.tsc). TUITOR=/path/to/tuitor uses that
# binary instead of building tuitor from this repository.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
install_help="see \"Installing adapters\" in $repo/README.md"

step() { echo "==> $*"; }

die() {
    echo "run.sh: $1" >&2
    shift
    for line in "$@"; do echo "    $line" >&2; done
    exit 1
}

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

# 4. Debug. However tuitor ends, by Q or by SIGINT, SIGTERM or SIGHUP, it ends dlv and the
# program and puts the terminal back.
step "starting tuitor on configuration \"orders\" (Q quits)"
exec "$tuitor" -C "$here" orders "$@"
