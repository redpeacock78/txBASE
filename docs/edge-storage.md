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

An object-store adapter can implement the same trait for a remote service without changing the XBF snapshot or generation rules.

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

`ObjectTable::retain_generations(keep_last)` explicitly removes older snapshot objects while preserving the newest committed generations and the current manifest.
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

## 7. Explicit non-goals

This slice does not promise an R2 adapter, a specific cloud vendor, multi-region consensus, automatic background garbage collection, immutable page splitting, or WASM hosting.

Those features can reuse the manifest and generation contract after their host-specific failure behavior has a deterministic test.
