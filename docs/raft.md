# Raft consensus design

Status: `serve-catalog` has an optional OpenRaft mode with explicit initial voters, quorum writes, a linearizable read barrier, and a separate authenticated peer listener. The peer API can add a prepared node as a learner and start log replication. Joint voter changes, membership-status commands, and failure-injection coverage remain outstanding. Without `--raft-*` options, `serve-catalog` keeps using the fixed-term replication path.

This document records the implemented Raft boundary and the remaining authority, recovery, and operations work.

## Current implementation status

### Implemented

- OpenRaft `=0.9.25` is pinned, and the repository defines `TypeConfig`, version 2 client commands, and responses.
- `RaftCatalogStateMachine` applies committed commands and persists catalog changes, the applied Raft position, and client retry results in one catalog journal commit.
- Blank entries, membership entries, and rejected commands persist their applied position without advancing the catalog transaction ID.
- The state machine stores the latest response per client and enforces exact retry, conflict, old-sequence, and gap behavior.
- Snapshot build, transfer, and install carry the catalog image, applied position, membership, and client retry state together.
- Catalog filesystem work runs through Tokio's blocking worker pool.
- `serve-catalog` starts an OpenRaft node from an explicit node ID, cluster ID, node directory, peer address, and initial member map. Only `--raft-bootstrap` initializes cluster membership.
- A separate peer listener handles vote, append, snapshot, and authenticated learner-add requests. It bounds requests to 2 MiB, applies a 10-second timeout, and checks bearer authentication, cluster and node identity, active membership, and a shared genesis-catalog fingerprint for Raft RPCs.
- Peer traffic may use HTTPS with a node certificate and key. Bearer-authenticated HTTP is accepted only for loopback peer URLs. The public catalog listener remains HTTP.
- Raft writes to `/transaction` and named-table mutation routes require `X-Txbase-Client-Id` and a positive `X-Txbase-Client-Sequence`. Exact retries return the stored result.
- Normal catalog reads call OpenRaft's linearizable read barrier before reading the local catalog. The server does not provide an explicitly stale follower-read mode.
- A three-node CI test exercises quorum commit, retry deduplication, authenticated learner addition after a commit, learner catch-up, and catalog convergence.
- `RaftLogStore` durably stores votes, log entries, committed position, and the last purged log ID in a node-specific directory.
- The log journal uses length-prefixed, SHA-256-checked JSON records, recovers an incomplete tail, and compacts purged history into a new generation.
- The node directory has an exclusive process lock, and the storage tests include OpenRaft's `testing::Suite` plus restart-recovery cases.

### Not implemented

- Voter promotion or removal through joint membership, and membership/status CLI commands.
- Joining from an empty catalog. A learner must currently be prepared from the exact committed genesis image used by the cluster.
- Deterministic tests for quorum loss, partitions, message loss or reordering, leader changes, restart recovery, and reads during leadership changes.
- Dedicated peer HTTPS certificate and host-verification integration tests.
- Mutual TLS and TLS for the public catalog listener.

Commands allow client IDs of up to 128 ASCII bytes, require a positive sequence and a non-empty catalog tag, and accept 1–1,000 transaction steps with at least one mutation.
The serialized command limit is 1 MiB.
The application state and complete snapshot are each limited to 64 MiB.
Client retry records are retained indefinitely; reaching the state limit fails closed until a client-retirement protocol is defined.

An empty catalog can initialize through `RaftCatalogStateMachine::open`.
A non-empty catalog requires `--raft-bootstrap` on the initial node or `--raft-initialize-catalog` on a prepared peer, plus a committed catalog snapshot; a non-empty catalog at transaction ID 0 is rejected because it has no transferable MVCC image.
Every initial voter and learner must start from the same catalog image. Peer RPC rejects a different genesis fingerprint.
Prepare a learner from the committed catalog snapshot and initialize its Raft catalog state with `--raft-initialize-catalog` before adding it.
Opening a catalog with data but no Raft state sidecar fails closed.
The current reader accepts `TXRA` state version 2 and does not migrate version 1 sidecars automatically.

