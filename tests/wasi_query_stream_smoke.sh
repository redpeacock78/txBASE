#!/usr/bin/env bash
set -euo pipefail

component=${1:?usage: wasi_query_stream_smoke.sh <component>}
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
temp_dir=$(mktemp -d)
trap 'rm -rf "$temp_dir"' EXIT

xxd -r -p < "$repo_root/tests/fixtures/users.dbf.hex" > "$temp_dir/users.dbf"
printf '%s\n' '{"NAME":"Alice"}' > "$temp_dir/expected-dbf.ndjson"
printf '%s\n' '{"NAME":"Alice"}' '{"NAME":"Bob"}' > "$temp_dir/expected-xbf.ndjson"

mkdir -p "$temp_dir/object-store/users/snapshots" "$temp_dir/object-store/users/wal"
xxd -r -p < "$repo_root/tests/fixtures/query-stream-users.xbf.hex" \
  > "$temp_dir/object-store/users/snapshots/0.xbf"
printf '%s\n' \
  '{"version":1,"generation":0,"root":"users/snapshots/0.xbf","wal_head":0,"history":[0]}' \
  > "$temp_dir/object-store/users/manifest.json"

wasmtime run --dir "$temp_dir::/data" "$component" \
  /data/users.dbf '{"projection":{"NAME":1}}' > "$temp_dir/actual.ndjson"
cmp "$temp_dir/expected-dbf.ndjson" "$temp_dir/actual.ndjson"

wasmtime run --dir "$temp_dir::/data" "$component" \
  --object-store /data/object-store users '{"projection":{"NAME":1}}' \
  > "$temp_dir/current-xbf.ndjson"
cmp "$temp_dir/expected-xbf.ndjson" "$temp_dir/current-xbf.ndjson"

wasmtime run --dir "$temp_dir::/data" "$component" \
  --object-store /data/object-store users '{"projection":{"NAME":1}}' --generation 0 \
  > "$temp_dir/retained-xbf.ndjson"
cmp "$temp_dir/expected-xbf.ndjson" "$temp_dir/retained-xbf.ndjson"

if wasmtime run --dir "$temp_dir::/data" "$component" \
  /data/users.dbf '{"sort":{"NAME":1}}' \
  > "$temp_dir/invalid.stdout" 2> "$temp_dir/invalid.stderr"; then
  printf '%s\n' 'unsupported streaming controls unexpectedly succeeded' >&2
  exit 1
fi

test ! -s "$temp_dir/invalid.stdout"
grep -q 'streaming query supports filter, projection, skip, and limit only' \
  "$temp_dir/invalid.stderr"

if wasmtime run --dir "$temp_dir::/data" "$component" \
  --object-store /data/object-store users '{"sort":{"NAME":1}}' \
  > "$temp_dir/invalid-xbf.stdout" 2> "$temp_dir/invalid-xbf.stderr"; then
  printf '%s\n' 'unsupported XBF streaming controls unexpectedly succeeded' >&2
  exit 1
fi

test ! -s "$temp_dir/invalid-xbf.stdout"
grep -q 'streaming query supports filter, projection, skip, and limit only' \
  "$temp_dir/invalid-xbf.stderr"

printf '%s\n' \
  '{"version":1,"base_generation":null,"target_generation":0,"root":"users/snapshots/0.xbf","wal_head":0}' \
  > "$temp_dir/object-store/users/wal/0.json"
if wasmtime run --dir "$temp_dir::/data" "$component" \
  --object-store /data/object-store users '{"projection":{"NAME":1}}' \
  > "$temp_dir/pending-wal.stdout" 2> "$temp_dir/pending-wal.stderr"; then
  printf '%s\n' 'read-only object store unexpectedly recovered a pending WAL' >&2
  exit 1
fi

test ! -s "$temp_dir/pending-wal.stdout"
grep -q 'object store is read-only' "$temp_dir/pending-wal.stderr"
