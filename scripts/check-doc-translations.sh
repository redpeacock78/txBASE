#!/usr/bin/env bash
set -euo pipefail

heading_levels() {
    awk '
        /^```/ { fenced = !fenced; next }
        !fenced && /^#{1,6}[[:space:]]/ {
            match($0, /^#+/)
            print substr($0, 1, RLENGTH)
        }
    ' "$1"
}

markdown_links() {
    awk '
        /^```/ { fenced = !fenced; next }
        !fenced {
            line = $0
            while (match(line, /\]\([^)]*\)/)) {
                print substr(line, RSTART + 2, RLENGTH - 3)
                line = substr(line, RSTART + RLENGTH)
            }
        }
    ' "$1"
}

status=0

if ! diff -u <(heading_levels README.md) <(heading_levels README.ja.md) >/dev/null; then
    printf 'heading structure differs: README.md <-> README.ja.md\n' >&2
    status=1
fi

for source in docs/*.md; do
    name=${source#docs/}
    translation="docs/ja/$name"
    if [[ ! -f "$translation" ]]; then
        printf 'missing Japanese translation: %s\n' "$translation" >&2
        status=1
        continue
    fi
    if ! diff -u <(heading_levels "$source") <(heading_levels "$translation") >/dev/null; then
        printf 'heading structure differs: %s <-> %s\n' "$source" "$translation" >&2
        status=1
    fi
    if ! diff -u <(markdown_links "$source") <(markdown_links "$translation") >/dev/null; then
        printf 'link targets differ: %s <-> %s\n' "$source" "$translation" >&2
        status=1
    fi
done

for translation in docs/ja/*.md; do
    name=${translation#docs/ja/}
    source="docs/$name"
    if [[ ! -f "$source" ]]; then
        printf 'English source is missing: %s\n' "$source" >&2
        status=1
    fi
done

for source in README.md README.ja.md docs/*.md docs/ja/*.md; do
    directory=.
    if [[ "$source" == */* ]]; then
        directory=${source%/*}
    fi
    while IFS= read -r target; do
        case "$target" in
            ''|\#*|http://*|https://*|mailto:*) continue ;;
        esac
        target=${target%%\#*}
        [[ -n "$target" ]] || continue
        if [[ ! -e "$directory/$target" ]]; then
            printf 'missing local Markdown link: %s -> %s\n' "$source" "$target" >&2
            status=1
        fi
    done < <(markdown_links "$source")
done

exit "$status"
