#!/usr/bin/env bash
# Debugs the PHP example in tuitor. tuitor listens for Xdebug ("Listen for Xdebug" in
# .vscode/launch.json, which stops on the first line); once it does, PHP runs orders.php
# with Xdebug on and its output goes to run.log. Arguments go to tuitor
# (./run.sh --script demo.tsc). PHP=/path/to/php picks a php with Xdebug;
# TUITOR=/path/to/tuitor uses that binary instead of building tuitor from this repository.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
install_help="see \"Installing adapters\" in $repo/README.md"
log="$here/run.log"
adapter="${XDG_DATA_HOME:-$HOME/.local/share}/tuitor/adapters/vscode-php-debug"
php_debug_version=1.40.2 # the release README.md installs
tuitor_pid=
helper_pid=

step() { echo "==> $*"; }

die() {
    echo "run.sh: $1" >&2
    shift
    for line in "$@"; do echo "    $line" >&2; done
    exit 1
}

cleanup() {
    # PHP, or the helper still waiting to start it.
    if [[ -n $helper_pid ]]; then
        kill "$helper_pid" 2>/dev/null || true
    fi
    # Interrupted while tuitor runs: stop the adapter it started (node), then tuitor.
    if [[ -n $tuitor_pid ]] && kill -0 "$tuitor_pid" 2>/dev/null; then
        pkill -P "$tuitor_pid" || true
        kill "$tuitor_pid" 2>/dev/null || true
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
step "checking PHP with Xdebug, Node.js and vscode-php-debug"
php=$(command -v "${PHP:-php}") || die "${PHP:-php} not found." \
    "Install PHP with Xdebug 3 ($install_help)," \
    "or point PHP at one: PHP=/path/to/php ./run.sh"
# XDEBUG_MODE=off, so that this check does not try to reach a debugger.
XDEBUG_MODE=off "$php" -r 'exit(version_compare(phpversion("xdebug") ?: "0", "3", ">=") ? 0 : 1);' ||
    die "$php has no Xdebug 3 loaded (\`$php -v\` would mention it)." \
        "Install it: apt-get install php-xdebug, dnf install php-xdebug, pacman -S xdebug" \
        "or pie install xdebug/xdebug ($install_help)," \
        "or use a php that has it: PHP=/path/to/php ./run.sh"
command -v node >/dev/null || die "Node.js is not installed; vscode-php-debug runs on it." \
    "Install Node.js 20.18 or newer: https://nodejs.org/en/download or your distribution's nodejs;" \
    "$install_help."
node_version=$(node --version)
IFS=. read -r node_major node_minor _ <<<"${node_version#v}"
(( node_major > 20 || (node_major == 20 && node_minor >= 18) )) ||
    die "Node.js $node_version is too old: vscode-php-debug needs 20.18 or newer." \
        "Install a newer one: https://nodejs.org/en/download or your distribution's nodejs; $install_help."
if [[ ! -f $adapter/extension/out/phpDebug.js ]]; then
    url="https://github.com/xdebug/vscode-php-debug/releases/download/v$php_debug_version/php-debug-$php_debug_version.vsix"
    echo "vscode-php-debug is not installed in $adapter."
    echo "README.md (\"Installing adapters\") installs it there from its GitHub release:"
    echo "    $url"
    read -r -p "Download and unpack it now? [y/N] " answer || answer=
    [[ $answer == [yY]* ]] || die "vscode-php-debug is not installed; nothing was changed." \
        "To install it yourself, $install_help; then run ./run.sh again."
    command -v curl >/dev/null || die "Installing it needs curl."
    command -v unzip >/dev/null || die "Installing it needs unzip."
    step "installing vscode-php-debug $php_debug_version"
    # The commands from README.md, "Installing adapters".
    rm -rf "$adapter" && mkdir -p "$adapter"
    curl -fL -o "$adapter/php-debug.vsix" "$url"
    unzip -q "$adapter/php-debug.vsix" -d "$adapter" && rm "$adapter/php-debug.vsix"
fi
command -v ss >/dev/null || die "ss is not installed; run.sh uses it to see when tuitor listens." \
    "Install iproute2, e.g. pacman -S iproute2 or apt-get install iproute2."
# The port of the first configuration, "Listen for Xdebug".
port=$(grep -m 1 -o '"port": *[0-9]*' "$here/.vscode/launch.json") || die "No port in .vscode/launch.json."
port=${port##*[: ]}
[[ -z $(ss -Hltn "sport = :$port") ]] || die "Port $port is taken, so tuitor cannot listen for Xdebug on it." \
    "Is another debugger listening? ss -ltnp 'sport = :$port' shows which program it is."
if [[ -n ${TUITOR:-} ]]; then
    tuitor=$(command -v "$TUITOR") || die "TUITOR=$TUITOR is not an executable."
else
    command -v cargo >/dev/null || die "cargo is not installed; it builds tuitor from this repository." \
        "Install Rust from https://rustup.rs, or point TUITOR at a tuitor binary: TUITOR=/path/to/tuitor ./run.sh"
fi

# 2. The example: orders.php needs no build.

# 3. tuitor: built from this repository so it matches the examples. The debug build is the
# one `cargo build` and `cargo test` already keep up to date; release would be a second,
# much slower build.
if [[ -z ${TUITOR:-} ]]; then
    step "building tuitor (cargo build -p tuitor)"
    cargo build -p tuitor --manifest-path "$repo/Cargo.toml"
    tuitor="${CARGO_TARGET_DIR:-$repo/target}/debug/tuitor"
fi

# 4. Debug. PHP connects to tuitor, so a helper starts it once tuitor listens. ss sees that
# without connecting: vscode-php-debug would take a test connection for Xdebug's.
again="XDEBUG_MODE=debug XDEBUG_SESSION=1 XDEBUG_CONFIG=client_port=$port $php $here/orders.php"
step "starting tuitor on configuration \"Listen for Xdebug\" (Q quits)"
echo "    PHP runs once tuitor listens on port $port; its output goes to $log."
echo "    To run PHP again while tuitor listens, in another terminal: $again"
(
    echo "To run PHP again while tuitor listens: $again"
    echo "Waiting for tuitor to listen on port $port."
    until [[ -n $(ss -Hltn "sport = :$port") ]]; do sleep 0.2; done
    echo "Running PHP."
    export XDEBUG_MODE=debug XDEBUG_SESSION=1 XDEBUG_CONFIG="client_port=$port"
    exec "$php" "$here/orders.php"
) >"$log" 2>&1 &
helper_pid=$!
# tuitor runs in the background too, so that a signal to this script reaches the trap at
# once; <&0 keeps the terminal as its input.
"$tuitor" -C "$here" "Listen for Xdebug" "$@" <&0 &
tuitor_pid=$!
wait "$tuitor_pid"
