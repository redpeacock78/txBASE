# WASM and worker host boundary

This document isolates the WASM and edge-runtime boundary.

The repository now contains a host-independent DBF core slice, a
runtime-neutral asynchronous object-store boundary, and a JavaScript host
adapter for asynchronous XBF object-table commits. It also contains a
Worker-compatible Fetch transport adapter with explicit timeout and
cancellation mapping. It also contains a Worker-compatible Web Streams query
adapter with bounded pull scheduling and `AbortSignal` cancellation. WASI-specific
query-stream runtime adapters remain future work.

## 0. Current implementation slice

`wasm::WasmCore` owns an in-memory `DbfTable` and reuses the native parser,
query validator, query executor, and mutation methods.

The shared `DbfTable::apply_operation` implementation is owned by
`src/dbf/operation.rs` and is used by DBF transactions, WAL recovery, catalog
table application, and WASM. WASM does not maintain a second path or body
validation implementation.

Its versioned boundary currently provides:

- `ABI_VERSION = 1`;
- all public JSON methods reject inputs larger than the shared
  `MAX_JSON_INPUT_BYTES` limit, currently 1 MiB, before deserialization;
- `open_dbf` and `snapshot` for byte-in/byte-out DBF state;
- `query_json` for the existing bounded query document;
- `query_stream_json` for a snapshot-owned stream that supports the existing
  filter, projection, skip, and limit controls and returns one JSON record per
  pull;
- `apply_operation_json` for the existing `POST`, `PUT`, `PATCH`, and `DELETE`
  operation IR;
- `apply_operations_json` for the same operation IR in the bounded
  `{"operations":[...]}` transaction document; it applies the whole batch to
  a private copy and publishes a snapshot only when every operation succeeds.
  The shared `MAX_OPERATION_BATCH` limit is 1,000 operations, and oversized
  or empty batches are rejected before any operation is applied;
- a `wasm-bindgen` `WasmDatabase` wrapper on `wasm32` with the same methods;
- native contract tests;
- a pinned Node.js `wasm-bindgen` smoke test that loads the generated wrapper,
  checks the ABI version and snapshot round trip, exercises all four mutation
  methods, and verifies atomic batch rollback;
- a `WasmObjectTable` adapter that accepts a JavaScript object-store host,
  bridges Promise-returning `get`, `putIfAbsent`, `compareAndSwap`, `delete`,
  and `list` methods to `AsyncObjectStore`, and exposes XBF read, commit,
  recovery, historical-read, retention, and orphan-cleanup methods;
- a `createWorkerObjectStore` adapter that maps those five operations to an
  HTTP object service through Web Fetch, conditional requests, strong
  SHA-256 ETags, request timeouts, and `AbortSignal` cancellation;
- a pinned Node.js host fixture that exercises compare-and-swap publication,
  a failed WAL cleanup followed by recovery, historical reads, retention, and
  host error mapping;
- a pinned Node.js Web Fetch fixture that exercises the Worker transport
  through the generated WASM wrapper, including timeout and cancellation;
- a `createWorkerQueryStream` adapter that exposes the WASM snapshot stream as
  a bounded Web `ReadableStream` of NDJSON chunks, with reader cancellation and
  `AbortSignal` lifecycle handling;
- runtime-neutral `AsyncObjectTable::query_stream` and `query_stream_at` adapters
  that load the current or one retained committed XBF snapshot through
  `AsyncObjectStore` and reuse the existing filter, projection, skip, and limit
  query-stream semantics after XBF-to-DBF conversion;
- a pinned Node.js Web Streams fixture that exercises backpressure-shaped pull
  scheduling, snapshot stability, invalid controls, and cancellation;
- a CI `wasm32-unknown-unknown` release build and wrapper smoke check.

The core does not write files, access a network, schedule tasks, or commit a
transaction. The host must persist the returned snapshot and provide
serialization, retry, and concurrency control.

## 1. Core boundary

The txBASE core should remain deterministic and mostly independent of its host.

The host should provide HTTP, asynchronous storage, clocks, and platform-specific APIs.

The core should own format codecs, query semantics, mutation rules, WAL encoding, and transaction state transitions.

## 2. Host and core split

A candidate arrangement is:

```text
JavaScript or TypeScript host
        |
        ├─ HTTP
        ├─ asynchronous object storage
        ├─ platform APIs
        ↓
     txbase.wasm
        |
        ├─ DBF and XBF codecs
        ├─ query engine
        ├─ mutation semantics
        ├─ WAL codec
        └─ transaction state machine
```

The host must not make the core depend on POSIX files or WASI-specific behavior.

## 3. Reused contracts

The WASM boundary should reuse the existing DBF and XBF codecs where supported.

It should expose the same bounded query, mutation, validation, and recovery semantics as the native library.

It should not introduce a second query language, a second transaction model, or a host-specific interpretation of DBF bytes.

The runtime-neutral `AsyncQueryStream` contract provides the same query stream item semantics to a host poller without selecting an executor.

Its in-memory implementations complete immediately; a worker or WASI host must still provide scheduling, wake-up, backpressure, timeout, cancellation, and transport behavior.

The native `ThreadedQueryStream` adapter supplies bounded scheduling, wake-up, backpressure, and drop cancellation outside `wasm32`.

It is a native host implementation of the shared contract and does not change the WASM ABI or provide worker/WASI host services.

