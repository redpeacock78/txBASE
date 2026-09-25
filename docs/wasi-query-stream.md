# WASI query streaming

Build the component with the pinned CI command, then run it in Wasmtime with a preopened directory.

## 1. Build and run

CI builds the `wasi-query-stream` example for Rust's `wasm32-wasip2` target.
The example uses `wasip3` to export a WASI 0.3 `wasi:cli/command` component.
This target-specific example requires Rust 1.87 or later; the crate's general minimum remains 1.85.

```bash
cargo build --locked --example wasi-query-stream --target wasm32-wasip2 --release
wasmtime run --dir ./data::/data target/wasm32-wasip2/release/examples/wasi_query_stream.wasm \
  /data/users.dbf '{"projection":{"NAME":1}}'
wasmtime run --dir ./data::/data target/wasm32-wasip2/release/examples/wasi_query_stream.wasm \
  --object-store /data/object-store users '{"projection":{"NAME":1}}'
wasmtime run --dir ./data::/data target/wasm32-wasip2/release/examples/wasi_query_stream.wasm \
  --object-store /data/object-store users '{"projection":{"NAME":1}}' --generation 0
```

The command prints one JSON object per line.
Wasmtime passes the component filename as argument zero, followed by the DBF path and query JSON.
The `--dir` option grants access to the host directory and maps it to `/data` inside the component.
The object-store form reads the current generation by default; `--generation` selects one retained generation.

## 2. Query contract

The DBF form reads one preopened DBF file into memory, then reuses `DbfTable::from_bytes`, `query::parse`, and `query::stream_query`.
The object-store form reads `namespace/manifest.json`, its XBF snapshot, and the namespace WAL directory through the existing `AsyncObjectTable::query_stream` or `query_stream_at` contract.
Its store root must be a directory exposed by `--dir`; the existing object-table layout places snapshots under `namespace/snapshots/` and recovery records under `namespace/wal/`.
The filesystem adapter is read-only and uses synchronous `std::fs` operations through `SyncObjectStoreAdapter`.
That adapter makes the calls fit the async trait but does not make filesystem I/O non-blocking.
The object store must remain stable during the query; this adapter does not coordinate with concurrent writers.
Because `AsyncObjectTable` recovers pending WAL records before reading, a recovery that needs to publish a manifest or delete a WAL record fails on the read-only store.
Snapshot loading and recovery finish before the first row is emitted.

Both forms reuse the shared query parser and query-stream contract.
They accept the shared streaming controls: `filter`, `projection`, `skip`, and `limit`.
It rejects `sort`, aggregation, pagination, and cursor controls before writing a row.

The component polls the existing `AsyncQueryStream` and serializes each result as one UTF-8 NDJSON line.
It writes rows to a WASI component-model byte stream and awaits `wasi:cli/stdout.write-via-stream` concurrently.
The stdout consumer therefore applies backpressure to row production.

## 3. Errors and cancellation

Argument, file, DBF, object-store, query, and stdout errors are written to stderr and return a failed command result.
Streaming can emit earlier rows before a later row error, so stdout is not an atomic result.
Storage, snapshot, and recovery errors occur before row production; the CI smoke test verifies that pending-WAL recovery fails without writing any row.
The CI smoke test also verifies that rejected streaming controls fail before writing any row.

Dropping the Rust query stream ends row production.
This command does not make synchronous DBF or object-store filesystem reads interruptible and does not define host timeout or process-cancellation policy.

## 4. CI boundary

The `wasi-query-stream` CI job installs the WASI target and Wasmtime `49.0.0`, builds the component, and runs `tests/wasi_query_stream_smoke.sh`.
The smoke check decodes pinned DBF and XBF fixtures, checks the DBF result and current and retained XBF generations, verifies that `sort` is rejected without stdout output, and verifies that pending-WAL recovery fails without stdout output.

This proves the component build and CLI behavior on the pinned Wasmtime runtime.
It does not prove deployment in another WASI host, writable or provider-backed storage, or non-blocking filesystem I/O.

## Primary references and scope

- [WASI 0.3 and native async](https://wasi.dev/releases/wasi-p3)
- [`wasip3` 0.9.0 bindings](https://docs.rs/wasip3/0.9.0%2Bwasi-0.3.0/wasip3/)
- [Rust `wasm32-wasip2` target](https://doc.rust-lang.org/rustc/platform-support/wasm32-wasip2.html)
- [Wasmtime CLI options](https://docs.wasmtime.dev/cli-options.html)
- [Bytecode Alliance Wasmtime setup action](https://github.com/bytecodealliance/actions)

WASI 0.3.1 is the current release and adds Component Model features beyond the async primitives introduced in WASI 0.3.0.
This adapter uses the 0.3.0 `stream` and `future` boundary and does not depend on the 0.3.1-only WIT features.
