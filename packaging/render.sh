#!/usr/bin/env bash
# Fills in a packaging template for the release workflow: @VERSION@ with $VERSION, and
# @SHA_<TARGET>@ with the checksum in <dir>/tuitor-$VERSION-<target>.tar.gz.sha256, the target
# in capitals with _ for - (the source archive's, tuitor-$VERSION-source.tar.gz, is @SHA_SOURCE@).
#
#     VERSION=0.1.0 packaging/render.sh packaging/homebrew/tuitor.rb.tmpl dist > tuitor.rb
#
# Fails, naming them, when a placeholder is left: a target that was not built.
set -euo pipefail

template=$1
dir=$2
: "${VERSION:?VERSION names the release, as in 0.1.0}"

args=(-e "s/@VERSION@/$VERSION/g")
for sum in "$dir"/*.sha256; do
    [[ -e $sum ]] || continue
    file=$(basename "$sum" .sha256)
    target=${file#tuitor-"$VERSION"-}
    target=${target%.tar.gz}
    key=$(tr 'a-z-' 'A-Z_' <<<"$target")
    args+=(-e "s/@SHA_${key}@/$(cut -d ' ' -f 1 "$sum")/g")
done
out=$(sed "${args[@]}" "$template")
if left=$(grep -o '@[A-Z0-9_]*@' <<<"$out"); then
    echo "render.sh: $template still has $(sort -u <<<"$left" | tr '\n' ' ')" >&2
    exit 1
fi
printf '%s\n' "$out"
