# Sourced by the tapes, out of sight, from the repository's root: a home of tuitor's own, as
# docs/screenshots/render.sh makes one, with the examples copied to ~/tuitor/examples and the
# adapter directory linked in, so that a demo starts without your breakpoints, runs and
# configurations and shows no path of yours. TUITOR is the binary the examples' run.sh use
# (target/release/tuitor by default); Docker keeps its own configuration, where buildx is.
# shellcheck shell=bash
repo=$PWD
export TUITOR="${TUITOR:-$repo/target/release/tuitor}"
TUITOR=$(realpath "$TUITOR")
PATH="$(dirname "$TUITOR"):$PATH"
adapters="${XDG_DATA_HOME:-$HOME/.local/share}/tuitor/adapters"
export DOCKER_CONFIG="${DOCKER_CONFIG:-$HOME/.docker}"
export GOPATH="${GOPATH:-$(go env GOPATH 2>/dev/null || echo "$HOME/go")}"
HOME=$(mktemp -d)
export HOME
unset XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME
mkdir -p "$HOME/.local/share/tuitor" "$HOME/tuitor" "$adapters"
ln -s "$adapters" "$HOME/.local/share/tuitor/adapters"
git -C "$repo" ls-files -z --cached --others --exclude-standard examples docs/tapes/build |
    tar -C "$repo" --null -T - -cf - | tar -C "$HOME/tuitor" -xf -
export PS1='$ '
cd "$HOME/tuitor/examples" || return