## 1. Purpose and boundary

In Raft mode, one catalog leader accepts mutations and commits each mutation only after a quorum has durably stored its log entry.

The replicated state machine remains the catalog. Raft supplies leadership and an ordered committed log; it does not replace `OperationIr`, `TransactionStep`, or the catalog journal.

The current `ReplicationLog` is not a Raft log. It has one configured term, ties its index to catalog transaction IDs, and applies entries directly through the catalog journal. Raft also logs membership changes and other protocol entries, so its log position must remain distinct from the catalog transaction ID.

This authority design does not provide distributed transactions across catalogs, partitioning, or linearizable reads from arbitrary followers. Those require separate contracts.

## 2. Protocol implementation

Use the Raft protocol described by Ongaro and Ousterhout and integrate OpenRaft `=0.9.25` rather than implementing elections, log matching, quorum commitment, or membership safety locally. Enable `single-term-leader` for the standard one-leader-per-term mode, `serde` for the versioned RPC and storage data, and `storage-v2` for the current separated log and state-machine storage interfaces.

OpenRaft provides the protocol engine, dynamic-membership operations, and a conformance suite for application storage. Its `storage-v2` feature is explicitly temporary in this release, so the exact dependency and feature set must remain pinned until an upgrade migrates the storage adapter.

The selected OpenRaft release documents its API as unstable before version 1.0. Pin the exact dependency and lockfile version; upgrade it only with the storage suite and the multi-node failure tests in this document.

The server creates a Tokio multi-thread runtime for OpenRaft. The state machine sends catalog filesystem work to `tokio::task::spawn_blocking`, and the synchronous peer HTTP client runs in Tokio's blocking pool. A write response is returned only after OpenRaft commits the command and the state machine durably applies it.

## 3. Node identity and membership

Each node has a stable positive node ID, a client address, a peer-RPC address, and a dedicated persistent data directory. One process at a time may own a node data directory. The directory records the cluster and node identity and must be separate from the catalog directory.

Cluster initialization is an explicit one-time operation over a declared initial voter set. Pass the same `--raft-initial-member ID=URL` set to every initial node and use `--raft-bootstrap` on exactly one node. Restarting a node must never initialize or replace cluster membership implicitly.

An authenticated cluster operator can add a prepared learner by sending `POST /raft/v1/learner` to the current leader's peer listener. The JSON body contains `version`, `cluster_id`, `node_id`, and `peer_address`; the request also requires the `TXBASE_REPLICATION_TOKEN` bearer credential.
The endpoint returns `202` after OpenRaft starts replication, not after the learner catches up. The joining node's `--raft-initial-member` map must include its own address and the existing peer that sends its first RPC, so the node can authenticate that peer before learning the committed membership.
If the current membership already contains the same node ID and address, the endpoint returns `200`; conflicting IDs or addresses are rejected.

The learner must be prepared from the cluster's exact committed genesis image and initialized with `--raft-initialize-catalog`. A blank catalog cannot adopt a cluster fingerprint yet.
The API does not promote or remove voters. A new voter must first catch up as a learner, then the cluster must commit the change through OpenRaft's joint-membership procedure. Editing local configuration alone cannot change voter authority.

The startup CLI rejects duplicate node IDs, duplicate peer URLs, missing local membership, conflicting replication modes, conflicting node identities, and attempts to run two nodes against one data directory. Voter changes and membership-status commands remain future work.

## 4. Durable log and catalog application

`RaftLogStore` persists votes, log entries, the committed position, and the last purged log ID. `RaftCatalogStateMachine` persists membership, the applied position, catalog state, and retry results. An append callback completes only after the journal has been synchronized to disk.

