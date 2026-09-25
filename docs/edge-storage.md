# Edge storage and object-store commits

This document defines the implemented local object-store boundary and the future cloud adapter boundary.

The local boundary provides an in-memory fixture and a durable filesystem backend for the same generation, conditional publication, retry, recovery, historical-read, and explicit-retention contract.

It does not claim an R2 adapter or any other cloud-provider implementation.

## 1. Current boundary

The native DBF and XBF paths remain local file and sidecar implementations.

The `txbase::edge::ObjectTable` API adds a separate object-store commit boundary for XBF snapshots.

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

`createWorkerObjectStore` supplies a Worker-compatible HTTP transport for those five methods.
It uses Web Fetch APIs, standard conditional request headers, strong SHA-256 ETags, `AbortSignal`, and a bounded request timeout without adding a cloud-provider dependency to the Rust core.
Its deterministic local HTTP fixture is exercised through the generated WASM wrapper in CI.

A provider-specific remote object-store adapter can implement `AsyncObjectStore` directly without changing the XBF snapshot or generation rules.

## 2. Manifest schema

One table uses this object layout:

```text
users/manifest.json
users/snapshots/0.xbf
users/snapshots/1.xbf
users/wal/1.json
```

The manifest has one versioned schema:

```json
{
  "version": 1,
  "generation": 142,
  "root": "users/snapshots/142.xbf",
  "wal_head": 142,
  "history": [140, 141, 142]
}
```

`root` names an immutable XBF snapshot.

`generation` is the visible table generation.

`wal_head` identifies the pending or published WAL generation in this local slice.

`history` lists the committed generations still addressable by `ObjectTable::read_at`.
Older manifests may omit it; those manifests expose only their current generation until the next commit writes history.

The reader rejects a snapshot whose embedded XBF generation differs from the manifest generation.

## 3. Commit protocol

`ObjectTable::commit` performs the following operations:

```text
write immutable XBF snapshot with put-if-absent
      ↓
write pending WAL object with put-if-absent
      ↓
compare-and-swap the manifest
      ↓
remove the pending WAL object
```

The manifest CAS uses the exact bytes read before the commit.

Two writers that publish different bytes for the same generation therefore produce a conflict instead of silently replacing one another.

Retrying the same generation with identical snapshot bytes returns `AlreadyCommitted`.

If publication fails after the snapshot and WAL objects exist, `ObjectTable::recover` validates the XBF generation and completes the manifest CAS when the recorded base generation is still current.

If the manifest was published but WAL cleanup failed, recovery removes the already-applied WAL without applying the snapshot twice.

A reader resolves one manifest before loading its `root` object.

Because the root object is immutable and the embedded generation is checked, a reader cannot accept a mixed-generation result.

`ObjectTable::read_at` reads a retained committed generation from its immutable snapshot object.
It returns no snapshot after that generation has been removed by retention.

## 4. Orphan cleanup

`ObjectTable::cleanup_orphans` keeps every generation listed in the current manifest history and any pending root whose WAL still follows the current base generation.

It removes stale WAL objects and snapshot objects that are not in the committed history or a recoverable pending commit.

The method does not remove a recoverable pending commit.

`ObjectTable::retain_generations(keep_last)` first publishes a compacted manifest containing only the newest committed generations, then removes older snapshot objects.
The manifest update uses compare-and-swap, so a concurrent commit fails the retention operation instead of being silently deleted.
If deletion is interrupted, the compacted manifest remains authoritative and a later orphan cleanup can remove the leftover objects.
It does not run automatically, so an adapter can select a retention and expiration policy appropriate to its storage service.

## 5. XBF relationship

The object-store boundary stores encoded XBF snapshots and reuses the XBF checksum, schema validation, generation field, and decode limits.

It does not create a second query language or mutation model.

The manifest is the mutable commit point, while snapshots and pending WAL objects remain immutable after publication.

## 6. Cloud adapter boundary

A remote adapter still needs to define:

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
The JavaScript WASM adapter and Worker Fetch adapter supply host-managed fixtures for this protocol; provider-specific consistency and retry behavior remain outside this generic transport.

The asynchronous query adapter supplies the generic storage-to-query handoff, but it does not select host scheduling, cancellation propagation, timeout, or retry behavior.

## 7. Explicit non-goals

This slice does not promise an R2 adapter, a specific cloud vendor, Worker or WASI query-stream scheduling, host-specific timeout or retry behavior, multi-region consensus, automatic background garbage collection, immutable page splitting, or a cloud-backed WASM host.

Those features can reuse the manifest and generation contract after their host-specific failure behavior has a deterministic test.

## Primary references and scope

- [POSIX `rename()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/rename.html)
- [POSIX `fsync()`](https://pubs.opengroup.org/onlinepubs/009695399/functions/fsync.html)
- [XBF v1 format draft](xbf.md)

POSIX `rename()` and `fsync()` are the relevant filesystem references for the local backend's temporary-file replacement and synchronization path.
The manifest, generation, compare-and-swap, recovery, and retention rules are txBASE-owned contracts, not claims about any cloud provider.

No cloud-provider source belongs in the current implementation contract because no remote adapter has been selected.
Provider documentation should be added to a provider-specific adapter document when one is implemented.
