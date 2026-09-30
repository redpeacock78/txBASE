# WASM and worker host boundary

This document defines the boundary between the txBASE core and WebAssembly hosts.

The core ABI and current host adapters are described here.
Object-store, streaming, Worker, and WASI details belong in their focused documents.

## 1. Current WASM API

`wasm::WasmCore` owns an in-memory `DbfTable` and reuses the native parser, query, and mutation implementations.

The shared `DbfTable::apply_operation` implementation also serves DBF transactions, WAL recovery, catalog table application, and WASM.

The boundary uses `ABI_VERSION = 1`.

- `WasmCore::open_dbf` parses DBF bytes into in-memory state; the JavaScript wrapper performs this step in `new WasmDatabase(dbf)`. `snapshot` returns the current state as DBF bytes.
- `query_json` runs the bounded query document.
- `query_stream_json` returns a `WasmQueryStream` for the supported filter, projection, skip, and limit controls.
- Each `WasmQueryStream.next_json()` call returns one JSON-encoded row string or JavaScript `null` at end-of-stream.
- After `WasmQueryStream.cancel()`, the next `next_json()` call throws a cancellation error synchronously.
- `apply_operation_json` applies one `POST`, `PUT`, `PATCH`, or `DELETE` operation.
- `apply_operations_json` rejects an empty batch, applies an `{"operations":[...]}` batch to a private copy, and returns a snapshot only after every operation succeeds.

Public JSON methods reject inputs above the shared 1 MiB `MAX_JSON_INPUT_BYTES` limit before deserialization.

The shared `MAX_OPERATION_BATCH` limit is 1,000 operations.

The generated `wasm-bindgen` `WasmDatabase` wrapper exposes the core API on `wasm32-unknown-unknown`.

## 2. Core and host responsibilities

`WasmCore` reads and updates in-memory DBF state but does not access host files, make network requests, schedule tasks, or persist a transaction.

The core owns DBF and XBF codecs, query semantics, mutation rules, and transaction-state transitions.

The host owns persistence of returned snapshots, platform I/O, scheduling, concurrency control, and host-specific timeout and retry policy.

The host boundary must not make core behavior depend on POSIX files, a JavaScript runtime, or a particular WASI version.

## 3. Implemented host adapters

- `AsyncObjectStore` and `AsyncObjectTable` provide runtime-neutral XBF object operations, recovery, retained reads, and conditional publication.
  The generated `WasmObjectTable` wrapper connects those operations to a JavaScript host whose methods return Promises.
  See [edge storage](edge-storage.md) for the shared storage contract.
- `createWorkerObjectStore` maps the object-store contract to Web Fetch and conditional HTTP requests.
  `createR2ObjectStore` maps it to a Cloudflare R2 binding.
  See [Worker object storage](worker-object-store.md) and [R2 object storage](r2-object-store.md) for host-specific behavior.
- `AsyncObjectTable::query_stream` and `query_stream_at` stream rows from current or retained XBF snapshots without exporting DBF bytes.
  The generated wrapper exposes these queries as `WasmObjectQueryStream`, and `createWorkerQueryStream` adapts them to bounded Web Streams with NDJSON chunks.
- `AsyncObjectQueryStream` supports a runtime-neutral `CancellationToken`; dropping a read or list future does not guarantee that its host I/O stops.
  The generated WASM Promise methods do not expose that Rust token.
  Worker signal-aware queries pass a per-query `AbortSignal` to the host operations.
  See [asynchronous streaming](async-streaming.md) and [Worker query streaming](worker-query-stream.md) for cancellation details.
- The WASI 0.3 CLI component queries DBF files or current and retained XBF snapshots through a writable, single-writer preopened filesystem store.
  It recovers pending WAL records before emitting rows when the host grants write access, while filesystem operations remain synchronous.
  The adapter does not coordinate concurrent writers.
  See [WASI query streaming](wasi-query-stream.md) for its command and runtime contract.

## 4. Verification boundary

CI builds the `wasm32-unknown-unknown` wrapper and runs the generated-wrapper smoke tests with Node.js.

Deterministic Node.js fixtures exercise the Worker Fetch, R2 binding, and Web Streams adapters.

The `wasi-query-stream` job builds the WASI 0.3 component and runs its CLI smoke test on the pinned Wasmtime runtime.

These checks cover generated bindings and local host fixtures.
They do not establish compatibility with a deployed Worker, a live R2 service, or a production WASI host.

The [quality matrix](quality-matrix.md) lists the assertions and their test names.

## 5. Unsupported host guarantees

The repository does not claim production Worker deployment or live R2 service validation.

Provider-backed WASI storage and non-blocking WASI filesystem I/O remain unimplemented.

Browser storage, Node.js WASI, Deno, and Bun are not compatibility commitments.

## Primary references and scope

- [wasm-bindgen: Exported Rust Types](https://wasm-bindgen.github.io/wasm-bindgen/reference/types/exported-rust-types.html)
- [wasm-bindgen: Promises and Futures](https://wasm-bindgen.github.io/wasm-bindgen/reference/js-promises-and-rust-futures.html)
- [wasm-bindgen: `Result<T, E>`](https://wasm-bindgen.github.io/wasm-bindgen/reference/types/result.html)

The wasm-bindgen guide describes how exported Rust types map to JavaScript classes and how async exports map to Promises.
It also specifies that an exported `Result::Err` becomes a JavaScript exception.

The txBASE ABI and input limits come from its implementation and tests; host compatibility comes from its adapters and CI evidence.
WASI-specific references belong in [WASI query streaming](wasi-query-stream.md), which documents that adapter's target and runtime boundaries.
