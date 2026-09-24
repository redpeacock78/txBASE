# Distributed evolution

This document isolates the replication and distributed-database boundary.

txBASE now has a local, process-scoped replication slice. It defines the
versioned entry and single-authority replay contract, but it is not network
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
the commit succeeds. `receive` accepts only the next index and transaction ID,
checks the term and representation tag, and then applies the same atomic
catalog commit.

The entry format defines duplicate delivery as a no-op when the complete entry
matches. A conflicting duplicate, an index or transaction gap, a term mismatch,
or an unavailable history prefix is rejected before a new commit.

The log can be serialized and restored as JSON. The caller must persist those
bytes with its own atomic file or object-store boundary; txBASE does not yet
install a durable `TXRP` sidecar or a network transport.

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
- recovery: a follower retries a missing prefix, and a serialized log can resume after process restart;
- deterministic failure fixture: the CI test suite delivers the second entry before the first and then recovers.

The following contracts remain open:

- schema migrations independent of the catalog representation tag;
- snapshot installation and follower-read consistency;
- partial replication recovery after log truncation;
- observability for lag and transport state;
- quorum and network failure behavior.

Change data capture, persistent WAL history, and replication must share the same ordering contract.

## 6. Acceptance conditions

The initial local replication slice is complete because it has:

- one selected local authority model: fixed-term single writer;
- the versioned `ReplicationEntry` and `ReplicationLog` formats;
- deterministic replay, duplicate-delivery, conflict, and ordering tests;
- partition-gap, serialized-log recovery, term, and schema-tag tests;
- explicit write consistency: only the next catalog transaction can commit;
- a leader/follower fixture that fails and recovers without external infrastructure.

Network replication, full MVCC coordination, follower reads, and distributed
partitioning remain future work.

## 7. Explicit non-goals

This document does not promise Raft, quorum, multi-region writes, global
transactions, durable replication history, or automatic partition balancing.

Those choices require the authority and recovery contracts above.

## Primary references and scope

- [In Search of an Understandable Consensus Algorithm (Raft)](https://raft.github.io/raft.pdf)
- [Raft consensus algorithm](https://raft.github.io/)

The Raft paper is a candidate protocol reference for the authority step in the progression.
It does not select Raft for txBASE and does not define the future txBASE log, schema, or recovery format.

The current repository has a local entry/replay implementation but no network
transport, consensus, quorum, follower-read, or distributed-join implementation.
Those statements remain design constraints rather than compatibility claims.