Entries at or below the purged index are discarded from an append batch; the store persists only the remaining suffix and treats a fully purged batch as a no-op. When an append overlaps retained logs, `RaftLogStore` replaces the suffix beginning at the first retained incoming index. It rejects gaps and any replacement at or below the committed index.

The journal stores length-prefixed JSON records with SHA-256 checksums. Startup truncates an incomplete final frame, but rejects a complete frame with a bad checksum or invalid log sequence. Purge writes a checkpoint into a new journal generation before removing the old generation.

`RaftLogStore::open` holds an exclusive lock for its node directory, so a second process cannot open the same store. Startup opens the durable log store and state machine before serving requests.

The existing `FileWal` is not a drop-in Raft store: its LSN starts at zero, and its public maintenance operation clears the entire log rather than truncating a conflicting suffix or purging a snapshot-covered prefix. Reuse its framing and recovery code only if the storage contract is extended without changing the existing `TXWL` format.

Raft application entries carry bounded catalog mutation steps, the expected catalog schema tag, request preconditions, a stable client ID, and a monotonically increasing client sequence. The implemented state machine applies only entries passed to OpenRaft's apply interface.
It checks the expected catalog transaction ID and schema tag under the catalog write lock.
It publishes a successful catalog mutation, its last-applied Raft position, and the retry result in one journal commit.

A deterministic command rejection, including a stale catalog tag, failed request precondition, or invalid transaction, advances the applied Raft position and records its stable response without advancing the catalog transaction ID.
Membership entries and protocol no-ops also advance the Raft position without pretending to be catalog transactions.

Each client may have one outstanding sequence. The state machine persists the latest sequence, request fingerprint, and response for each client.
An exact retry returns the stored response; a conflicting retry, an older sequence, or a sequence gap is rejected without reapplying the mutation.
Client IDs are not reused, and this state is included in snapshots.

The applied-position marker and request result must be durable with the catalog mutation. After a crash, replay resumes after that marker; it must not apply a committed mutation twice or report an uncommitted mutation as successful.

The existing `TXRP` sidecar remains a pre-consensus replication format. It cannot serve as the Raft WAL because it stores one fixed term and assumes every entry advances the catalog transaction sequence.

## 5. Client writes and reads

A mutation succeeds only after Raft commits it on a quorum and the local state machine durably applies it. Without a quorum, the server returns an unavailable or not-leader response and does not fall back to a local write.

Every Raft mutation includes a bounded client ID and positive sequence. Clients serialize writes per ID and retry an uncertain request with the same ID, sequence, body, and precondition headers. The state machine retains the latest sequence and response for each client; a client must receive or resolve one sequence before advancing to the next.

Normal catalog reads call `Raft::ensure_linearizable()` before reading the local applied state. If the node cannot satisfy the barrier, the server returns `503` instead of serving a possibly stale read. The current HTTP mode does not redirect reads to a leader or expose a stale follower-read option.

## 6. Peer transport and security

Peer RPC uses a version 1 internal envelope for voting, log replication, and snapshot installation at `/raft/v1/vote`, `/raft/v1/append`, and `/raft/v1/snapshot`. The authenticated `POST /raft/v1/learner` control route uses the same peer listener and bearer token. The listener is separate from the public catalog listener, caps each request at 2 MiB, applies a 10-second RPC timeout, and checks Raft RPC senders against the active membership, cluster ID, and shared genesis fingerprint. It also checks that the OpenRaft vote identifies the same sender.

Every Raft peer request requires the `TXBASE_REPLICATION_TOKEN` bearer credential. Non-loopback peer URLs must use HTTPS with `--raft-peer-cert` and `--raft-peer-key`; the client verifies the certificate and host name with the operating system's trust facilities. Plain HTTP with a bearer token is allowed only for loopback URLs. This TLS configuration applies only to the peer listener; the public catalog listener still uses HTTP.

## 7. Snapshots, recovery, and migration

