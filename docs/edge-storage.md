# Edge storage and object-store commits

This document defines the generic XBF object-store contract, its local memory and filesystem backends, and the host adapters that reuse it.

The local boundary provides an in-memory fixture and a durable filesystem backend for the same generation, conditional publication, retry, recovery, historical-read, and explicit-retention contract.

The R2 binding's provider-specific conditions and pagination are documented separately in [Cloudflare R2 object-store adapter](r2-object-store.md).

## 1. Current boundary

The native DBF and XBF paths remain local file and sidecar implementations.

The `txbase::edge::ObjectTable` API adds a separate object-store commit boundary for XBF snapshots.

New commits store the encoded XBF snapshot as immutable 4 MiB byte pages and publish a page manifest.
Each page reference records its storage generation, byte length, and CRC-32C; matching pages at the same index may be reused from the immediately previous page-backed generation.
`ObjectTable` and `AsyncObjectTable` expose `page_manifest_at` and `read_page_at` for retained page-backed generations.
`read` and `read_at` still assemble and validate the complete XBF snapshot before decoding it; pages are byte ranges, not record-aligned database pages.
Version-1 manifests that point to whole `.xbf` snapshots remain readable.

`MemoryObjectStore` is a deterministic fixture that implements the `ObjectStore` contract with immutable object publication and manifest compare-and-swap.

`FilesystemObjectStore` persists the same contract below one directory.
It creates parent directories, publishes immutable objects with exclusive creation, serializes store operations with a lock file, and replaces manifests through a synced temporary file.
The filesystem backend is a local durable adapter and does not claim cloud-provider consistency.

`AsyncObjectStore` defines the same five primitive operations as a runtime-neutral future boundary.
`SyncObjectStoreAdapter` exposes the existing synchronous stores through already-ready futures, so native tests can exercise the asynchronous contract without selecting an executor.
`AsyncObjectTable` reuses the manifest, generation, recovery, retention, and orphan-cleanup rules through that future boundary.
`with_limits` applies the configured XBF limits to both snapshot encoding before publication and snapshot decoding during reads.
The synchronous adapter does not make filesystem I/O non-blocking, but it exercises the same high-level protocol without selecting an executor.

`AsyncObjectTable::query_stream` returns an `AsyncObjectQueryStream` that loads the current recovered committed snapshot through `AsyncObjectStore` and reuses the existing query snapshot stream.

`AsyncObjectTable::query_stream_at(generation, request)` selects one retained committed generation through the same boundary.

The first poll may be `Pending`; once loading completes, the stream owns a stable DBF-representable snapshot and accepts only filter, projection, skip, and limit controls.

Unsupported stream controls are rejected before the asynchronous storage read starts.

A missing current or retained snapshot, or an XBF value that the existing DBF export contract cannot represent, produces one query error item and then end-of-stream.

On `wasm32`, `WasmObjectTable` wraps the same asynchronous table protocol for a JavaScript host object.
The host supplies Promise-returning `get`, `putIfAbsent`, `compareAndSwap`, `delete`, and `list` methods.
The adapter exposes XBF bytes, manifest inspection, commit, historical reads, recovery, retention, and orphan cleanup through the generated `wasm-bindgen` wrapper.
It maps tagged host rejection codes to the shared object-store error categories, while timeout, cancellation, retry, and transport policy remain host responsibilities.
The same wrapper exposes `query_stream_json` for the current snapshot and `query_stream_json_at` for a retained generation.
Signal-aware `_with_signal` variants pass a query-scoped `AbortSignal` as the trailing argument to each host operation.
Each method resolves after loading and converting the selected snapshot, then returns the shared owned query stream.

`createWorkerObjectStore` supplies a Worker-compatible HTTP transport for those five methods.
It uses Web Fetch APIs, standard conditional request headers, strong SHA-256 ETags, `AbortSignal`, and a bounded request timeout without adding a cloud-provider dependency to the Rust core.
Its deterministic local HTTP fixture is exercised through the generated WASM wrapper in CI.

