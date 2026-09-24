# Worker Fetch object-store adapter

This document defines the Web Fetch API adapter for the asynchronous object-store contract.

It is implemented in `src/worker-object-store.mjs` and can be passed directly to the generated `WasmObjectTable` wrapper.

The adapter is compatible with Worker-style hosts that provide `fetch`, `URL`, `Headers`, `AbortController`, Web Crypto, and request-context timers.

It is a transport adapter, not a claim that txBASE supports a particular cloud provider.

## 1. Adapter boundary

`createWorkerObjectStore` returns the five Promise-returning methods required by `WasmObjectTable`:

| Method | Result |
| --- | --- |
| `get(key)` | `Uint8Array` or `null` when the object does not exist |
| `putIfAbsent(key, bytes)` | resolves after an immutable first publication |
| `compareAndSwap(key, expected, replacement)` | resolves after a conditional replacement |
| `delete(key)` | resolves after deletion; a missing object is already deleted |
| `list(prefix)` | sorted object-key strings |

The adapter accepts a base HTTP URL, optional default headers, an optional `AbortSignal`, and a non-negative request timeout.

The default timeout is 30 seconds and the default cache mode is `no-store`.

The adapter does not retry requests automatically.

The caller can retry a failed high-level operation after inspecting the shared recovery and conflict rules.

## 2. HTTP object protocol

The base URL identifies the object collection and must use `http` or `https`.

The adapter appends each slash-separated object-key component as a percent-encoded URL path component.

An object service must implement the following contract:

| Operation | Request | Success |
| --- | --- | --- |
| Read | `GET /<key>` | `200` with object bytes; `404` means `null` |
| Put if absent | `PUT /<key>` with `If-None-Match: *` and the bytes as the body | `200`, `201`, or `204` |
| Compare and swap | `PUT /<key>` with the replacement body and either `If-None-Match: *` or `If-Match: <etag>` | `200`, `201`, or `204` |
| Delete | `DELETE /<key>` | `200`, `204`, or `404` |
| List | `GET /?prefix=<encoded prefix>` | `200` with a JSON array of strings |

The service must return `409 Conflict` or `412 Precondition Failed` when a conditional write does not match.

For a non-empty compare-and-swap expected value, `<etag>` is the strong quoted SHA-256 digest of the expected bytes, encoded as unpadded base64url.

For an empty expected value, the adapter uses `If-None-Match: *`.

Other `4xx` responses are invalid requests, except `408` and `429`, which are unavailable transport outcomes.

All `5xx` responses are unavailable outcomes.

Object keys use ordinary non-empty path components; `.` and `..` components are rejected before a request is sent.

List prefixes may end in `/` to select a namespace, but may not contain an empty component elsewhere.

## 3. Timeout and cancellation

Each operation creates a private `AbortController` for its fetch request.

When the configured timeout expires, the request is aborted and the host rejection has `code: "unavailable"`.

When the caller's `AbortSignal` is aborted, the request is aborted and the host rejection has `code: "cancelled"`.

The WASM bridge maps `invalid`, `conflict`, `missing`, `unavailable`, and `cancelled` to the corresponding `ObjectStoreError` category.

The adapter removes its abort listener and timeout after the request settles.

This makes request cancellation visible to `AsyncObjectTable` without adding a Worker-specific dependency to the Rust core.

## 4. WASM usage

```js
import { createWorkerObjectStore } from "./worker-object-store.mjs";

const store = createWorkerObjectStore({
  baseUrl: "https://objects.example.test/users/",
  headers: { Authorization: `Bearer ${env.TXBASE_TOKEN}` },
  signal: request.signal,
  timeoutMs: 10_000,
});
const table = new WasmObjectTable(store, "users");
```

The object service is responsible for authentication, request routing, immutable object creation, conditional manifest updates, and strong ETag generation.

The adapter remains independent of the XBF manifest and recovery protocol.

## 5. Verification and remaining host work

`tests/wasm_worker_smoke.mjs` runs the generated WASM wrapper against a deterministic local HTTP object service.

The smoke test covers byte publication, manifest compare-and-swap, listing, missing reads, conflict mapping, timeout mapping, and caller cancellation through the WASM bridge.

The test uses Node's Web Fetch APIs as a portable host fixture; it is not a deployed Cloudflare Worker or WASI runtime test.

Worker or WASI query-stream scheduling, transport backpressure, asynchronous local storage, and host lifecycle behavior remain host-specific.

Cloud-provider authentication, consistency guarantees, retention, orphan cleanup scheduling, and service-specific retry policy belong in a provider adapter document.

## Primary references and scope

- [Cloudflare Workers fetch API](https://developers.cloudflare.com/workers/runtime-apis/fetch/)
- [Cloudflare Workers web standards](https://developers.cloudflare.com/workers/runtime-apis/web-standards/)
- [Cloudflare Workers request `AbortSignal`](https://developers.cloudflare.com/workers/runtime-apis/request/)
- [Fetch Standard](https://fetch.spec.whatwg.org/)
- [DOM Standard `AbortController`](https://dom.spec.whatwg.org/#interface-abortcontroller)
- [RFC 9110: HTTP Semantics](https://www.rfc-editor.org/rfc/rfc9110.html)

Cloudflare's documentation establishes the Worker Fetch and Web API host vocabulary.

The Fetch and DOM specifications establish the transport and cancellation primitives.

RFC 9110 establishes the conditional request semantics used by the adapter.

The HTTP object protocol, error mapping, and XBF integration are txBASE-owned contracts.
