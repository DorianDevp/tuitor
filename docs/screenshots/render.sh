#!/usr/bin/env bash
# Renders the README's screenshots, assets/screenshots/*.png, from sessions on the examples:
# all of them, or those named (render.sh history paths), with only the scenes they need.
# Each scene is an example's run.sh with `tuitor --script <scene>.tsc`, which saves screens
# as ANSI (@dump); ansi2html.py makes a page of each, Chromium takes its picture at 4/3 of a
# pixel per CSS pixel, so that a cell is 10 × 24 pixels and the logo's octants fall on whole
# pixels, and ImageMagick brings it down to 256 colours.
#
# Needs what the examples need (PHP with Xdebug, Node.js and npm, js-debug, vscode-php-debug,
# Go with Delve on PATH, Chromium, ss), intelephense for PATHS (INTELEPHENSE=/path/to/it when
# it is not on PATH), the Iosevka Nerd Font, and ImageMagick's magick (MAGICK=convert for
# ImageMagick 6; CHROMIUM=google-chrome for another browser). tuitor gets a home of its own, with
# the examples copied to ~/tuitor/examples and your adapter directory linked in, so that the
# sessions start without your breakpoints, runs and configurations and the pictures show no
# path of yours; the adapters and the Go and npm caches stay yours.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
out=$repo/assets/screenshots
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# The pictures each run of scenes dumps. A run's scenes go together: php.tsc needs the traced
# run php-trace.tsc leaves in FLOW, php-paths-flow.tsc the runs php-paths.tsc leaves.
declare -A pictures=(
    [php]="overview flow history"
    [tabs]="tabs code doctor"
    [go]="project palette"
    [php-paths]="sets compare timeline paths"
)
runs=(php tabs go php-paths)
wanted=("$@")
if ((${#wanted[@]} == 0)); then
    for run in "${runs[@]}"; do read -ra names <<<"${pictures[$run]}"; wanted+=("${names[@]}"); done
fi
# needs run: whether a picture asked for comes from that run.
needs() {
    local name
    for name in ${pictures[$1]}; do
        [[ " ${wanted[*]} " == *" $name "* ]] && return 0
    done
    return 1
}
for name in "${wanted[@]}"; do
    [[ " ${pictures[*]} " == *" $name "* ]] || { echo "render.sh: no picture $name (${pictures[*]})" >&2; exit 2; }
done

intelephense=$(command -v "${INTELEPHENSE:-intelephense}") ||
    { echo "render.sh: ${INTELEPHENSE:-intelephense} not found; PATHS needs it (INTELEPHENSE=/path/to/it)" >&2; exit 1; }
cargo build -p tuitor --manifest-path "$repo/Cargo.toml"
export TUITOR="${CARGO_TARGET_DIR:-$repo/target}/debug/tuitor"
adapters="${XDG_DATA_HOME:-$HOME/.local/share}/tuitor/adapters"
export npm_config_cache="${npm_config_cache:-$HOME/.npm}"
GOPATH=$(go env GOPATH) GOCACHE=$(go env GOCACHE)
export GOPATH GOCACHE
export HOME=$work/home
unset XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME DISPLAY WAYLAND_DISPLAY
mkdir -p "$HOME/.local/share/tuitor" "$HOME/.config/tuitor" "$HOME/tuitor" "$adapters" "$out"
ln -s "$adapters" "$HOME/.local/share/tuitor/adapters"
# The examples' files that git does not ignore: no build, node_modules, trace or log.
git -C "$repo" ls-files -z --cached --others --exclude-standard examples |
    tar -C "$repo" --null -T - -cf - | tar -C "$HOME/tuitor" -xf -
examples=$HOME/tuitor/examples
cat >"$HOME/.config/tuitor/config.toml" <<EOF
[lsp.php]
command = "$intelephense"
initialization_options = { storagePath = "$work/intelephense", globalStoragePath = "$work/intelephense" }
EOF
cp "$here"/*.tsc "$work"

# example scene [arguments for tuitor]: dumps land in $work.
scene() {
    local example=$1 scene=$2
    shift 2
    echo "==> $example: $scene"
    "$examples/$example/run.sh" --script "$work/$scene" "$@" </dev/null >"$work/run.log" 2>&1 ||
        { tail -n 20 "$work/run.log" >&2; exit 1; }
}

if needs php; then
    TRACE=1 scene php php-trace.tsc
    TRACE=1 scene php php.tsc --size 160x38
fi
if needs tabs; then
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
fi
if needs go; then
    scene go go.tsc --size 160x28
fi
if needs php-paths; then
    scene php-paths php-paths.tsc --size 160x44
    scene php-paths php-paths-flow.tsc --size 160x44
fi

# name: $work/name.ans to $out/name.png.
shot() {
    local size
    size=$(python3 -I "$here/ansi2html.py" "$work/$1.ans" "$work/$1.html")
    "${CHROMIUM:-chromium}" --headless=new --disable-gpu --hide-scrollbars --user-data-dir="$work/chromium" \
        --force-device-scale-factor=1.3333333 --window-size="$size" --screenshot="$work/$1.png" \
        "file://$work/$1.html" >/dev/null 2>&1
    "${MAGICK:-magick}" "$work/$1.png" -dither None -colors 256 "PNG8:$out/$1.png"
    echo "    $out/$1.png"
}

echo "==> pictures"
for name in "${wanted[@]}"; do
    shot "$name"
done
