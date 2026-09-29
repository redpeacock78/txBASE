# Distributed evolution

This document isolates the replication and distributed-database boundary.

txBASE has a process-scoped fixed-term replication mode with bounded HTTP
delivery, plus an optional OpenRaft mode initialized from an explicitly
configured voter set. The latter provides quorum writes, a linearizable read
barrier, authenticated peer RPC, and peer HTTPS. Its peer API adds learners,
changes voter sets through joint consensus, and waits for learner catch-up
before promotion. CI covers learner snapshot transfer and membership changes,
plus a three-node partition, failover, healing, and restart scenario.

CI verifies peer HTTPS certificate trust and hostname matching.
The failover test also holds one non-empty `AppendEntries` request past its RPC timeout and releases it after the remaining quorum commits.
This covers one stale-request case, not broad delayed or reordered RPC schedules.
The `/transaction` retry test drops a successful handler response after commit, but does not simulate a socket disconnect.
Broader delayed or reordered schedules, socket-level retries after losing a committed response, interrupted joint-membership recovery, reads during leadership changes, and crash-boundary injection remain open.

The fixed-term mode is not consensus. The current Raft boundary and remaining
work are documented in [Raft consensus design](raft.md).

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
bounded HTTP status, contiguous entry-range, entry, snapshot, and progress delivery plus one-shot client catch-up (current transport slice)
      ↓
optional Raft mode initialized from an explicit voter set (current implementation; see [Raft consensus design](raft.md))
      ↓
learner admission, joint-consensus voter changes, and three-node partition/failover testing (implemented)
      ↓
broader delayed or reordered RPC schedules, interrupted joint-membership recovery, and crash-boundary tests (remaining Raft work)
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

Mutation-only transactions retain the version 1 `operations` shape.
A transaction containing `setConstraints` uses version 2 with an ordered
`steps` array of mutation IR values and constraint commands.
Version 2 preserves the request order and still requires at least one record
mutation.
Older readers reject version 2, so every follower must support it before the
authority submits a constraint-mode transaction.

`ReplicationLog` selects one fixed term as the local authority. `propose`
commits a batch through the existing catalog journal and records it only after
the commit succeeds. It journals the next `TXRP` sidecar image in the same
catalog transaction, so the catalog data and local replication position recover
together. The ordered-step proposal path commits version 2 entries through the
same journal boundary. `receive` accepts only the next index and transaction ID,
checks the term and representation tag, and then applies the same atomic
catalog commit while preserving step order.
`receive_batch` validates one contiguous `ReplicationEntryBatch` and delivers
its entries in order. The page is a transport boundary rather than a
transaction boundary, so a failure may leave an already applied entry prefix
in place and a retry safely acknowledges that prefix as duplicates.

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
replacement. This is a local recovery primitive; networked snapshot transport
remains a future distributed contract.

### Authority-side log compaction

`ReplicationLog::snapshot_at` exports a validated catalog image at any
retained index covered by the applied log. `compact_through` accepts that image,
verifies its term, position, schema, and exact local catalog bytes, and removes
only the log prefix through the image. Entries after the image remain available
for immediate replay.

Compaction changes only the journaled `TXRP` sidecar. The catalog transaction ID
does not advance, the catalog MVCC history is not silently garbage-collected,
and reopening the catalog restores the new base position and retained suffix.
Reads before the new base return an explicit history-unavailable error.

The sidecar-only journal path gives one local authority a crash-recoverable
retention primitive. It does not decide which followers have acknowledged a
snapshot, establish a quorum, or expose networked log truncation.

### Follower watermark coordination

`ReplicationProgress` is the control-plane acknowledgement sent by a follower
after it has applied a position.
It contains a bounded follower identifier, term, log index, catalog transaction
ID, and catalog representation tag.
`ReplicationLog::progress_for` constructs this acknowledgement from the
follower's applied log position and live catalog representation.

