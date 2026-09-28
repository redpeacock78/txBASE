# Raft consensus design

Status: Raft is the selected consensus protocol. The repository does not yet integrate a Raft runtime; the current catalog server remains a fixed-term, single-authority implementation.

This document defines the target authority, persistence, application, and operations contracts. It does not describe features as implemented.

## Current implementation status

The repository pins OpenRaft `=0.9.25` and defines a `TypeConfig`, version 1 client commands, and client responses.
The command validator bounds client IDs to 128 ASCII bytes, requires a positive sequence and non-empty catalog tag, accepts 1–1,000 transaction steps with at least one mutation, validates request preconditions, and caps serialized commands at 1 MiB.
These types are not connected to `serve-catalog`; durable Raft log storage, a catalog state-machine adapter, snapshots, and peer transport remain unimplemented.
The current catalog server therefore remains a fixed-term, single-authority implementation.

## 1. Purpose and boundary

Raft will make one catalog leader at a time accept mutations and will commit each mutation only after a quorum has durably stored its log entry.

The replicated state machine remains the catalog. Raft supplies leadership and an ordered committed log; it does not replace `OperationIr`, `TransactionStep`, or the catalog journal.

The current `ReplicationLog` is not a Raft log. It has one configured term, ties its index to catalog transaction IDs, and applies entries directly through the catalog journal. Raft also logs membership changes and other protocol entries, so its log position must remain distinct from the catalog transaction ID.

This authority design does not provide distributed transactions across catalogs, partitioning, or linearizable reads from arbitrary followers. Those require separate contracts.

## 2. Protocol implementation

Use the Raft protocol described by Ongaro and Ousterhout and integrate OpenRaft `=0.9.25` rather than implementing elections, log matching, quorum commitment, or membership safety locally. Enable `single-term-leader` for the standard one-leader-per-term mode, `serde` for the versioned RPC and storage data, and `storage-v2` for the current separated log and state-machine storage interfaces.

OpenRaft provides the protocol engine, dynamic-membership operations, and a conformance suite for application storage. Its `storage-v2` feature is explicitly temporary in this release, so the exact dependency and feature set must remain pinned until an upgrade migrates the storage adapter.

The selected OpenRaft release documents its API as unstable before version 1.0. Pin the exact dependency and lockfile version; upgrade it only with the storage suite and the multi-node failure tests in this document.

Use OpenRaft's Tokio runtime for protocol tasks. Catalog and log filesystem operations must not block the runtime's executor threads, and an RPC success must not be returned before the required durable write completes.

## 3. Node identity and membership

Each node has a stable positive node ID, a client address, a peer-RPC address, and a dedicated persistent data directory. One process at a time may own a node data directory.

Cluster initialization is an explicit one-time operation over a declared initial voter set. Restarting a node must never initialize or replace cluster membership implicitly.

Adding a voter first adds the node as a learner and catches it up. The cluster then commits a membership change using OpenRaft's joint-membership procedure. Removing or replacing a voter uses the same committed membership path; editing local configuration alone cannot change voter authority.

The operational interface must expose explicit initialization, join, membership-change, and status operations. It must reject duplicate node IDs, conflicting cluster identities, and attempts to run two nodes against one data directory.

## 4. Durable log and catalog application

The OpenRaft storage adapter must durably store votes, log entries, membership state, snapshots, and the committed and applied positions. A successful append or vote response means the corresponding state survives process restart. The log format must detect torn or corrupt records and recover only a validated prefix.

The existing `FileWal` is not a drop-in Raft store: its LSN starts at zero, and its public maintenance operation clears the entire log rather than truncating a conflicting suffix or purging a snapshot-covered prefix. Reuse its framing and recovery code only if the storage contract is extended without changing the existing `TXWL` format.

Raft application entries carry bounded catalog mutation steps, the expected catalog representation, request preconditions, a stable client ID, and a monotonically increasing client sequence. The state machine applies only committed entries and uses the catalog journal to atomically publish a successful catalog mutation with its last-applied Raft position and retry result.

A rejected precondition still advances the applied Raft position and records its stable response without advancing the catalog transaction ID. Membership entries and protocol no-ops also advance Raft position without pretending to be catalog transactions.

