#!/usr/bin/env bash
# Debugs the php-paths example in tuitor. tuitor listens for Xdebug ("Listen for Xdebug" in
# .vscode/launch.json, which stops on the first line of each request); once it does, PHP
# handles two requests with Xdebug on, `invoice.print 1002` and, once that has ended,
# `order.show 1001`, and their output goes to run.log. Arguments go to tuitor
# (./run.sh --script demo.tsc). PHP=/path/to/php picks a php with Xdebug;
# TUITOR=/path/to/tuitor uses that binary instead of building tuitor from this repository.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
install_help="see \"Installing adapters\" in $repo/README.md"
log="$here/run.log"
adapter="${XDG_DATA_HOME:-$HOME/.local/share}/tuitor/adapters/vscode-php-debug"
php_debug_version=1.40.2 # the release README.md installs

step() { echo "==> $*"; }

die() {
    echo "run.sh: $1" >&2
    shift
    for line in "$@"; do echo "    $line" >&2; done
    exit 1
}

# 1. Prerequisites.
step "checking PHP with Xdebug and Node.js"
php=$(command -v "${PHP:-php}") || die "${PHP:-php} not found." \
    "Install PHP with Xdebug 3 ($install_help)," \
    "or point PHP at one: PHP=/path/to/php ./run.sh"
# XDEBUG_MODE=off, so that this check does not try to reach a debugger.
if ! XDEBUG_MODE=off "$php" -r 'exit(version_compare(phpversion("xdebug") ?: "0", "3", ">=") ? 0 : 1);'; then
    # Installed but not enabled: some packages (Arch's xdebug) ship the ini line commented out.
    ext_dir=$("$php" -r 'echo rtrim(ini_get("extension_dir"), "/");')
    if [[ -f $ext_dir/xdebug.so ]]; then
        read -ra scanned <<<"$("$php" -r 'echo str_replace(",", " ", (string) php_ini_scanned_files());')"
        ini=$(grep -lE '^[[:space:]]*;[[:space:]]*zend_extension[[:space:]]*=.*xdebug' "${scanned[@]}" /dev/null | head -n 1 || true)
        [[ -z $ini ]] || die "Xdebug is installed ($ext_dir/xdebug.so) but $ini has it commented out." \
            "Enable it: sudo sed -i 's/^;[[:space:]]*zend_extension/zend_extension/' $ini"
        die "Xdebug is installed ($ext_dir/xdebug.so) but PHP does not load it." \
            "Add zend_extension=xdebug.so to an ini file that \`$php --ini\` lists."
    fi
    die "$php has no Xdebug 3 loaded (\`$php -v\` would mention it)." \
        "Install it: apt-get install php-xdebug, dnf install php-xdebug, pacman -S xdebug" \
        "or pie install xdebug/xdebug ($install_help)," \
        "or use a php that has it: PHP=/path/to/php ./run.sh"
fi
command -v node >/dev/null || die "Node.js is not installed; vscode-php-debug runs on it." \
    "Install Node.js 20.18 or newer: https://nodejs.org/en/download or your distribution's nodejs;" \
    "$install_help."
node_version=$(node --version)
IFS=. read -r node_major node_minor _ <<<"${node_version#v}"
(( node_major > 20 || (node_major == 20 && node_minor >= 18) )) ||
    die "Node.js $node_version is too old: vscode-php-debug needs 20.18 or newer." \
        "Install a newer one: https://nodejs.org/en/download or your distribution's nodejs; $install_help."
command -v ss >/dev/null || die "ss is not installed; run.sh uses it to see when tuitor listens." \
    "Install iproute2, e.g. pacman -S iproute2 or apt-get install iproute2."
port=$(grep -m 1 -o '"port": *[0-9]*' "$here/.vscode/launch.json") || die "No port in .vscode/launch.json."
port=${port##*[: ]}
[[ -z $(ss -Hltn "sport = :$port") ]] || die "Port $port is taken, so tuitor cannot listen for Xdebug on it." \
    "Is another debugger listening? ss -ltnp 'sport = :$port' shows which program it is."
# PATHS asks intelephense; everything else works without it.
command -v intelephense >/dev/null ||
    echo "note: intelephense is not on PATH, so P (PATHS) has no server to ask unless [lsp.php] in" \
        "tuitor's config.toml names one: npm install -g intelephense installs it."
if [[ -n ${TUITOR:-} ]]; then
    tuitor=$(command -v "$TUITOR") || die "TUITOR=$TUITOR is not an executable."
else
    command -v cargo >/dev/null || die "cargo is not installed; it builds tuitor from this repository." \
        "Install Rust from https://rustup.rs, or point TUITOR at a tuitor binary: TUITOR=/path/to/tuitor ./run.sh"
fi

# 2. The example needs no build, and no composer install: public/index.php loads the classes.

# 3. tuitor: built from this repository so it matches the examples.
if [[ -z ${TUITOR:-} ]]; then
    step "building tuitor (cargo build -p tuitor)"
    cargo build -p tuitor --manifest-path "$repo/Cargo.toml"
    tuitor="${CARGO_TARGET_DIR:-$repo/target}/debug/tuitor"
fi

# vscode-php-debug, which tuitor installs from its GitHub release as README.md ("Installing
# adapters") does, once you say so.
if [[ ! -f $adapter/extension/out/phpDebug.js ]]; then
    step "installing vscode-php-debug $php_debug_version"
    "$tuitor" install vscode-php-debug --version "$php_debug_version" ||
        die "vscode-php-debug is not installed." "To install it yourself, $install_help; then run ./run.sh again."
fi

# 4. Debug. PHP connects to tuitor, so a helper runs the two requests once tuitor listens (ss
# sees that without connecting: vscode-php-debug would take a test connection for Xdebug's),
# the second once the first has ended, or gives up once tuitor has ended.
requests=("invoice.print 1002" "order.show 1001")
again="XDEBUG_MODE=debug XDEBUG_SESSION=1 XDEBUG_CONFIG=client_port=$port $php $here/public/index.php"
step "starting tuitor on configuration \"Listen for Xdebug\" (Q quits)"
echo "    PHP handles ${requests[0]}, then ${requests[1]}, once tuitor listens on port $port;"
echo "    their output goes to $log."
echo "    To send a request while tuitor listens, in another terminal: $again order.show 1003"
script_pid=$$ # tuitor's, after the exec below
(
    echo "To send a request while tuitor listens: $again order.show 1003"
    echo "Waiting for tuitor to listen on port $port."
    until [[ -n $(ss -Hltn "sport = :$port") ]]; do
        kill -0 "$script_pid" 2>/dev/null || exit 0
        sleep 0.2
    done
    export XDEBUG_MODE=debug XDEBUG_SESSION=1 XDEBUG_CONFIG="client_port=$port"
    for request in "${requests[@]}"; do
        kill -0 "$script_pid" 2>/dev/null || exit 0
        echo "Request: $request"
        read -ra route_and_id <<<"$request"
        "$php" "$here/public/index.php" "${route_and_id[@]}"
    done
) >"$log" 2>&1 &
# However tuitor ends, by Q or by SIGINT, SIGTERM or SIGHUP, it ends vscode-php-debug, which
# stops PHP, and puts the terminal back.
exec "$tuitor" -C "$here" "Listen for Xdebug" "$@"