The authority validates the acknowledgement against its live log and catalog.
It rejects a different term or schema tag, an index outside the retained log,
an index and transaction pair that is not contiguous, and a position that
regresses for an already registered follower.
An exact repeated acknowledgement is a duplicate and does not change state.

The authority computes the safe compaction index as the minimum acknowledged
index across all registered followers.
`compact_through_acknowledged` rejects compaction when no follower has
registered progress or when the requested snapshot is beyond that minimum.
`compact_through` applies the same minimum-index safety gate when followers
are registered; it is not a bypass for follower safety.

Follower progress is stored in a separate journaled `TXRG` sidecar named
`.txbase.replication-progress`; it is not mixed into the `TXRP` data-plane
sidecar.
The progress sidecar is updated through metadata-only catalog journal commits,
so it does not advance the catalog transaction ID.
`ReplicationLog::open` validates every persisted acknowledgement against the
current term, schema, retained base, and catalog position before restoring it.
Snapshot installation replaces the `TXRP` state and removes the `TXRG` state
in the same catalog journal commit; a duplicate installation also removes
stale progress state.

This contract is a watermark safety check, not quorum or consensus.
It has no membership configuration or lifecycle, lease, fencing token, durable
network retry queue, or failure detector.

### HTTP transport boundary

The catalog server exposes a version 1 JSON delivery surface for contiguous entry
ranges, already constructed replication entries, snapshots, and follower progress acknowledgements:

| Route | Contract |
| --- | --- |
| `GET` or `HEAD /replication/status` | Returns the transport version, fixed term, base and last positions, catalog representation tag, follower count, and safe compaction index. |
| `GET` or `HEAD /replication/entries?after=<index>&limit=<count>` | Returns a bounded contiguous `ReplicationEntryBatch` page after the requested retained index, with `next_after` when another entry remains. |
| `GET` or `HEAD /replication/snapshot` | Exports the current validated `ReplicationSnapshot` JSON. |
| `POST /replication/entry` | Validates and delivers one `ReplicationEntry`; exact duplicates are acknowledged. |
| `POST /replication/snapshot` | Validates and installs one `ReplicationSnapshot` atomically. |
| `POST /replication/progress` | Authority-only endpoint that validates one `ReplicationProgress` acknowledgement and returns the current safe compaction index. |

Entry and progress JSON helpers, including their serialized output, use the
existing 1 MiB JSON bound. Entry-range responses use the same bound. Snapshot JSON uses the existing 64 MiB
encoded-payload bound. Invalid documents return
`422`; term, position, schema, conflicting-duplicate, progress, and snapshot state conflicts return
`409`; storage failures return `500`. Responses identify the transport version
and, for apply or progress operations, the resulting index and transaction ID.
The entry-range query accepts a non-negative `after` index and a `limit` from 1 through
128, defaults to `after=0` and the maximum limit, and returns contiguous retained entries only.
An `after` position before the retained base or beyond the applied index returns `409`.
Malformed or repeated query parameters return `400`. The serialized response is capped at the
shared 1 MiB JSON boundary, so a page may contain fewer entries than its requested limit.

This is a delivery boundary, not a leader-election protocol.
The default `authority` role captures `/transaction` and named-table mutation routes in the same catalog journal commit as `TXRP` state and rechecks table ETags before commit.
The `follower` role reports its role through `status`, rejects direct catalog mutations with `409`, and still accepts replication delivery.
When `TXBASE_REPLICATION_TOKEN` is configured, the replication routes require
RFC 6750 Bearer authorization and return `401` with `WWW-Authenticate: Bearer`
for missing or invalid credentials. Without that environment variable, the routes
remain unauthenticated for local development compatibility. The replication client
supports HTTPS and verifies certificates and host names with the operating
system's trust facilities. Bearer tokens require HTTPS, except for loopback
HTTP. The public catalog listener remains HTTP-only, so deployments that need
network encryption must terminate TLS at a reverse proxy and protect the
proxy-to-server connection, for example by keeping it on loopback. These
limits apply to fixed-term replication: it has no mutual TLS, streaming,
durable retry queue, backpressure, quorum, or authority discovery. Optional
Raft mode uses a separate authenticated peer listener and supports peer HTTPS;
see [Raft consensus design](raft.md).

