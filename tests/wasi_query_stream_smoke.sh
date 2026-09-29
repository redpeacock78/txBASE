#!/usr/bin/env bash
set -euo pipefail

component=${1:?usage: wasi_query_stream_smoke.sh <component>}
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
temp_dir=$(mktemp -d)
trap 'rm -rf "$temp_dir"' EXIT

xxd -r -p < "$repo_root/tests/fixtures/users.dbf.hex" > "$temp_dir/users.dbf"
printf '%s\n' '{"NAME":"Alice"}' > "$temp_dir/expected-dbf.ndjson"
printf '%s\n' '{"NAME":"Alice"}' '{"NAME":"Bob"}' > "$temp_dir/expected-xbf.ndjson"
printf '%s\n' '{"NAME":"Bob"}' > "$temp_dir/expected-bob.ndjson"
: > "$temp_dir/expected-empty.ndjson"

# Generate more than 2 MiB of output so a delayed pipe reader exercises backpressure.
backpressure_records=131072
read -r header_low header_high record_low record_high < <(
  od -An -tu1 -j8 -N4 "$temp_dir/users.dbf"
)
header_length=$((header_low + 256 * header_high))
record_length=$((record_low + 256 * record_high))
record_hex=$(xxd -p -s "$header_length" -l "$record_length" "$temp_dir/users.dbf" | tr -d '\n')
backpressure_dbf="$temp_dir/backpressure.dbf"
head -c "$header_length" "$temp_dir/users.dbf" > "$backpressure_dbf"
printf -v record_count_hex '%08x' "$backpressure_records"
record_count_le="${record_count_hex:6:2}${record_count_hex:4:2}${record_count_hex:2:2}${record_count_hex:0:2}"
printf '%s' "$record_count_le" | xxd -r -p | dd of="$backpressure_dbf" bs=1 seek=4 conv=notrunc 2>/dev/null
awk -v record="$record_hex" -v count="$backpressure_records" \
  'BEGIN { for (i = 0; i < count; i++) printf "%s", record }' \
  | xxd -r -p >> "$backpressure_dbf"
printf '\x1a' >> "$backpressure_dbf"

mkdir -p "$temp_dir/object-store/users/snapshots" "$temp_dir/object-store/users/wal"
xxd -r -p < "$repo_root/tests/fixtures/query-stream-users.xbf.hex" \
  > "$temp_dir/object-store/users/snapshots/0.xbf"
printf '%s\n' \
  '{"version":1,"generation":0,"root":"users/snapshots/0.xbf","wal_head":0,"history":[0]}' \
  > "$temp_dir/object-store/users/manifest.json"

wasmtime run --dir "$temp_dir::/data" "$component" \
  /data/users.dbf '{"projection":{"NAME":1}}' > "$temp_dir/actual.ndjson"
cmp "$temp_dir/expected-dbf.ndjson" "$temp_dir/actual.ndjson"

# Stop reading stdout briefly, then validate the complete result after the pipe drains.
timeout 30s wasmtime run --dir "$temp_dir::/data" "$component" \
  /data/backpressure.dbf '{"projection":{"NAME":1}}' \
  | {
    IFS= read -r -N 1 first_byte
    printf '%s' "$first_byte" > "$temp_dir/backpressure.ndjson"
    sleep 0.1
    cat >> "$temp_dir/backpressure.ndjson"
  }
expected_row=$(sed -n '1p' "$temp_dir/expected-dbf.ndjson")
awk -v expected_rows="$backpressure_records" -v expected_row="$expected_row" '
  $0 != expected_row { printf "unexpected output at row %d\n", NR > "/dev/stderr"; exit 1 }
  END {
    if (NR != expected_rows) {
      printf "expected %d rows, received %d\n", expected_rows, NR > "/dev/stderr"
      exit 1
    }
  }
' "$temp_dir/backpressure.ndjson"

# DBF queries apply filtering before skip and limit.
wasmtime run --dir "$temp_dir::/data" "$component" \
  /data/users.dbf '{"filter":{"NAME":{"$eq":"Nobody"}},"projection":{"NAME":1},"skip":0,"limit":1}' \
  > "$temp_dir/filtered-dbf.ndjson"
cmp "$temp_dir/expected-empty.ndjson" "$temp_dir/filtered-dbf.ndjson"

wasmtime run --dir "$temp_dir::/data" "$component" \
  /data/users.dbf '{"filter":{"NAME":{"$eq":"Alice"}},"projection":{"NAME":1},"skip":1,"limit":1}' \
  > "$temp_dir/skipped-dbf.ndjson"
