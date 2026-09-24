# Distributed evolution

This document isolates the replication and distributed-database boundary.

txBASE now has a local, process-scoped replication slice and a bounded HTTP
delivery surface. It defines the versioned entry, single-authority replay,
snapshot installation, and versioned transport contracts, but it is not quorum
replication or consensus.

## 1. Prerequisites

Distributed behavior comes after the local transaction and edge-storage contracts are stable.

The current exclusive table lock does not imply consensus, replication, or multi-region behavior.

The first requirement is a deterministic mutation representation whose recovery behavior is already tested locally.

The current slice provides that boundary through `ReplicationEntry` and
`ReplicationLog`. It reuses `OperationIr` and the catalog journal instead of
introducing a second mutation engine.

## 2. Suggested progression

The implementation order should be:

```text
fine-grained WAL
      ↓
deterministic mutation IR
      ↓
single-writer correctness (current local slice)
      ↓
replicated log (current local replay slice)
      ↓
bounded HTTP entry and snapshot delivery (current transport slice)
      ↓
Raft or another selected authority protocol
```

Each step needs a standalone contract before the next step depends on it.

## 3. Replication entries

A version 1 log entry contains:

```text
version
term
index
transaction ID
catalog representation tag
operation IR
```

`ReplicationEntry` serializes this shape as JSON and rejects unknown fields,
unsupported versions, read operations, empty batches, and non-positive
positions. The catalog representation tag prevents an entry from being
applied to a different catalog image.

`ReplicationLog` selects one fixed term as the local authority. `propose`
commits a batch through the existing catalog journal and records it only after
the commit succeeds. It journals the next `TXRP` sidecar image in the same
catalog transaction, so the catalog data and local replication position recover
together. `receive` accepts only the next index and transaction ID, checks the
term and representation tag, and then applies the same atomic catalog commit.

The entry format defines duplicate delivery as a no-op when the complete entry
matches. A conflicting duplicate, an index or transaction gap, a term mismatch,
or an unavailable history prefix is rejected before a new commit.

The in-memory JSON form remains available for transport-independent fixtures.
The durable `TXRP` sidecar starts with the four-byte `TXRP` magic, a sidecar
format version byte, and the validated `ReplicationLog` JSON payload.
`ReplicationLog::open` loads that sidecar, checks the requested term and
catalog transaction position, and bootstraps a missing sidecar only at the
matching catalog position.

`read_at` returns a retained `Catalog::from_path_at` image only when the
requested transaction is covered by the applied log and the log position
matches the live catalog. `read_applied` reads the latest such image.
These methods define a local historical follower-read primitive; they do not
provide leases, linearizability, quorum freshness, or network transport.

### Snapshot installation

`ReplicationSnapshot` version 1 contains the fixed term, last log index, last
catalog transaction ID, catalog representation tag, and one encoded catalog
MVCC image. `ReplicationLog::snapshot` captures the current catalog image
under the catalog read lock and validates its schema tag before returning the
transport-independent value. The encoded catalog payload is limited to 64 MiB.

`ReplicationLog::install_snapshot` validates the version, positions, schema,
and term before changing local state. It replaces DBF, memo, and schema files
through the catalog journal, resets catalog MVCC history to the installed base,
clears catalog CDC and table-local transient MVCC, CDC, WAL, state, and index
sidecars, and journals the matching empty `TXRP` position in the same commit.
The in-memory log then resumes at the snapshot's next index and transaction.

Installing the same image is an acknowledged duplicate. Older images,
conflicting images at the same transaction, sidecar divergence, and a catalog
that changed during installation are rejected without publishing a partial
replacement. This is a local recovery primitive; snapshot transport, log
truncation policy, and authority coordination remain future distributed
contracts.

### HTTP transport boundary

The catalog server exposes a version 1 JSON delivery surface for an already
constructed replication entry or snapshot:

| Route | Contract |
| --- | --- |
| `GET` or `HEAD /replication/status` | Returns the transport version, fixed term, base and last positions, and the current catalog representation tag. |
| `GET` or `HEAD /replication/snapshot` | Exports the current validated `ReplicationSnapshot` JSON. |
| `POST /replication/entry` | Validates and delivers one `ReplicationEntry`; exact duplicates are acknowledged. |
| `POST /replication/snapshot` | Validates and installs one `ReplicationSnapshot` atomically. |