The Worker-compatible `createWorkerQueryStream` adapter uses the Web Streams
pull boundary instead of a Rust executor. It advances the WASM snapshot by at
most one record per pull, applies a positive queue high-water mark, emits
UTF-8 NDJSON chunks, and maps reader or `AbortSignal` cancellation to the
WASM stream lifecycle.

The object-store contract belongs below the shared table and transaction interfaces.
The runtime-neutral `AsyncObjectStore` contract is implemented for the five primitive object operations.
`AsyncObjectTable` reuses the manifest, generation, recovery, retention, and conditional-publication contract through those operations.
Neither boundary selects an executor or turns blocking filesystem calls into non-blocking work.

The `wasm-bindgen` JavaScript adapter uses the same five operations as a host
object whose methods return Promises.
`get` resolves to a `Uint8Array` or `null`; `list` resolves to string keys; the
other methods resolve to `undefined`.
Host rejection objects may provide `code` values `invalid`, `conflict`,
`missing`, `unavailable`, or `cancelled`; the adapter maps them to the shared
`ObjectStoreError` categories and maps untagged rejection to `unavailable`.
The Worker Fetch adapter supplies HTTP transport, timeout, and cancellation
mapping, while retry policy remains a host or provider concern.

## 4. Target hosts

The same core could eventually run behind:

- native Rust APIs;
- Cloudflare Workers or another worker runtime;
- browser storage such as OPFS;
- Node, Deno, or Bun adapters;
- WASI-compatible runtimes;
- another host that supplies the required storage primitives.

The list is a compatibility target, not a promise that every host will be supported.

## 5. Host services

Before implementation, the boundary must define:

- asynchronous range reads and writes;
- immutable object publication;
- conditional manifest updates;
- bounded memory and payload limits;
- cancellation and timeout behavior;
- error and retry mapping;
- deterministic clock or generation inputs for tests.

The host owns scheduling and platform I/O.

The core owns the meaning of a committed database state.

## 6. Acceptance conditions

The current core slice meets the following initial conditions:

- a versioned host-neutral ABI;
- one native host fixture;
- identical query and mutation implementation paths across native and WASM;
- an asynchronous object-table fixture using the runtime-neutral store contract;
- a JavaScript host-backed asynchronous object-table fixture using the generated
  `wasm-bindgen` wrapper;
- a Worker-compatible Fetch object-store adapter and deterministic HTTP fixture;
- a Worker-compatible Web Streams query adapter and deterministic Node.js fixture;
- runtime-neutral asynchronous-storage-backed query-stream adapters for current
  and retained DBF-representable XBF snapshots;
- explicit malformed-input errors at the byte and JSON boundaries;
- a Node.js host smoke test for the generated `wasm-bindgen` wrapper.

The following conditions remain before calling a deployed worker or WASI host
complete:

- one smoke test in the selected worker or WASI runtime;
- a WASI-specific query-stream scheduler, backpressure, and lifecycle contract;
- host-specific timeout, retry, and cancellation behavior for asynchronous query streams;
- a provider-specific consistency and retry contract when remote storage is selected.

Until then, the Worker Fetch transport is a current generic host boundary, and
deployed worker or WASI runtime integration remains future work.

## 7. Explicit non-goals

This document does not promise a JavaScript ORM, a browser-only database, or a POSIX emulation layer inside WASM.

Those would be separate products and would obscure the shared core contract.

## Primary references and scope

- [WebAssembly Core Specification](https://webassembly.github.io/spec/core/)
- [WASI](https://wasi.dev/)
- [WebAssembly Component Model](https://component-model.bytecodealliance.org/)
- [Cloudflare Workers WebAssembly](https://developers.cloudflare.com/workers/runtime-apis/webassembly/)
- [Cloudflare Workers fetch API](https://developers.cloudflare.com/workers/runtime-apis/fetch/)
- [Cloudflare Workers web standards](https://developers.cloudflare.com/workers/runtime-apis/web-standards/)
- [Cloudflare Workers Request `AbortSignal`](https://developers.cloudflare.com/workers/runtime-apis/request/)
- [Node.js WASI](https://nodejs.org/api/wasi.html)
- [wasm-bindgen guide](https://rustwasm.github.io/docs/wasm-bindgen/)
- [`wasm-bindgen-futures` API](https://docs.rs/wasm-bindgen-futures/latest/wasm_bindgen_futures/)
- [`js-sys` `Function::apply` API](https://docs.rs/js-sys/latest/js_sys/struct.Function.html)

The WebAssembly and WASI specifications define the core module and host-interface vocabulary.
The Component Model and the Cloudflare Workers and Node.js pages are implementation references for possible hosts, not txBASE compatibility commitments.

The repository has a WASM core implementation, a generated-wrapper Node.js
smoke check, a JavaScript host-backed asynchronous object-table fixture, a
Worker-compatible Fetch object-store adapter and smoke fixture, and
runtime-neutral asynchronous object-store and object-table contracts. It also
has a runtime-neutral asynchronous-storage-backed query-stream adapter for
DBF-representable XBF snapshots, plus a Worker-compatible Web Streams query
adapter and smoke fixture.
It does not claim that a deployed worker or WASI runtime, WASI-specific
query-stream scheduler, provider-specific consistency or retry policy, or a
native recovery path is already supported.
