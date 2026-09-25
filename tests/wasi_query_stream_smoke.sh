#!/usr/bin/env bash
set -euo pipefail

component=${1:?usage: wasi_query_stream_smoke.sh <component>}
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
temp_dir=$(mktemp -d)
trap 'rm -rf "$temp_dir"' EXIT

xxd -r -p < "$repo_root/tests/fixtures/users.dbf.hex" > "$temp_dir/users.dbf"
printf '%s\n' '{"NAME":"Alice"}' > "$temp_dir/expected.ndjson"

wasmtime run --dir "$temp_dir::/data" "$component" \
  /data/users.dbf '{"projection":{"NAME":1}}' > "$temp_dir/actual.ndjson"
cmp "$temp_dir/expected.ndjson" "$temp_dir/actual.ndjson"

if wasmtime run --dir "$temp_dir::/data" "$component" \
  /data/users.dbf '{"sort":{"NAME":1}}' \
  > "$temp_dir/invalid.stdout" 2> "$temp_dir/invalid.stderr"; then
  printf '%s\n' 'unsupported streaming controls unexpectedly succeeded' >&2
  exit 1
fi

test ! -s "$temp_dir/invalid.stdout"
grep -q 'streaming query supports filter, projection, skip, and limit only' \
  "$temp_dir/invalid.stderr"