A Raft snapshot contains the catalog MVCC image, the last-applied Raft log ID, the effective membership, and the application deduplication state.
The current adapter stores application state in `.txbase.raft-state` and the versioned snapshot in `.txbase.raft-snapshot`.
Snapshot payloads use the `TXRF` version 1 header and embed the `TXRA` version 2 application state.
Installation publishes the catalog image and both sidecars together.

The existing `ReplicationSnapshot` contains catalog replication position but no Raft membership or committed Raft log ID. It is not a complete Raft snapshot and must not be installed as one.

Startup validates the node identity and durable Raft state, restores the snapshot, and replays committed entries in order before the node serves reads or writes. A node whose catalog image and applied position disagree fails closed and requires recovery rather than choosing one copy silently.

Migration from a fixed-term `TXRP` authority is manual. Stop the old writers, choose and validate one authoritative catalog image, back it up, and prepare every initial Raft voter from that same image. Bootstrap one node with the declared initial membership and initialize the other nodes' Raft state explicitly. The peer API can add a prepared learner, but the CLI cannot join an empty node or initialize one from a cluster snapshot. It must not infer a voter set or promote an old follower log automatically.

## 8. Verification and acceptance

The state-machine CI tests cover catalog commit atomicity, restart-safe retries, sequence rejection, no-op and membership entries, and snapshot installation.
The storage adapter's tests run OpenRaft's `testing::Suite` and restart-recovery checks.
The three-node integration test exercises a quorum commit on two initial voters, authenticated learner addition, log catch-up, a repeated client command, and catalog convergence.
It does not yet inject deterministic network delay, message loss, partitions, reordering, restarts, or leader changes.

Acceptance requires tests for durable term and vote recovery, conflicting log replacement, quorum loss, leader change, client retry after a lost response, apply-marker recovery, snapshot installation and suffix retention, learner catch-up from a purged log through snapshot transfer, joint membership changes, and linearizable reads during leadership changes.

Crash injection must cover each boundary between log persistence, quorum commitment, catalog journal publication, applied-position persistence, and client response. A green single-node test or an in-memory protocol test does not establish these guarantees.

## Primary references and scope

- [In Search of an Understandable Consensus Algorithm (Raft)](https://raft.github.io/raft.pdf) defines the consensus protocol and joint-consensus membership change.
- [OpenRaft 0.9.25 documentation](https://docs.rs/openraft/0.9.25/openraft/) documents the selected implementation and its pre-1.0 API status.
- [OpenRaft feature flags](https://docs.rs/openraft/0.9.25/openraft/docs/feature_flags/) documents standard Raft mode and the temporary `storage-v2` API.
- [OpenRaft `RaftLogStorage`](https://docs.rs/openraft/0.9.25/openraft/storage/trait.RaftLogStorage.html) defines durable log-store behavior.
- [OpenRaft `RaftStateMachine`](https://docs.rs/openraft/0.9.25/openraft/storage/trait.RaftStateMachine.html) defines applied-state, entry application, and snapshot behavior.
- [OpenRaft getting started and storage test suite](https://docs.rs/openraft/0.9.25/openraft/docs/getting_started/) defines the application storage and network adapters and points to `testing::Suite`.
- [OpenRaft cluster formation](https://docs.rs/openraft/0.9.25/openraft/docs/cluster_control/cluster_formation/) defines the one-time `Raft::initialize()` operation.
- [OpenRaft network traits](https://docs.rs/openraft/0.9.25/openraft/network/) define the peer RPC adapter.
- [OpenRaft dynamic membership](https://docs.rs/openraft/0.9.25/openraft/docs/cluster_control/dynamic_membership/) defines learner catch-up and voter changes.

The Raft paper is the protocol source. OpenRaft documentation is the source for the selected library's API and adapter requirements. txBASE owns the catalog command format, durable storage layout, HTTP behavior, migration procedure, and compatibility guarantees.