Entry request bodies use the existing 1 MiB JSON input bound. Snapshot request
bodies use the existing 64 MiB encoded-payload bound. Invalid documents return
`422`; term, position, schema, conflicting-duplicate, and snapshot state conflicts return
`409`; storage failures return `500`. Responses identify the transport version
and, for apply operations, the resulting index and transaction ID.

This is a delivery boundary, not a leader-election protocol.
The default `authority` role captures `/transaction` and named-table mutation routes in the same catalog journal commit as `TXRP` state and rechecks table ETags before commit.
The `follower` role reports its role through `status`, rejects direct catalog mutations with `409`, and still accepts replication delivery.
The transport has no authentication, TLS, streaming, retry queue, backpressure,
quorum, or authority discovery.

## 4. Co-location before distributed joins

Distributed relational support should first favor co-location.

Candidate partition keys include:

```text
tenant_id
workspace_id
user_id
```

Related tables should be placed together when possible:

```text
Shard A
├── user 1
├── posts for user 1
└── comments for user 1
```

This reduces the cases where a join becomes a network shuffle.

True distributed joins and distributed transactions remain later features.

## 5. Required contracts

The local slice defines the following initial contracts:

- authority: one process-local writer and one fixed term; no quorum is claimed;
- conflict and retry: exact duplicates are acknowledged, conflicting duplicates and gaps are rejected;
- schema version: the catalog representation tag must match before commit;
- recovery: a follower retries a missing prefix, and the journaled `TXRP` log can resume after process restart;
- follower reads: a caller can read a retained catalog image at an applied log transaction;
- snapshot recovery: a follower can install one validated catalog image atomically and resume at its next log position;
- transport: the catalog server accepts versioned entry and snapshot JSON through bounded HTTP routes with explicit conflict statuses;
- authority capture: the default authority role journals `/transaction` and named-table mutations with `TXRP` state, while the follower role rejects direct catalog mutations;
- deterministic failure fixture: the CI test suite delivers the second entry before the first and then recovers.

The following contracts remain open:

- schema migrations independent of the catalog representation tag;
- authentication and authority-coordinated log truncation;
- observability for lag and transport state;
- quorum and network failure behavior.

Change data capture, persistent WAL history, and replication must share the same ordering contract.

## 6. Acceptance conditions

The initial local replication slice is complete because it has:

- one selected local authority model: fixed-term single writer;
- the versioned `ReplicationEntry` and `ReplicationLog` formats;
- the journaled `TXRP` sidecar with term and catalog-position checks;
- the local historical follower-read boundary with applied-position checks;
- versioned snapshot export and atomic installation with stale, conflicting, and concurrent-change rejection;
- catalog-history compaction and `TXRP` base-position recovery after snapshot installation;
- deterministic replay, duplicate-delivery, conflict, and ordering tests;
- partition-gap, serialized-log recovery, term, and schema-tag tests;
- snapshot round-trip, installation, resume, stale-image, and conflict tests;
- bounded HTTP status, entry delivery, duplicate delivery, snapshot installation, and export tests;
- default authority capture, table-ETag recheck, and follower read-only role tests;
- explicit write consistency: only the next catalog transaction can commit;
- a leader/follower fixture that fails and recovers without external infrastructure.

Quorum replication, full MVCC coordination, distributed follower-read guarantees, and distributed
partitioning remain future work.

## 7. Explicit non-goals

This document does not promise Raft, quorum, multi-region writes, global
transactions, networked log truncation, distributed follower-read guarantees, or automatic partition
balancing.

Those choices require the authority and recovery contracts above.

## Primary references and scope

- [In Search of an Understandable Consensus Algorithm (Raft)](https://raft.github.io/raft.pdf)
- [Raft consensus algorithm](https://raft.github.io/)
- [RFC 9110: HTTP Semantics](https://www.rfc-editor.org/rfc/rfc9110.html)

The Raft paper is a candidate protocol reference for the authority step in the progression.
It does not select Raft for txBASE and does not define the future txBASE log, schema, or recovery format.

The current repository has a local entry/replay implementation, a versioned
snapshot installation primitive, a journaled `TXRP` sidecar, bounded HTTP
delivery routes, default authority capture for catalog mutations, a read-only
follower role, and a bounded historical follower-read primitive, but no
consensus, quorum, distributed follower-read guarantee, or distributed-join
implementation.
Those statements remain design constraints rather than compatibility claims.
