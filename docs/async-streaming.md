# Asynchronous query streaming

This document defines the runtime-neutral polling boundary for long-lived query streams.

It does not select an executor, a worker runtime, a network protocol, or a storage service.

## 1. Poll contract

`AsyncQueryStream` exposes `poll_next` with the same three-state result shape used by Rust's task system.

| Result | Meaning |
| --- | --- |
| `Poll::Ready(Some(item))` | One query result is available. |
| `Poll::Ready(None)` | The stream is finished and must not yield another item. |
| `Poll::Pending` | The stream is not ready; it must arrange for the supplied waker to be called before the next poll. |

The caller pins the stream for each poll.

Dropping the stream is the cancellation boundary for resources owned by an implementation.

The trait does not require `Send`, `Sync`, or a particular executor because worker and WASM hosts may use local tasks.

An implementation must not perform a blocking operation inside `poll_next`.

## 2. Current implementations

`QueryStream` and `QuerySnapshotStream` implement `AsyncQueryStream`.

Both streams read an in-memory table, so their polls complete immediately with `Ready` and do not need the task context.

`QueryStream` borrows the source table.

`QuerySnapshotStream` owns a cloned table and keeps the result stable after the source table changes.

`BoundedQueryStream` remains a blocking iterator over a standard-library channel.

It does not implement `AsyncQueryStream` because calling `recv` from `poll_next` would block the host task.

On native targets, `stream_query_threaded` returns `ThreadedQueryStream`.

It clones the table, runs the existing snapshot stream on one worker thread, and exposes the result through a positive-capacity standard-library channel.

Its `poll_next` uses only `try_recv`, registers the caller's waker when the channel is empty, and rechecks the channel after registration to avoid a lost wake-up.

The full channel applies backpressure to the worker.

Dropping the stream sets its cancellation flag, closes the receiver, and joins the worker so native production does not outlive the query owner.

This adapter is not compiled for `wasm32`.

This statement applies to `ThreadedQueryStream` only.
The separate WASI query-stream component polls the shared in-memory `QueryStream` and bridges rows to asynchronous WASI stdout; its command and runtime boundary are described in [WASI query streaming](wasi-query-stream.md).

`AsyncObjectTable::query_stream` returns `AsyncObjectQueryStream` for the current committed snapshot.

`AsyncObjectTable::query_stream_at` selects one retained generation through the same asynchronous-storage boundary.

Its first poll recovers and reads the selected committed XBF snapshot through `AsyncObjectStore`, converts that snapshot through the existing XBF-to-DBF export contract, and then delegates row delivery to the owned snapshot stream.

The load may return `Pending` and must wake the caller through the future's host contract; after the load completes, rows come from a stable in-memory snapshot.

The adapter preserves the store implementation's scheduling behavior; `SyncObjectStoreAdapter` returns ready futures but does not make blocking filesystem I/O non-blocking.

The adapter accepts the same filter, projection, skip, and limit controls as the existing snapshot stream.

Unsupported sort, aggregation, pagination, and cursor controls are rejected before the storage future is created.

An absent current or retained snapshot, or an XBF snapshot that cannot be represented as DBF, is reported as one `QueryError` item and the stream then ends.

## 3. Host responsibilities

A host-specific stream implementation owns the behavior that the shared contract cannot decide.

The native threaded adapter currently supplies worker scheduling, bounded backpressure, waker notification, and drop cancellation for in-process queries.

- The runtime schedules polling and supplies the task waker.
- The producer wakes the task after an asynchronous read or write makes an item available.
- The producer applies the desired queue or channel bound and defines what happens when the consumer is slow.
- Dropping the stream releases or cancels host resources.
- The host maps transport, storage, timeout, and host-specific cancellation failures into the stream's item error type.

The shared query layer owns filtering, projection, skip, limit, and the distinction between an item, end of stream, and pending work.

## 4. Relation to other contracts

The synchronous iterator and bounded NDJSON HTTP contracts remain unchanged.

`AsyncQueryStream` is a library boundary for native, worker, WASM, and WASI adapters.

The native threaded adapter is one concrete host implementation.

It does not make the filesystem channel non-blocking, add resume tokens, or define a remote storage protocol.

The Worker-compatible Web Streams adapter supplies pull scheduling, bounded queueing, NDJSON transport chunks, and `AbortSignal` cancellation for both the in-memory WASM query snapshot and the JavaScript-hosted object table.
`WasmObjectTable.query_stream_json` and `query_stream_json_at` return a Promise that resolves after the runtime-neutral `AsyncObjectTable` has loaded the current or selected retained XBF snapshot.
The runtime-neutral `AsyncObjectStore` contract has no operation cancellation token, so its generic adapters cannot abort an in-flight object-store Promise.
The Worker signal-aware methods `query_stream_json_with_signal` and `query_stream_json_at_with_signal` pass a query-scoped signal through the JavaScript host object table to the matching Fetch request, aborting that snapshot load without affecting concurrent queries.
WASI scheduling and remote retry policy remain host-specific.

## 5. Worker Web Streams adapter

`createWorkerQueryStream` wraps a synchronous or Promise-backed WASM query stream in a standard `ReadableStream`.
It awaits asynchronous stream creation before the first row, then each `pull` advances the snapshot by at most one record and enqueues one UTF-8 NDJSON chunk.
The positive `queueSize` high-water mark delegates demand control to the Web Streams queue.
Object-table queries load and convert one whole XBF snapshot before streaming rows; they do not stream remote pages.

The adapter propagates reader cancellation and an `AbortSignal` to the active WASM query stream's `cancel()` method.
Malformed query input, unsupported controls, and lifecycle failures remain errors; they are not converted into an empty result.

The full boundary, error categories, and deterministic generated-wrapper fixture are documented in [Worker query stream adapter](worker-query-stream.md).

## Primary references and scope

- [Rust `Context`](https://doc.rust-lang.org/std/task/struct.Context.html)
- [Rust `Poll`](https://doc.rust-lang.org/std/task/enum.Poll.html)
- [Rust `Pin`](https://doc.rust-lang.org/std/pin/index.html)

These references define the task context, readiness states, waker contract, and pinning model used by the boundary.

They do not define txBASE query semantics or imply compatibility with a particular async runtime.
