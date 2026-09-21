# Edge storage and object-store commits

This document isolates the future storage model for object stores and edge runtimes.

The design is not a current txBASE feature.

## 1. Current boundary

The current storage boundary is local and range-oriented.

DBF and the draft XBF path use files, sidecars, WAL records, and local replacement rules.

Object storage has different primitives: immutable objects, conditional writes, and manifests that identify a committed generation.

The object-store design must preserve the existing query, mutation, and recovery contracts instead of creating a second database model.

## 2. Object layout

A candidate layout is:

```text
table.meta
pages/
  000001
  000002
  000003
wal/
  00000042
```

A manifest can identify the committed state:

```json
{
  "generation": 142,
  "root": "pages/000031",
  "wal_head": 9482
}
```

Pages and WAL records are immutable after publication.

The manifest is the mutable commit point.

## 3. Commit protocol

A candidate commit sequence is:

```text
write immutable pages
      ↓
write WAL
      ↓
conditionally update the manifest
      ↓
commit
```

An ETag or generation precondition can provide compare-and-swap protection against concurrent writers.

A failed manifest update must leave the previous generation readable.

Readers must resolve one manifest generation before reading pages so one result cannot mix generations accidentally.

## 4. Required contracts

Before implementation, this model must define:

- consistency and visibility for readers;
- manifest compare-and-swap failure behavior;
- retry and idempotency rules;
- orphan-page and orphan-WAL cleanup;
- generation retention and snapshot expiration;
- corruption detection and recovery;
- a deterministic local fixture that does not require a cloud account.

These contracts cover the failure cases that local file replacement currently handles directly.

## 5. Relation to XBF

XBF is the natural native format for immutable pages and generation snapshots.

The object-store backend should reuse XBF checksums, schema rules, query semantics, and transaction state transitions where they match.

Storage-specific code should own object keys, conditional manifest updates, retries, and garbage collection.

## 6. Acceptance conditions

An initial object-store slice is complete only when it has:

- one documented manifest schema;
- one deterministic in-memory or local object-store fixture;
- concurrent-writer conflict tests;
- reader generation-consistency tests;
- retry and orphan-cleanup behavior;
- recovery tests for interrupted page, WAL, and manifest publication.

Until then, object-storage commits remain future work.

## 7. Explicit non-goals

This document does not promise an R2 adapter, a specific cloud vendor, multi-region consensus, or automatic garbage collection policy.

Those choices require the contracts above and an end-to-end test.