cmp "$temp_dir/expected-empty.ndjson" "$temp_dir/skipped-dbf.ndjson"

wasmtime run --dir "$temp_dir::/data" "$component" \
  --object-store /data/object-store users '{"projection":{"NAME":1}}' \
  > "$temp_dir/current-xbf.ndjson"
cmp "$temp_dir/expected-xbf.ndjson" "$temp_dir/current-xbf.ndjson"

# The current XBF query filters rows and stops after its one-row limit.
wasmtime run --dir "$temp_dir::/data" "$component" \
  --object-store /data/object-store users \
  '{"filter":{"NAME":{"$gte":"A"}},"projection":{"NAME":1},"skip":0,"limit":1}' \
  > "$temp_dir/limited-xbf.ndjson"
cmp "$temp_dir/expected-dbf.ndjson" "$temp_dir/limited-xbf.ndjson"

wasmtime run --dir "$temp_dir::/data" "$component" \
  --object-store /data/object-store users \
  '{"filter":{"NAME":{"$gte":"B"}},"projection":{"NAME":1},"skip":0,"limit":1}' \
  > "$temp_dir/filtered-xbf.ndjson"
cmp "$temp_dir/expected-bob.ndjson" "$temp_dir/filtered-xbf.ndjson"

wasmtime run --dir "$temp_dir::/data" "$component" \
  --object-store /data/object-store users \
  '{"filter":{"NAME":{"$gte":"A"}},"projection":{"NAME":1},"skip":1,"limit":1}' --generation 0 \
  > "$temp_dir/retained-xbf.ndjson"
cmp "$temp_dir/expected-bob.ndjson" "$temp_dir/retained-xbf.ndjson"

if wasmtime run --dir "$temp_dir::/data" "$component" \
  --object-store /data/object-store ../users '{"projection":{"NAME":1}}' \
  > "$temp_dir/traversal.stdout" 2> "$temp_dir/traversal.stderr"; then
  printf '%s\n' 'object store unexpectedly accepted a traversal namespace' >&2
  exit 1
fi

test ! -s "$temp_dir/traversal.stdout"
grep -q 'object-store namespace must contain ordinary non-empty key components' \
  "$temp_dir/traversal.stderr"

for dot_case in dot dotdot; do
  case "$dot_case" in
    dot) invalid_root='users/snapshots/./0.xbf' ;;
    dotdot) invalid_root='users/snapshots/../0.xbf' ;;
  esac
  mkdir -p "$temp_dir/invalid-root-$dot_case/users"
  printf '{"version":1,"generation":0,"root":"%s","wal_head":0,"history":[0]}\n' \
    "$invalid_root" > "$temp_dir/invalid-root-$dot_case/users/manifest.json"
  if wasmtime run --dir "$temp_dir::/data" "$component" \
    --object-store "/data/invalid-root-$dot_case" users '{"projection":{"NAME":1}}' \
    > "$temp_dir/invalid-root-$dot_case.stdout" 2> "$temp_dir/invalid-root-$dot_case.stderr"; then
    printf 'object store unexpectedly accepted snapshot root %s\n' "$invalid_root" >&2
    exit 1
  fi

  test ! -s "$temp_dir/invalid-root-$dot_case.stdout"
  grep -q 'snapshot root does not match generation 0' \
    "$temp_dir/invalid-root-$dot_case.stderr"
done

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

cp "$temp_dir/object-store/users/snapshots/0.xbf" "$temp_dir/outside.xbf"
mkdir -p "$temp_dir/symlink-store/users/snapshots" "$temp_dir/symlink-store/users/wal"
printf '%s\n' \
  '{"version":1,"generation":0,"root":"users/snapshots/0.xbf","wal_head":0,"history":[0]}' \
  > "$temp_dir/symlink-store/users/manifest.json"
ln -s ../../../outside.xbf "$temp_dir/symlink-store/users/snapshots/0.xbf"
if wasmtime run --dir "$temp_dir::/data" "$component" \
  --object-store /data/symlink-store users '{"projection":{"NAME":1}}' \
  > "$temp_dir/symlink.stdout" 2> "$temp_dir/symlink.stderr"; then
  printf '%s\n' 'object store unexpectedly followed a symbolic link' >&2
  exit 1
fi

test ! -s "$temp_dir/symlink.stdout"
grep -q 'symbolic links' "$temp_dir/symlink.stderr"