A provider-specific remote object-store adapter can implement `AsyncObjectStore` directly without changing the XBF snapshot or generation rules.

## 2. Manifest schema

One table uses this object layout:

New commits use this object layout:

```text
users/manifest.json
users/snapshots/142.pages.json
users/pages/142/0.bin
users/pages/141/1.bin
users/wal/142.json
```

The top-level manifest has this version-2 schema:

```json
{
  "version": 2,
  "generation": 142,
  "root": "users/snapshots/142.pages.json",
  "wal_head": 142,
  "history": [140, 141, 142]
}
```

`root` names an immutable page manifest.

The page manifest describes the encoded XBF byte sequence:

```json
{
  "version": 1,
  "generation": 142,
  "snapshot_length": 4194304,
  "page_size": 4194304,
  "pages": [
    {"generation": 142, "length": 4194304, "crc32c": 1234567890}
  ]
}
```

The array position is the page index.
Every page except the final page is exactly 4 MiB.
The page object's key is `users/pages/{generation}/{index}.bin`; a page reference can name an older storage generation when its bytes were reused.
Page CRC-32C values use the checksum implementation already defined by the XBF format.

`generation` is the visible table generation.

`wal_head` identifies the pending or published WAL generation in this local slice.

`history` lists the committed generations still addressable by `ObjectTable::read_at`.
The page-reading APIs return data only for retained page-backed generations.
Older manifests may omit it; those manifests expose only their current generation until the next commit writes history.

Readers continue to accept version-1 manifests whose `root` names a whole `.xbf` object.
The next successful commit writes a version-2 manifest and a page-manifest root.
Historical lookup rejects a generation when both legacy and page-manifest roots exist for it.
The reader rejects a snapshot whose embedded XBF generation differs from the top-level manifest generation.

## 3. Commit protocol

`ObjectTable::commit` performs the following operations:

```text
encode XBF and plan immutable pages plus their checksums
      ↓
write version-2 pending WAL, including the complete page manifest
      ↓
write or reuse immutable pages, then publish the page manifest
      ↓
compare-and-swap the manifest
      ↓
remove the pending WAL object
```

The manifest CAS uses the exact bytes read before the commit.

Two writers that publish different bytes for the same generation therefore produce a conflict instead of silently replacing one another.

Retrying the same generation with identical snapshot bytes returns `AlreadyCommitted`.

If `put-if-absent` reports a conflict, the commit reuses a page only when its existing bytes match exactly.
A content conflict removes that attempt's pending WAL so orphan cleanup can remove the conflicting page; transient storage errors keep the WAL plan available for retry.
This lets a retry resume after partial page publication without overwriting immutable data.

If publication fails after the WAL and all pages exist, `ObjectTable::recover` validates the page manifest, each page checksum, and the XBF generation, then completes the manifest CAS when the recorded base generation is still current.
Recovery skips an incomplete page set and leaves the old manifest visible; retrying the same target generation can finish that commit.

If the manifest was published but WAL cleanup failed, recovery removes the already-applied WAL without applying the snapshot twice.

A reader resolves one top-level manifest before loading its page manifest and pages.

The page manifest and pages are immutable, and each page's length and CRC-32C are checked before the bytes are assembled.
The embedded XBF generation is also checked, so a reader cannot accept a mixed-generation result.

`ObjectTable::read_at` reads a retained committed generation by assembling its immutable page set, or by reading its legacy whole-XBF object.
It returns no snapshot after that generation has been removed by retention.

`page_manifest_at` returns the manifest for a retained page-backed generation.
`read_page_at` returns one raw XBF byte page and validates its length and CRC-32C; it does not decode a partial XBF snapshot.
Both methods return no page data for a legacy whole-XBF generation.

## 4. Orphan cleanup

`ObjectTable::cleanup_orphans` keeps every generation listed in the current manifest history and every page referenced by those generations.
It also keeps the root and planned page references of any pending commit whose WAL still follows the current base generation.

