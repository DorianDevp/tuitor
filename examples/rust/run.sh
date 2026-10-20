#!/usr/bin/env bash
# Builds the Rust example and debugs it in tuitor, which stops at the start of main.
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
step "checking cargo and gdb"
command -v cargo >/dev/null || die "cargo is not installed." \
    "Install Rust from https://rustup.rs (it builds the example and tuitor)."
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
fi

# 2. The example, into target/ here, where .vscode/launch.json looks for it (even if
# CARGO_TARGET_DIR points elsewhere).
step "building orders (cargo build)"
cargo build --manifest-path "$here/Cargo.toml" --target-dir "$here/target"

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
