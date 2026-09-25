# Cloudflare R2 object-store adapter

This document defines the adapter for a Cloudflare Workers R2 bucket binding.

It is implemented in `src/r2-object-store.mjs` and returns the five Promise-based methods required by `WasmObjectTable`.

It defines R2-specific conditional writes and pagination; the generic Worker Fetch protocol remains in [Worker Fetch object-store adapter](worker-object-store.md).

## 1. Adapter boundary

`createR2ObjectStore(bucket)` accepts a Workers R2 bucket binding and returns `get`, `putIfAbsent`, `compareAndSwap`, `delete`, and `list` methods.

| Method | Result |
| --- | --- |
| `get(key)` | `Uint8Array` or `null` when the object is absent |
| `putIfAbsent(key, bytes)` | publishes an object only if the key is absent |
| `compareAndSwap(key, expected, replacement)` | replaces an object only if its current bytes match `expected` |
| `delete(key)` | deletes an object; deleting a missing object succeeds |
| `list(prefix)` | returns sorted keys from every matching R2 list page |

The adapter requires the R2 binding methods `get`, `put`, `delete`, and `list`, plus the Web `Headers` API supported by Workers.

The adapter does not retry binding operations or add a timeout or cancellation mechanism.

## 2. Conditional writes

`putIfAbsent` and a compare-and-swap with `expected === null` call `put` with `If-None-Match: *`.

For a non-null `expected`, the adapter reads the current object and compares its bytes before writing.

If the bytes match, it sends `If-Match` with the object's quoted `httpEtag`.

This second condition prevents a concurrent update from being overwritten after the read.

R2's raw `etag` is not treated as a txBASE SHA-256 digest; the adapter uses the quoted `httpEtag` only as the R2 conditional-write token.

When an R2 conditional `put` returns `null`, the adapter rejects with `code: "conflict"`.

An absent `get` result maps to `null`, a malformed binding result maps to `invalid`, and a rejected binding operation maps to `unavailable`.

R2 documents strong consistency for reads, writes, deletes, and object listing.

## 3. Listing and pagination

The adapter requests at most 1,000 keys per page and follows R2's opaque `cursor` while `truncated` is true.

It does not stop when a page contains fewer keys than requested because R2 may return a shorter page.

It rejects malformed pages, keys outside the requested prefix, and missing or repeated cursors.

The `AsyncObjectStore` contract returns all matching keys in one `Vec`, so the adapter accumulates every page in memory before sorting the result.

## 4. WASM usage

```js
import { WasmObjectTable } from "./txbase.js";
import { createR2ObjectStore } from "./r2-object-store.mjs";

const store = createR2ObjectStore(env.TXBASE);
const table = new WasmObjectTable(store, "users");
```

The Worker must bind an R2 bucket to `env.TXBASE` in its Wrangler configuration.

The table namespace selects key prefixes; it is not an authorization boundary for the bucket.

Worker routes must enforce their own authorization, and the binding should have only the required bucket access.

## 5. Verification and limits

`tests/wasm_r2_smoke.mjs` connects the generated WASM wrapper to a deterministic in-memory R2 binding fixture.

The CI check covers snapshot publication and reading, missing reads, conditional conflicts, competing compare-and-swap writes, idempotent deletion, multi-page listing, malformed cursors, and unavailable bindings.

The fixture checks the binding API shape; it does not deploy a Worker or contact a Cloudflare account.

The adapter implements object-store operations only; it does not configure bucket lifecycle rules, schedule retention or orphan cleanup, or provide cross-object transactions.

## Primary references and scope

- [Cloudflare R2 Workers API reference](https://developers.cloudflare.com/r2/api/workers/workers-api-reference/)
- [Cloudflare R2 consistency model](https://developers.cloudflare.com/r2/reference/consistency/)

The Workers API reference defines the binding methods, conditional operations, ETag fields, and cursor-based listing used here.

The consistency document defines Cloudflare's consistency claims for R2 operations.

The adapter mapping and its limits are txBASE contracts; the local fixture does not verify Cloudflare's live service behavior.
