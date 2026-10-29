#!/usr/bin/env bash
# Writes docs/book/sitemap.xml from SUMMARY.md, since mdBook makes none. Run after
# `mdbook build docs`:
#
#     mdbook build docs && docs/llms-full.sh && docs/sitemap.sh
set -euo pipefail

docs=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
out=$docs/book/sitemap.xml
site=https://doriandevp.github.io/tuitor
today=$(date -u +%F)

{
    echo '<?xml version="1.0" encoding="UTF-8"?>'
    echo '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">'
    echo "  <url><loc>$site/</loc><lastmod>$today</lastmod></url>"
    grep -o '](\([^)]*\.md\))' "$docs/src/SUMMARY.md" | sed 's/^](\(.*\))$/\1/' | while read -r page; do
        echo "  <url><loc>$site/${page%.md}.html</loc><lastmod>$today</lastmod></url>"
    done
    echo '</urlset>'
} >"$out"
echo "$out"
