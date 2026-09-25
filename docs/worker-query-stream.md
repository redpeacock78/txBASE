# Worker query stream adapter

This document defines the JavaScript host adapter for exposing the WASM query stream as a Web `ReadableStream`.

It is a Worker-compatible host boundary, not a claim that txBASE has a deployed Cloudflare Worker integration.

## 1. Boundary

`WasmDatabase.query_stream_json()` creates a snapshot query stream from the existing bounded query parser and executor.

`WasmObjectTable.query_stream_json(body)` returns a JavaScript `Promise` that resolves after the current committed XBF snapshot has been recovered and loaded.
`WasmObjectTable.query_stream_json_at(generation, body)` selects one retained generation instead.
Both methods return an owned `WasmObjectQueryStream` and use the same query parser, validation, direct XBF row mapping, and shared row stream as the runtime-neutral async object-table API.
DBF export representability is not required for XBF queries; XBF-only JSON mappings are defined in [XBF](xbf.md).

Each stream owns its table snapshot and validated query request.
Later commits to either table do not change records already visible to that stream.

`WasmQueryStream.next_json()` and `WasmObjectQueryStream.next_json()` return one projected record as a JSON string or `null` at end of stream.
Their `cancel()` methods are idempotent and make later pulls fail with a cancellation error.

The stream accepts only the existing filter, projection, skip, and limit controls.
Sort, aggregation, cursor, and page controls are rejected before the stream is created because they require buffering or a different resume contract.

## 2. Web Streams adapter

`createWorkerQueryStream({ database, query, generation, signal, queueSize })` returns a `ReadableStream` whose chunks are UTF-8 `Uint8Array` values.
Each chunk contains exactly one JSON record followed by `\n`, so the stream can be used as an `application/x-ndjson` response body.
Without `generation`, the adapter calls `database.query_stream_json(body)`.
With a non-negative unsigned 64-bit `BigInt`, it calls `database.query_stream_json_at(generation, body)`.

The adapter uses an underlying pull source.
The adapter awaits a Promise-returning stream factory when needed, then each pull advances the WASM stream by at most one record and enqueues one chunk.
The `queueSize` option is a positive safe integer high-water mark for the Web Streams queue and defaults to `1`.
The adapter does not pre-materialize the result array.
For object-store queries, the complete committed XBF snapshot is loaded before the first row is emitted and rows are mapped on demand; remote snapshot reads are not page-streamed.

The Web Streams consumer controls demand through `read()`, `pipeTo()`, or `Response` body consumption.
When the consumer is slow, the underlying source is not pulled beyond the configured queue bound.

## 3. Cancellation and errors

An `AbortSignal` may be supplied by the host.
Aborting it cancels the WASM stream and errors the `ReadableStream` with `WorkerQueryStreamError` code `cancelled`.
With the signal-aware WASM object-table methods, the Worker adapter also forwards a per-query signal to the Fetch request loading that query's snapshot, so cancelling the query aborts that request without aborting other queries.
The runtime-neutral `AsyncObjectStore` contract still has no per-operation cancellation token; other adapters and hosts must define their own I/O cancellation policy.
Calling the reader's `cancel()` follows the same cancellation path without converting normal consumer cancellation into a data error.

Malformed query input, unsupported streaming controls, an invalid generation or queue size, or a missing WASM method use code `invalid`.
For a Promise-backed object table, initialization failures reach the stream as an error before its first row.
Unexpected WASM failures use code `unavailable`.
The adapter does not retry, catch, or reinterpret query failures as an empty stream.

Cancellation is a lifecycle boundary for the in-memory implementation, whose stream does not perform storage I/O.
Remote snapshot I/O is cancelled only by the Worker signal-aware object-table path described above.

## 4. Verification

`tests/wasm_query_stream_smoke.mjs` loads the generated `wasm-bindgen` wrapper and verifies:

- one-record NDJSON chunks match the shared query result;
- the query snapshot survives later database mutation;
- Promise-backed object-table queries return current and retained-generation rows;
- cancelling while a Promise-backed stream is initializing emits no row and cancels the stream after it resolves;
- queue size validation rejects an unbounded or zero-capacity configuration;
- sort is rejected before streaming;
- `AbortSignal` cancellation rejects the pending reader with the `cancelled` category;
- direct WASM cancellation is observable and terminal.

`tests/wasm_worker_smoke.mjs` verifies that cancelling a query aborts its in-flight snapshot Fetch, while a concurrent query remains unaffected until separately cancelled.

The CI WASM job builds `wasm32-unknown-unknown`, generates the pinned Node.js wrapper, and runs this fixture with the existing WASM smoke tests.

## 5. Scope

The adapter uses Web platform stream primitives and is suitable for a Worker-style host or a Node.js Web Streams fixture.
It does not provide a WASI scheduler, a network query endpoint, remote page reads, cursor resumption, or a generic `AsyncObjectStore` cancellation contract beyond the Worker Fetch path.

Those are separate host contracts and must define their own I/O, timeout, retry, backpressure, and recovery behavior.

## Primary references and scope

- [WHATWG Streams Standard](https://streams.spec.whatwg.org/)
- [WHATWG DOM `AbortSignal`](https://dom.spec.whatwg.org/#interface-AbortSignal)
- [Cloudflare Workers Streams](https://developers.cloudflare.com/workers/runtime-apis/streams/)
- [Cloudflare Workers `ReadableStream`](https://developers.cloudflare.com/workers/runtime-apis/streams/readablestream/)
- [wasm-bindgen: Promises and Futures](https://wasm-bindgen.github.io/wasm-bindgen/reference/js-promises-and-rust-futures.html)
- [wasm-bindgen: exported Rust types](https://wasm-bindgen.github.io/wasm-bindgen/reference/types/exported-rust-types.html)

The Streams Standard defines pull sources, asynchronous pull algorithms, queueing, backpressure, readers, and cancellation.
The DOM Standard defines `AbortSignal` lifecycle semantics.
The `wasm-bindgen` guide defines how an exported Rust `async fn` becomes a JavaScript `Promise` and how an exported Rust type can be returned from it.
Cloudflare's documentation is an implementation reference for a Worker-compatible host.
These sources do not define txBASE query semantics, cancellation of txBASE storage futures, or promise a deployed Worker runtime fixture.
