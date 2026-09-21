#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
binary="$repo_root/target/debug/txbase"
if [[ -x "$binary.exe" ]]; then
    binary="$binary.exe"
fi
if [[ ! -x "$binary" ]]; then
    printf 'txbase binary is missing: %s\n' "$binary" >&2
    exit 1
fi

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/txbase-e2e.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT
dbf="$work_dir/users.dbf"
step=0

run_step() {
    step=$((step + 1))
    printf '[%02d] %s\n' "$step" "$1" >&2
    shift
    "$@"
}

run_step "initialize a DBF through the CLI" "$binary" init "$dbf" \
    --field ID:N:4:0 --field NAME:C:16
run_step "insert Alice through the CLI" "$binary" insert "$dbf" '{"ID":1,"NAME":"Alice"}'
run_step "insert Bob through the CLI" "$binary" insert "$dbf" '{"ID":2,"NAME":"Bob"}'

step=$((step + 1))
printf '[%02d] inspect the schema through the CLI\n' "$step" >&2
schema=$("$binary" schema "$dbf")
if ! grep -Eq '"record_count"[[:space:]]*:[[:space:]]*2' <<<"$schema"; then
    printf 'schema did not report two records: %s\n' "$schema" >&2
    exit 1
fi

step=$((step + 1))
printf '[%02d] read the current table through the CLI\n' "$step" >&2
current=$("$binary" read "$dbf")
if ! grep -Eq '"ID"[[:space:]]*:[[:space:]]*1' <<<"$current" || ! grep -Eq '"ID"[[:space:]]*:[[:space:]]*2' <<<"$current"; then
    printf 'current read did not contain both records: %s\n' "$current" >&2
    exit 1
fi

step=$((step + 1))
printf '[%02d] list committed MVCC snapshots through the CLI\n' "$step" >&2
versions=$("$binary" mvcc list "$dbf")
if ! grep -Fq '[1,2]' <<<"$versions"; then
    printf 'unexpected MVCC versions: %s\n' "$versions" >&2
    exit 1
fi

step=$((step + 1))
printf '[%02d] read the first MVCC snapshot through the CLI\n' "$step" >&2
snapshot=$("$binary" mvcc read "$dbf" 1)
if ! grep -Eq '"ID"[[:space:]]*:[[:space:]]*1' <<<"$snapshot" || grep -Eq '"ID"[[:space:]]*:[[:space:]]*2' <<<"$snapshot"; then
    printf 'snapshot 1 was not stable: %s\n' "$snapshot" >&2
    exit 1
fi

run_step "verify the table through the CLI" "$binary" verify "$dbf"
printf 'CLI E2E passed\n'