Each client may have one outstanding sequence. The state machine persists the latest sequence and response for each client: an exact retry returns the stored response, an older sequence is rejected without reapplication, and a sequence gap is rejected. Client IDs are not reused, and this state is included in snapshots.

The applied-position marker and request result must be durable with the catalog mutation. After a crash, replay resumes after that marker; it must not apply a committed mutation twice or report an uncommitted mutation as successful.

The existing `TXRP` sidecar remains a pre-consensus replication format. It cannot serve as the Raft WAL because it stores one fixed term and assumes every entry advances the catalog transaction sequence.

## 5. Client writes and reads

A mutation succeeds only after Raft commits it on a quorum and the local state machine durably applies it. Without a quorum, the server returns an unavailable or not-leader response and does not fall back to a local write.

Every retryable mutation includes a bounded client ID and sequence. Clients serialize writes per ID; the state machine persists the latest sequence and response so a client can safely retry after losing the response to a committed write.

Normal catalog reads require a linearizable quorum-confirmed read or a leader read barrier before reading the local applied state. A follower may serve an explicitly stale or historical read, but the response must expose its applied position and must not claim linearizability.

## 6. Peer transport and security

Peer RPC uses a versioned internal protocol for voting, log replication, and snapshot installation. The peer listener is separate from the public catalog listener, bounds request sizes and timeouts, and identifies the sending node against the active membership.

Peer traffic must be authenticated and encrypted when it crosses a host boundary. A bearer token over plaintext is not sufficient because it exposes credentials and catalog mutations to network observers. The existing Rustls dependencies can support the transport, but the current HTTP replication routes do not provide Raft RPCs or native server-side TLS.

## 7. Snapshots, recovery, and migration

A Raft snapshot contains the catalog image, the last-applied Raft log ID, the effective membership, and the application deduplication state. Installation publishes these together before the receiver discards the covered log prefix.

The existing `ReplicationSnapshot` contains catalog replication position but no Raft membership or committed Raft log ID. It is not a complete Raft snapshot and must not be installed as one.

Startup validates the node identity and durable Raft state, restores the snapshot, and replays committed entries in order before the node serves reads or writes. A node whose catalog image and applied position disagree fails closed and requires recovery rather than choosing one copy silently.

Migration from a fixed-term `TXRP` authority is explicit. Stop the old writers, choose and validate one authoritative catalog image, back it up, initialize a Raft cluster from that image, and install the resulting snapshot on other nodes. The system must not infer a voter set or promote an old follower log automatically.

## 8. Verification and acceptance

The storage adapter must pass OpenRaft's `testing::Suite` before it is used by the server. CI must also exercise multiple real Raft nodes with deterministic network delay, message loss, partitions, reordering, and restart points.

Acceptance requires tests for durable term and vote recovery, conflicting log replacement, quorum loss, leader change, client retry after a lost response, apply-marker recovery, snapshot installation and suffix retention, learner catch-up, joint membership changes, and linearizable reads during leadership changes.

Crash injection must cover each boundary between log persistence, quorum commitment, catalog journal publication, applied-position persistence, and client response. A green single-node test or an in-memory protocol test does not establish these guarantees.

## Primary references and scope

- [In Search of an Understandable Consensus Algorithm (Raft)](https://raft.github.io/raft.pdf) defines the consensus protocol and joint-consensus membership change.
- [OpenRaft 0.9.25 documentation](https://docs.rs/openraft/0.9.25/openraft/) documents the selected implementation and its pre-1.0 API status.
- [OpenRaft feature flags](https://docs.rs/openraft/0.9.25/openraft/docs/feature_flags/) documents standard Raft mode and the temporary `storage-v2` API.
- [OpenRaft getting started and storage test suite](https://docs.rs/openraft/0.9.25/openraft/docs/getting_started/) defines the application storage and network adapters and points to `testing::Suite`.
- [OpenRaft dynamic membership](https://docs.rs/openraft/0.9.25/openraft/docs/cluster_control/dynamic_membership/) defines learner catch-up and voter changes.

The Raft paper is the protocol source. OpenRaft documentation is the source for the selected library's API and adapter requirements. txBASE owns the catalog command format, durable storage layout, HTTP behavior, migration procedure, and compatibility guarantees.
