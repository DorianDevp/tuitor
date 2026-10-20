#!/usr/bin/env bash
# Renders the README's screenshots, assets/screenshots/*.png, from sessions on the examples.
# Each scene is an example's run.sh with `tuitor --script <scene>.tsc`, which saves screens
# as ANSI (@dump); ansi2html.py makes a page of each, Chromium takes its picture and
# ImageMagick brings it down to 256 colours.
#
# Needs what the examples need (PHP with Xdebug, Node.js and npm, js-debug, vscode-php-debug,
# Go with Delve on PATH, Chromium, ss), the Iosevka Nerd Font, and magick. tuitor gets a home
# of its own, so that the sessions start without your breakpoints, runs and configurations;
# the adapters and the Go and npm caches stay yours.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
examples=$repo/examples
out=$repo/assets/screenshots
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cargo build -p tuitor --manifest-path "$repo/Cargo.toml"
export TUITOR="${CARGO_TARGET_DIR:-$repo/target}/debug/tuitor"
export XDG_DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
export npm_config_cache="${npm_config_cache:-$HOME/.npm}"
GOPATH=$(go env GOPATH) GOCACHE=$(go env GOCACHE)
export GOPATH GOCACHE
export HOME=$work/home
unset XDG_CONFIG_HOME XDG_STATE_HOME DISPLAY WAYLAND_DISPLAY
mkdir -p "$HOME" "$out"
cp "$here"/*.tsc "$work"

# example scene [arguments for tuitor]: dumps land in $work.
scene() {
    local example=$1 scene=$2
    shift 2
    echo "==> $example: $scene"
    "$examples/$example/run.sh" --script "$work/$scene" "$@" </dev/null >"$work/run.log" 2>&1 ||
        { tail -n 20 "$work/run.log" >&2; exit 1; }
}

TRACE=1 scene php php-trace.tsc
TRACE=1 scene php php.tsc --size 160x38
# PHP for the second tab, once it listens.
(
    for _ in $(seq 300); do
        if [[ -n $(ss -Hltn 'sport = :9003') ]]; then
            XDEBUG_MODE=debug XDEBUG_SESSION=1 XDEBUG_CONFIG=client_port=9003 \
                exec "${PHP:-php}" "$examples/php/orders.php"
        fi
        sleep 0.2
    done
) >/dev/null 2>&1 &
HEADLESS=1 scene angular tabs.tsc --open "$examples/php" --size 160x40
scene go go.tsc --size 160x28

# name: $work/name.ans to $out/name.png.
shot() {
    local size
    size=$(python3 -I "$here/ansi2html.py" "$work/$1.ans" "$work/$1.html")
    chromium --headless=new --disable-gpu --hide-scrollbars --user-data-dir="$work/chromium" \
        --force-device-scale-factor=1.5 --window-size="$size" --screenshot="$work/$1.png" \
        "file://$work/$1.html" >/dev/null 2>&1
    magick "$work/$1.png" -dither None -colors 256 "PNG8:$out/$1.png"
    echo "    $out/$1.png"
}

echo "==> pictures"
for name in overview flow history tabs code doctor project palette; do
    shot "$name"
done