### HTTP client

`ReplicationHttpClient` connects the existing delivery routes to a local
`ReplicationLog`.
It accepts an `http://` or `https://` authority URL, an optional base path,
a positive socket timeout, and an optional validated Bearer token.
HTTPS uses the platform trust store and verifies the authority name; private
certificate authorities must be installed in that trust store.

`status()` validates the remote term, schema tag, retained base, and applied
position.
`entries()` reads one bounded contiguous page, `snapshot()` reads the current
validated image, and the three POST methods deliver an entry, snapshot, or
follower progress acknowledgement.

`catch_up()` is a bounded one-shot pull operation.
It reads the authority status, installs the current snapshot when the local
cursor is behind retention or the requested history has been compacted, reads
pages until the initial authority position is reached, applies each page in
order, and acknowledges the resulting follower position.
An entry page remains independently atomic, so a failure can leave an applied
prefix that a later call safely retries.

The public CLI exposes the same one-shot operation as
`txbase replicate catch-up DIRECTORY AUTHORITY_URL --replication-term TERM --follower-id ID`.
It opens the local catalog and `TXRP` sidecar, optionally reads
`TXBASE_REPLICATION_TOKEN`, and prints the synchronization result as JSON.
The command is intentionally not a daemon, scheduler, durable retry queue, or leader
election process; invoke it again to resume a persisted prefix.

The client uses HTTP/1.1 with `Connection: close`, explicit
`Content-Length`, the existing `1 MiB` entry/progress and `64 MiB` snapshot
limits, and no chunked decoding.
Each request uses a bounded retry policy: three total attempts by default, an
initial 50 millisecond delay, and exponential backoff capped at one second.
Only socket I/O failures and HTTP `408`, `429`, `500`, `502`, `503`, or `504`
are retried. Exact repeated entry, snapshot, and progress requests are safe
under the delivery contracts; conflict responses and malformed responses are
terminal. `ReplicationRetryPolicy` allows at most eight total attempts and a
30-second backoff cap. This is an in-process request policy, not a durable
retry queue.
The fixed-term catalog listener does not terminate TLS, and this client does
not implement mutual TLS, streaming, backpressure, authority discovery, quorum,
or consensus. The optional Raft peer transport is separate.

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
- log retention: the local authority can compact through an exact retained snapshot without advancing the catalog transaction or losing the log suffix;
- follower safety: the authority accepts monotonic follower progress, journals it in `TXRG`, restores and validates it after restart, exposes the minimum acknowledged index, and permits compaction only through that index;
- transport: the catalog server exposes bounded status and contiguous entry-range reads, accepts versioned entry, snapshot, and progress JSON through HTTP routes with explicit conflict statuses, and `ReplicationHttpClient::catch_up` connects those routes to local ordered replay;
- authority capture: the default authority role journals `/transaction` and named-table mutations with `TXRP` state, while the follower role rejects direct catalog mutations;
- authentication: `TXBASE_REPLICATION_TOKEN` optionally protects the replication routes with RFC 6750 Bearer credentials;
- Raft mode: peer RPC requires that Bearer credential, uses HTTPS for non-loopback peer URLs, and checks cluster, sender, membership, and genesis-catalog identity;
- transport retry: `ReplicationHttpClient` retries bounded transient socket and HTTP failures, while exact replication POST duplicates remain safe and no durable retry queue is claimed;
- deterministic failure fixture: the CI test suite delivers the second entry before the first and then recovers.

The following contracts remain open:

- schema migrations independent of the catalog representation tag;
- TLS for the public catalog listener, mutual TLS, streaming, durable retry queues, backpressure, and authority discovery;
- broader delayed or reordered RPC schedules beyond one stale `AppendEntries` case; socket-level retries after losing a committed response; interrupted joint-membership recovery; reads during leadership changes; and crash-boundary injection;
- observability for lag and transport state;

