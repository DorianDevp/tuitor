#!/usr/bin/env bash
# Builds the C example and debugs it in tuitor, which stops at the start of main.
# Arguments go to tuitor (./run.sh --script demo.tsc). TUITOR=/path/to/tuitor uses
# that binary instead of building tuitor from this repository.
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

# 1. Prerequisites.
step "checking cc, make and gdb"
command -v cc >/dev/null || die "cc (a C compiler) is not installed." \
    "Install one, e.g. pacman -S gcc or apt-get install gcc."
command -v make >/dev/null || die "make is not installed." \
    "Install it, e.g. pacman -S make or apt-get install make."
command -v gdb >/dev/null || die "gdb is not installed." \
    "Install gdb 14 or newer built with Python, e.g. pacman -S gdb or apt-get install gdb;" \
    "$install_help."
gdb_version=$(gdb --version | awk 'NR == 1 { print $NF }')
(( ${gdb_version%%.*} >= 14 )) || die "gdb $gdb_version is too old: tuitor talks to gdb's DAP server, new in gdb 14." \
    "Install gdb 14 or newer built with Python (Debian 13, Ubuntu 24.04 and Arch have one); $install_help."
# Try what tuitor does: start gdb's DAP server and send it an initialize request.
request='{"seq": 1, "type": "request", "command": "initialize", "arguments": {"adapterID": "gdb"}}'
reply=$(printf 'Content-Length: %d\r\n\r\n%s' "${#request}" "$request" |
    timeout 20 gdb --interpreter=dap --quiet 2>&1) || true
[[ $reply == *'"success": true'* ]] || die "gdb $gdb_version does not answer as a DAP server (gdb --interpreter=dap)." \
    "Its DAP server is written in Python, so gdb must be built with Python" \
    "(gdb -batch -ex 'python print(1)' shows whether it is); $install_help."
if [[ -n ${TUITOR:-} ]]; then
    tuitor=$(command -v "$TUITOR") || die "TUITOR=$TUITOR is not an executable."
else
    command -v cargo >/dev/null || die "cargo is not installed; it builds tuitor from this repository." \
        "Install Rust from https://rustup.rs, or point TUITOR at a tuitor binary: TUITOR=/path/to/tuitor ./run.sh"
fi

# 2. The example.
step "building orders (make: -g -O0)"
make -C "$here" --no-print-directory

# 3. tuitor: built from this repository so it matches the examples. The debug build is the
# one `cargo build` and `cargo test` already keep up to date; release would be a second,
# much slower build.
if [[ -z ${TUITOR:-} ]]; then
    step "building tuitor (cargo build -p tuitor)"
    cargo build -p tuitor --manifest-path "$repo/Cargo.toml"
    tuitor="${CARGO_TARGET_DIR:-$repo/target}/debug/tuitor"
fi

# 4. Debug. However tuitor ends, by Q or by SIGINT, SIGTERM or SIGHUP, it ends gdb and the
# program and puts the terminal back.
step "starting tuitor on configuration \"orders\" (Q quits)"
exec "$tuitor" -C "$here" orders "$@"
