#!/usr/bin/env bash
# Writes docs/book/llms-full.txt: every page of the book, in SUMMARY.md's order, as one Markdown
# file for language models (https://llmstxt.org). Run after `mdbook build docs`:
#
#     mdbook build docs && docs/llms-full.sh && docs/sitemap.sh
set -euo pipefail

docs=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
out=$docs/book/llms-full.txt
site=https://doriandevp.github.io/tuitor

{
    echo "# tuitor"
    echo
    echo "> $(sed -n 's/^description = "\(.*\)"$/\1/p' "$docs/book.toml")"
    echo
    grep -o '](\([^)]*\.md\))' "$docs/src/SUMMARY.md" | sed 's/^](\(.*\))$/\1/' | while read -r page; do
        echo
        echo "<!-- $site/${page%.md}.html -->"
        echo
        if [[ $page == changelog.md ]]; then
            cat "$docs/../CHANGELOG.md"
        else
            cat "$docs/src/$page"
        fi
    done
} >"$out"
echo "$out"
