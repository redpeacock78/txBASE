# Worker query stream adapter

This document defines the JavaScript host adapter for exposing the WASM query stream as a Web `ReadableStream`.

It is a Worker-compatible host boundary, not a claim that txBASE has a deployed Cloudflare Worker integration.

## 1. Boundary

`WasmDatabase.query_stream_json()` creates a snapshot query stream from the existing bounded query parser and executor.

The stream owns its table snapshot and its validated query request.
Later mutations of the `WasmDatabase` do not change records already visible to that stream.

`WasmQueryStream.next_json()` returns one projected record as a JSON string or `null` at end of stream.
`WasmQueryStream.cancel()` is idempotent and makes later pulls fail with a cancellation error.

The stream accepts only the existing filter, projection, skip, and limit controls.
Sort, aggregation, cursor, and page controls are rejected before the stream is created because they require buffering or a different resume contract.

## 2. Web Streams adapter

`createWorkerQueryStream({ database, query, signal, queueSize })` returns a `ReadableStream` whose chunks are UTF-8 `Uint8Array` values.
Each chunk contains exactly one JSON record followed by `\n`, so the stream can be used as an `application/x-ndjson` response body.

The adapter uses an underlying pull source.
One pull advances the WASM stream by at most one record and enqueues one chunk.
The `queueSize` option is a positive safe integer high-water mark for the Web Streams queue and defaults to `1`.
The adapter does not pre-materialize the result array.

The Web Streams consumer controls demand through `read()`, `pipeTo()`, or `Response` body consumption.
When the consumer is slow, the underlying source is not pulled beyond the configured queue bound.

## 3. Cancellation and errors

An `AbortSignal` may be supplied by the host.
Aborting it cancels the WASM stream and errors the `ReadableStream` with `WorkerQueryStreamError` code `cancelled`.
Calling the reader's `cancel()` follows the same cancellation path without converting normal consumer cancellation into a data error.

Malformed query input, unsupported streaming controls, an invalid queue size, or a missing WASM method use code `invalid`.
Unexpected WASM failures use code `unavailable`.
The adapter does not retry, catch, or reinterpret query failures as an empty stream.

Cancellation is a lifecycle boundary for this in-memory implementation.
It does not cancel a remote storage request because the stream does not perform storage I/O.

## 4. Verification

`tests/wasm_query_stream_smoke.mjs` loads the generated `wasm-bindgen` wrapper and verifies:

- one-record NDJSON chunks match the shared query result;
- the query snapshot survives later database mutation;
- queue size validation rejects an unbounded or zero-capacity configuration;
- sort is rejected before streaming;
- `AbortSignal` cancellation rejects the pending reader with the `cancelled` category;
- direct WASM cancellation is observable and terminal.

The CI WASM job builds `wasm32-unknown-unknown`, generates the pinned Node.js wrapper, and runs this fixture with the existing WASM smoke tests.

## 5. Scope

The adapter uses Web platform stream primitives and is suitable for a Worker-style host or a Node.js Web Streams fixture.
It does not provide a WASI scheduler, a network query endpoint, remote page reads, cursor resumption, or a provider-specific storage stream.

Those are separate host contracts and must define their own I/O, timeout, retry, backpressure, and recovery behavior.

## Primary references and scope

- [WHATWG Streams Standard](https://streams.spec.whatwg.org/)
- [WHATWG DOM `AbortSignal`](https://dom.spec.whatwg.org/#interface-AbortSignal)
- [Cloudflare Workers Streams](https://developers.cloudflare.com/workers/runtime-apis/streams/)
- [Cloudflare Workers `ReadableStream`](https://developers.cloudflare.com/workers/runtime-apis/streams/readablestream/)
- [wasm-bindgen guide](https://rustwasm.github.io/docs/wasm-bindgen/)

The Streams Standard defines pull sources, queueing, backpressure, readers, and cancellation.
The DOM Standard defines `AbortSignal` lifecycle semantics.
Cloudflare's documentation is an implementation reference for a Worker-compatible host.
These sources do not define txBASE query semantics or promise a deployed Worker runtime fixture.
