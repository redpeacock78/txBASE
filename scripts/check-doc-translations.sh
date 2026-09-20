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

status=0

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
done

for translation in docs/ja/*.md; do
    name=${translation#docs/ja/}
    source="docs/$name"
    if [[ ! -f "$source" ]]; then
        printf 'English source is missing: %s\n' "$source" >&2
        status=1
    fi
done

exit "$status"