Change data capture, persistent WAL history, and replication must share the same ordering contract.

## 6. Acceptance conditions

The initial local replication slice is complete because it has:

- one selected local authority model: fixed-term single writer;
- the versioned `ReplicationEntry` and `ReplicationLog` formats;
- the journaled `TXRP` sidecar with term and catalog-position checks, plus the validated `TXRG` follower-progress sidecar;
- the local historical follower-read boundary with applied-position checks;
- versioned snapshot export and atomic installation with stale, conflicting, and concurrent-change rejection;
- catalog-history compaction and `TXRP` base-position recovery after snapshot installation;
- deterministic replay, duplicate-delivery, conflict, and ordering tests;
- partition-gap, serialized-log recovery, term, and schema-tag tests;
- snapshot round-trip, installation, resume, stale-image, and conflict tests;
- retained snapshot export, suffix-preserving log compaction, sidecar-only journal recovery, and compaction conflict tests;
- follower watermark monotonicity, bounded `TXRG` progress-sidecar persistence, restart restoration, malformed-sidecar rejection, minimum-index compaction gating, snapshot cleanup, and bounded HTTP progress tests;
- bounded HTTP status, contiguous entry-range, entry delivery, duplicate delivery, snapshot installation, export, client parsing, authenticated requests, and one-shot catch-up tests;
- bounded transient HTTP retry and terminal conflict no-retry tests;
- default authority capture, table-ETag recheck, and follower read-only role tests;
- explicit write consistency: only the next catalog transaction can commit;
- a leader/follower fixture that fails and recovers without external infrastructure.

These acceptance conditions cover only the fixed-term slice.
Raft quorum writes and linearizable catalog reads have separate acceptance
boundaries in [Raft consensus design](raft.md).
The Raft CI tests cover learner transfer, joint-consensus voter changes, and a
three-node partition/failover/restart scenario, but not the additional
peer-TLS and failure cases documented in [Raft consensus design](raft.md).

## 7. Explicit non-goals

This document describes the fixed-term replication slice and does not define
the optional Raft protocol. It does not claim multi-region writes, global
transactions, automatic partition balancing, TLS for the public catalog
listener, mutual TLS, durable retry queues, or authority discovery.

Raft learner admission, joint-consensus voter changes, and the remaining
failure-testing work are specified separately. Other distributed features
need their own authority and recovery contracts.

## Primary references and scope

- [In Search of an Understandable Consensus Algorithm (Raft)](https://raft.github.io/raft.pdf)
- [Raft consensus algorithm](https://raft.github.io/)
- [RFC 9110: HTTP Semantics](https://www.rfc-editor.org/rfc/rfc9110.html)
- [OpenRaft 0.9.25 dynamic membership](https://docs.rs/openraft/0.9.25/openraft/docs/cluster_control/dynamic_membership/index.html)
- [Rustls platform verifier](https://github.com/rustls/rustls-platform-verifier)
- [Rustls `StreamOwned`](https://docs.rs/rustls/0.23.45/rustls/struct.StreamOwned.html)

The Raft paper defines the protocol; [Raft consensus design](raft.md)
records the txBASE decision, current implementation, and remaining work.

The repository has the fixed-term `TXRP` path and an optional Raft path
initialized from an explicit voter set. Raft mode provides authenticated peer
RPC, peer HTTPS, quorum writes, a linearizable read barrier, learner admission,
joint-consensus voter changes, and a three-node partition/failover/restart test.
The integration tests verify peer certificate trust and hostname matching.
The three-node failover test also delays one non-empty `AppendEntries` request past its RPC timeout before releasing it.
Broader delayed or reordered schedules and other failure scenarios remain open; the public catalog listener remains HTTP.
These boundaries are design constraints rather than compatibility guarantees.