It removes stale WAL objects, page manifests, legacy snapshots, and pages that are not in the committed history or a recoverable pending commit.

The method does not remove a recoverable pending commit.

`ObjectTable::retain_generations(keep_last)` first publishes a compacted manifest containing only the newest committed generations, then removes unreferenced roots and pages.
Pages shared by a retained generation remain available even when the generation that originally stored them is no longer retained.
The manifest update uses compare-and-swap, so a concurrent commit fails the retention operation instead of being silently deleted.
If deletion is interrupted, the compacted manifest remains authoritative and a later orphan cleanup can remove the leftover objects.
It does not run automatically, so an adapter can select a retention and expiration policy appropriate to its storage service.

## 5. XBF relationship

The object-store boundary stores encoded XBF snapshots as opaque byte pages and reuses the XBF checksum, schema validation, generation field, and decode limits.

It does not create a second query language or mutation model.

The top-level manifest is the mutable commit point, while page manifests, pages, legacy snapshots, and pending WAL objects remain immutable after publication.

## 6. Cloud adapter boundary

The R2 adapter implements the Cloudflare binding's conditional writes and cursor-based list operation; its exact mapping is in [Cloudflare R2 object-store adapter](r2-object-store.md).

An adapter for another provider still needs to define:

- conditional-write and retry error mapping;
- consistency guarantees for `get`, `list`, and compare-and-swap;
- snapshot expiration and generation retention;
- authentication and request limits;
- orphan cleanup scheduling;
- cloud-specific corruption and availability behavior.

These concerns do not belong in `MemoryObjectStore` or in the XBF codec.

`FilesystemObjectStore` supplies a local persistence fixture for those tests, but its lock file and filesystem durability behavior are not a substitute for a remote service's consistency contract.

The local and asynchronous object-table tests also verify that an encode-limit failure happens before the snapshot or pending WAL object is published.

The Worker Fetch adapter implements the host-managed HTTP shape described in [Worker Fetch object-store adapter](worker-object-store.md).
The remote adapter may implement `ObjectStore` for a blocking native client or `AsyncObjectStore` for another host-managed client.
The high-level asynchronous manifest protocol covers commit, recovery, retention, and conditional publication.
The JavaScript WASM adapter, Worker Fetch adapter, and R2 binding adapter supply host-managed implementations for this protocol.
The R2 adapter's conditional-write behavior is specific to Cloudflare and remains outside the generic transport contract.

The asynchronous query adapter supplies the generic storage-to-query handoff, but it does not select host scheduling, cancellation propagation, timeout, or retry behavior.
The Worker Web Streams adapter supplies demand control and forwards cancellation to signal-aware WASM host operations.
`createWorkerObjectStore` uses that per-query signal to abort the corresponding in-flight Fetch request.
The runtime-neutral `AsyncObjectStore` contract and custom hosts that ignore the optional signal do not gain cancellation from this adapter.

## 7. Explicit non-goals

This slice does not promise a deployed Worker or live R2 integration, provider-managed retention scheduling, WASI query-stream scheduling, a runtime-neutral cancellation contract for `AsyncObjectStore`, host-specific timeout or retry behavior, multi-region consensus, or automatic background garbage collection.

Those features can reuse the manifest and generation contract after their host-specific failure behavior has a deterministic test.

## Primary references and scope

- [POSIX `rename()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/rename.html)
- [POSIX `fsync()`](https://pubs.opengroup.org/onlinepubs/009695399/functions/fsync.html)
- [XBF v1 format draft](xbf.md)

POSIX `rename()` and `fsync()` are the relevant filesystem references for the local backend's temporary-file replacement and synchronization path.
The manifest, generation, compare-and-swap, recovery, and retention rules are txBASE-owned contracts, not claims about any cloud provider.

The local manifest and generation rules remain txBASE-owned contracts.
R2's provider-specific guarantees and their official sources belong in [Cloudflare R2 object-store adapter](r2-object-store.md), not in this generic contract.
