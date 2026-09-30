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
The object-store form may need write access to recover a pending WAL before it can emit rows.
The host must grant that capability to the preopened directory.
The object-store form reads the current generation by default; `--generation` selects one retained generation.

## 2. Query contract

The DBF form reads one preopened DBF file into memory, then reuses `DbfTable::from_bytes`, `query::parse`, and `query::stream_query`.
The object-store form reads `namespace/manifest.json`, its XBF snapshot, and the namespace WAL directory through the existing `AsyncObjectTable::query_stream` or `query_stream_at` contract.
Its store root must be a directory exposed by `--dir`; the existing object-table layout places snapshots under `namespace/snapshots/` and recovery records under `namespace/wal/`.
The filesystem adapter implements `AsyncObjectStore` directly with WASI 0.3 filesystem descriptors, streams, and futures; it does not call `std::fs` or wrap filesystem calls in `SyncObjectStoreAdapter`.
It resolves the store root beneath a matching preopened directory and performs relative descriptor operations without following symbolic links.
It creates immutable objects exclusively, replaces manifests through a synchronized temporary file and same-directory rename, and deletes recovered WAL records.
Reads and writes use WASI streams, including concurrent polling of the byte producer and filesystem consumer for writes.
The adapter validates namespace and object-key components, rejects `.` and `..`, and refuses to traverse symbolic links.
The smoke test verifies that a `..` namespace, snapshot roots containing `.` or `..`, and a symlinked snapshot fail without writing rows to stdout.
The WASI filesystem interface exposes no interprocess-lock operation used by this adapter.
The host must prevent other processes from writing to the object store while this command runs; the adapter cannot enforce this single-writer precondition.
Because `AsyncObjectTable` recovers pending WAL records before reading, the host must allow writes when recovery needs to publish a manifest or delete a WAL record.
Snapshot loading and recovery finish before the first row is emitted.

Both forms reuse the shared query parser and query-stream contract.
They accept the shared streaming controls: `filter`, `projection`, `skip`, and `limit`.
Queries with `sort`, aggregation, pagination, or cursor controls fail before writing any rows.

The component polls the existing `AsyncQueryStream` and serializes each result as one UTF-8 NDJSON line.
It writes rows to a WASI component-model byte stream and awaits `wasi:cli/stdout.write-via-stream` concurrently.
The stdout consumer therefore applies backpressure to row production.
The smoke check delays stdout consumption while a DBF query emits 131,072 copies of one row, then validates every output row and the total count.

## 3. Errors and cancellation

Argument, file, DBF, object-store, query, and stdout errors are written to stderr and return a failed command result.
Streaming can emit earlier rows before a later row error, so stdout is not an atomic result.
Storage, snapshot, and recovery errors occur before row production; the CI smoke test verifies that pending-WAL recovery completes before writing rows.
The CI smoke test also verifies that rejected streaming controls fail before writing any row.

Dropping the Rust query stream ends row production.
The object-store path awaits WASI filesystem operations, but this command does not define whether dropping an operation future cancels host work or establish host timeout or process-cancellation policy.

## 4. CI boundary

The `wasi-query-stream` CI job installs the WASI target and Wasmtime `49.0.0`, builds the query component, pending-stream fixture, and direct object-store check component, then passes all three to `tests/wasi_query_stream_smoke.sh`.
The smoke check exercises missing reads, immutable creation and conflict behavior, recursive listing, compare-and-swap success and conflict, idempotent deletion, path validation, and a 2 MiB streamed write/read through the WASI adapter.
It also decodes pinned DBF and XBF fixtures and compares DBF, current-XBF, and retained-XBF query results while exercising `filter`, `projection`, `skip`, and `limit`.
It also verifies that `sort`, invalid namespaces and snapshot roots, and symbolic-link traversal fail without stdout output.
Pending-WAL recovery is checked both when the manifest already reflects the pending generation and when recovery must publish the missing manifest.
It reads the first stdout byte, pauses the pipe reader for 100 ms during a 131,072-row DBF query, then checks the complete output after the reader resumes.
The pending-stream fixture returns `Poll::Pending` once and stores the supplied waker.
A separate future invokes that waker only after the stream poll returns, after which the fixture must emit exactly one row.
A ten-second timeout makes a lost or ignored wake fail the smoke check.
This covers executor re-poll scheduling through the WASI query-output path, not host-I/O wake sources or asynchronous-storage lifecycle behavior.

This proves the component build and CLI behavior on the pinned Wasmtime `49.0.0` runtime, not production readiness of a WASI host.
WASI 0.3.1 is a stable specification, but Wasmtime's `wasmtime-wasi::p3` host module is documented as experimental, unstable, and incomplete.
The smoke test covers asynchronous filesystem calls, writable local storage, and recovery on this runtime, but does not validate a custom embedded host, deployment in another runtime, provider-backed storage, or host-side scheduling and cancellation guarantees.

## Primary references and scope

- [WebAssembly Component Model](https://component-model.bytecodealliance.org/)
- [WASI 0.3 and native async](https://wasi.dev/releases/wasi-p3)
- [`wasip3` 0.9.0 bindings](https://docs.rs/wasip3/0.9.0%2Bwasi-0.3.0/wasip3/)
- [`wasip3` filesystem descriptor API](https://docs.rs/wasip3/0.9.0%2Bwasi-0.3.0/wasip3/filesystem/types/struct.Descriptor.html)
- [`wasip3` preopened directories API](https://docs.rs/wasip3/0.9.0%2Bwasi-0.3.0/wasip3/filesystem/preopens/fn.get_directories.html)
- [Rust `wasm32-wasip2` target](https://doc.rust-lang.org/rustc/platform-support/wasm32-wasip2.html)
- [WASI filesystem WIT interface](https://github.com/WebAssembly/WASI/blob/main/proposals/filesystem/wit/types.wit)
- [Wasmtime CLI options](https://docs.wasmtime.dev/cli-options.html)
- [Wasmtime 49.0.0 WASI P3 host implementation](https://docs.rs/wasmtime-wasi/49.0.0/wasmtime_wasi/p3/index.html)
- [Bytecode Alliance Wasmtime setup action](https://github.com/bytecodealliance/actions)

The Component Model guide describes the component and interface concepts used by this command.
WASI 0.3.1 is the current release and adds Component Model features beyond the async primitives introduced in WASI 0.3.0.
This adapter uses the 0.3.0 `stream` and `future` boundary and does not depend on the 0.3.1-only WIT features.
