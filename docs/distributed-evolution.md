# Distributed evolution

This document isolates the future replication and distributed-database boundary.

The design is not a current txBASE feature.

## 1. Prerequisites

Distributed behavior comes after the local transaction and edge-storage contracts are stable.

The current exclusive table lock does not imply consensus, replication, or multi-region behavior.

The first requirement is a deterministic mutation representation whose recovery behavior is already tested locally.

## 2. Suggested progression

The implementation order should be:

```text
fine-grained WAL
      ↓
deterministic mutation IR
      ↓
single-writer correctness
      ↓
replicated log
      ↓
Raft or another selected authority protocol
```

Each step needs a standalone contract before the next step depends on it.

## 3. Replication entries

A future log entry could contain:

```text
term
index
transaction ID
operation IR
```

Replicating a logical mutation is preferable to copying complete DBF files when the operation and recovery contracts are stable.

The entry format must define idempotency, ordering, duplicate delivery, rejection, and replay behavior.

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

Before implementation, this model must define:

- the authority and quorum model;
- conflict and retry semantics;
- schema-version and migration behavior;
- snapshot installation and follower-read rules;
- recovery after partial replication;
- observability for terms, indexes, generations, and lag;
- deterministic local fixtures for partition and network failure.

Change data capture, persistent WAL history, and replication must share the same ordering contract.

## 6. Acceptance conditions

An initial distributed slice is complete only when it has:

- one selected authority protocol;
- a versioned replicated-entry format;
- deterministic replay and duplicate-delivery tests;
- partition, recovery, and schema-version tests;
- explicit read and write consistency guarantees;
- an end-to-end fixture that can fail without external infrastructure.

Until then, replication, full MVCC, follower reads, and distributed partitioning remain future work.

## 7. Explicit non-goals

This document does not promise Raft, multi-region writes, global transactions, or automatic partition balancing.

Those choices require the authority and recovery contracts above.
