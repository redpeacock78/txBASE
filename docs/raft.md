# Raft consensus design

Status: `serve-catalog` has an optional OpenRaft mode with explicit initial voters, quorum writes, a linearizable read barrier, and an authenticated peer listener.
The peer API and `txbase raft membership` CLI add learners, report effective membership, and change voters through joint consensus; blank learners can join clusters with empty or non-empty genesis catalogs.
Raft integration tests cover quorum loss, leader replacement, log reconciliation, restart, membership recovery, and snapshot catch-up.
The five-node failover test runs the partition and recovery scenario under all 24 release-order permutations of four held non-empty `AppendEntries` requests, one to each peer.
After each delayed stale request is delivered, the target peer must retain catalog transaction 3 and the `Failover` record.
It also verifies that reads fail closed without a quorum and recover after connectivity returns.
The `/transaction` retry test sends real HTTP requests through a local TCP proxy, drops the first successful response after commit, and verifies an identical retry, one-time mutation, and `409` for a different payload at the same client sequence.
A child-process test now terminates the process hosting the three-node test cluster at four durability boundaries, restarts the same node directories, and verifies exact retry and one-time application.
RAFT-006 also delays two successive non-empty `AppendEntries` requests from one leader to one peer.
Each delay matches one of the next two expected log indices and stays armed across RPC retries until release, so membership traffic or a timed-out attempt cannot bypass it.
The test arms the second delay while the first request is paused, then releases the requests in sequence.
During this sequence, the current leader is the sole voter and every peer is a learner; other nodes are blocked from sending to the delayed target until catch-up.
The test restores the original voter set after every node catches up.
OpenRaft 0.9.25 runs one replication task per target and awaits each `append_entries` future, so this covers successive requests rather than overlapping calls from the same leader to that peer ([threading model](https://github.com/databendlabs/openraft/blob/v0.9.25/openraft/%73rc/docs/internal/threading.md); [replication implementation](https://github.com/databendlabs/openraft/blob/v0.9.25/openraft/%73rc/replication/mod.rs)).
In a later phase of the same five-node test, one voter is isolated while the remaining quorum commits two commands.
It delays the request for the second missing log index after verifying that the first command has reached the voter, then checks that each record is applied once after release.
The 24 orders exhaust release order only for the fixed four-request scenario; other request batches, terms, and partition conditions remain untested.
Without `--raft-*` options, `serve-catalog` keeps using the fixed-term replication path.

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
- A separate peer listener handles vote, append, snapshot, learner-preparation, learner-add, membership-status, and voter-change requests. It bounds requests to 2 MiB, applies a 10-second timeout, and checks bearer authentication, cluster and node identity, active membership, and a shared genesis-catalog fingerprint for Raft RPCs.
- Peer traffic may use HTTPS with a node certificate and key. `--raft-peer-client-ca` enables mTLS and requires HTTPS for every initial member. A node presents its peer certificate and key as its client identity when connecting to an mTLS peer; each peer verifies that identity against its configured CA. Outgoing clients verify server certificates and host names with the operating system's trust facilities. The public catalog listener has separate TLS and mTLS options; see [public catalog listener transport security](catalog-listener-security.md).
- `RaftMembershipHttpClient` and `txbase raft membership` provide authenticated status, learner-add, and voter-change operations. The client reuses the verified HTTP transport, supports an optional client certificate and key, and validates versioned response shapes and voter-set consistency.
- Raft writes to `/transaction` and named-table mutation routes require `X-Txbase-Client-Id` and a positive `X-Txbase-Client-Sequence`. Exact retries return the stored result.
- Token-free catalog reads call OpenRaft's linearizable read barrier before reading the local catalog. A read that supplies a minimum read token waits for the local applied index to reach that token instead.
- A three-node CI test exercises quorum commit and retry deduplication, learner catch-up before promotion, joint voter promotion and demotion, retained-learner shutdown, and quorum writes after demotion.
- A three-node `/transaction` test sends requests through a local TCP proxy that drops the first successful HTTP response after commit; an exact retry returns identical JSON and transaction ID without a duplicate mutation, while a different payload at the same client sequence returns `409`.
- A child-process test terminates the process hosting three logical nodes after log-entry persistence, commit-marker persistence, atomic catalog and applied-state publication, or receipt of the OpenRaft application response. It restarts all three node directories, retries the same client request, and verifies one catalog mutation at every node.
- A two-node CI test verifies that an empty-catalog learner can join a cluster with an empty genesis catalog.
- A three-node CI test verifies that a blank learner receives the non-empty genesis catalog and a committed update before it joins as a learner.
- A three-node CI test isolates one voter, commits four commands on the remaining quorum, snapshots and purges the leader log, then verifies that the voter installs the snapshot and applies the next client sequence after reconnecting.
- Peer-RPC HTTPS tests accept a trusted server certificate, reject untrusted or wrong-host server certificates, require a trusted client certificate when mTLS is enabled, and reject a missing client certificate.
- The Raft peer-listener integration test verifies that a trusted client can reach the membership route while missing and untrusted client certificates fail during TLS negotiation.
- `RaftLogStore` durably stores votes, log entries, committed position, and the last purged log ID in a node-specific directory.
- The log journal uses length-prefixed, SHA-256-checked JSON records, recovers an incomplete tail, and compacts purged history into a new generation.
- The node directory has an exclusive process lock, and the storage tests include OpenRaft's `testing::Suite` plus restart-recovery cases.

### Not implemented

- Broader deterministic coverage for delayed or reordered RPC schedules beyond the fixed four-request release permutations, the sequential same-peer pair, and one two-command voter catch-up with its second request delayed; varied terms and partition conditions remain untested.
- Follower reads that guarantee the latest quorum-committed state; read tokens provide session monotonicity only.

Commands allow client IDs of up to 128 ASCII bytes, require a positive sequence and a non-empty catalog tag, and accept 1–1,000 transaction steps with at least one mutation.
The serialized command limit is 1 MiB.
The application state and complete snapshot are each limited to 64 MiB.
Client retry records are retained indefinitely; reaching the state limit fails closed until a client-retirement protocol is defined.

An empty catalog can initialize through `RaftCatalogStateMachine::open`.
An empty learner can join a cluster with either an empty or non-empty genesis catalog.
For a non-empty genesis catalog, the leader first verifies that the candidate has no catalog data, snapshot, applied Raft state, membership, client state, or durable Raft log.
The candidate then adopts the cluster fingerprint and receives the leader's current OpenRaft snapshot before OpenRaft registers it as a learner.
Normal log replication catches it up after the transferred snapshot.
Candidates with existing state must already match the cluster's genesis fingerprint.
A non-empty catalog requires `--raft-bootstrap` on the initial node or `--raft-initialize-catalog` on a prepared peer, plus a committed catalog snapshot; a non-empty catalog at transaction ID 0 is rejected because it has no transferable MVCC image.
Every initial voter must start from the same catalog image. Learners may start blank and adopt the cluster image only through the checked join flow.
Opening a catalog with data but no Raft state sidecar fails closed.
The current reader accepts `TXRA` state version 2 and does not migrate version 1 sidecars automatically.

## 1. Purpose and boundary

In Raft mode, one catalog leader accepts mutations and commits each mutation only after a quorum has durably stored its log entry.

The replicated state machine remains the catalog. Raft supplies leadership and an ordered committed log; it does not replace `OperationIr`, `TransactionStep`, or the catalog journal.

The current `ReplicationLog` is not a Raft log. It has one configured term, ties its index to catalog transaction IDs, and applies entries directly through the catalog journal. Raft also logs membership changes and other protocol entries, so its log position must remain distinct from the catalog transaction ID.

This authority design does not provide distributed transactions across catalogs, partitioning, or linearizable reads from arbitrary followers. Those require separate contracts.
The applied-index token read contract is described in [Client writes and reads](#5-client-writes-and-reads).

## 2. Protocol implementation

Use the Raft protocol described by Ongaro and Ousterhout and integrate OpenRaft `=0.9.25` rather than implementing elections, log matching, quorum commitment, or membership safety locally. Enable `single-term-leader` for the standard one-leader-per-term mode, `serde` for the versioned RPC and storage data, and `storage-v2` for the current separated log and state-machine storage interfaces.

OpenRaft provides the protocol engine, dynamic-membership operations, and a conformance suite for application storage. Its `storage-v2` feature is explicitly temporary in this release, so the exact dependency and feature set must remain pinned until an upgrade migrates the storage adapter.

The selected OpenRaft release documents its API as unstable before version 1.0. Pin the exact dependency and lockfile version; upgrade it only with the storage suite and the multi-node failure tests in this document.

The server creates a Tokio multi-thread runtime for OpenRaft. The state machine sends catalog filesystem work to `tokio::task::spawn_blocking`, and the synchronous peer HTTP client runs in Tokio's blocking pool. A write response is returned only after OpenRaft commits the command and the state machine durably applies it.

## 3. Node identity and membership

Each node has a stable positive node ID, a client address, a peer-RPC address, and a dedicated persistent data directory. One process at a time may own a node data directory. The directory records the cluster and node identity and must be separate from the catalog directory.

Cluster initialization is an explicit one-time operation over a declared initial voter set. Pass the same `--raft-initial-member ID=URL` set to every initial node and use `--raft-bootstrap` on exactly one node. Restarting a node must never initialize or replace cluster membership implicitly.

An authenticated cluster operator can add a learner by sending `POST /raft/v1/learner` to the current leader's peer listener. The JSON body contains `version`, `cluster_id`, `node_id`, and `peer_address`; the request also requires the `TXBASE_REPLICATION_TOKEN` bearer credential.
The endpoint returns `202` after the asynchronous join workflow starts, not after the learner catches up. The leader calls the candidate's `/raft/v1/learner/prepare` route, transfers a snapshot through the normal snapshot RPC, and then registers it with OpenRaft.
The joining node's `--raft-initial-member` map must include its own address and the existing peer that sends its first RPC, so the node can authenticate that peer before learning the committed membership.
If the current membership already contains the same node ID and address, the endpoint returns `200`; conflicting IDs or addresses are rejected.

An empty learner can join clusters with either an empty or non-empty genesis catalog.
For a non-empty genesis catalog, the candidate adopts the cluster fingerprint only if its catalog and Raft log are pristine and its Raft state is uninitialized.
The leader installs a current snapshot before registering the candidate, so normal log replication starts after the catalog image and applied position are in place.
Existing candidates must already have the cluster fingerprint.

Use authenticated `GET /raft/v1/membership` to inspect a node's local effective membership. The response includes the node and leader IDs, server state, effective membership log index, voter configurations, voter and learner IDs, node addresses and roles, and whether a membership change is in progress. It reads local OpenRaft metrics rather than a linearizable cluster-wide view; a joint configuration appears as multiple voter sets.

A voter change follows four steps:

1. Read the effective voter set and membership log index from `GET /raft/v1/membership`.
2. Send authenticated `POST /raft/v1/membership` to the current leader with `version`, `cluster_id`, `expected_membership_log_index`, `expected_voter_ids`, and the desired `voter_ids`.
   For a non-no-op change from a stable configuration, the expected index and voter IDs act as a compare-and-swap guard.
   A target set that is already stable returns `200`; an accepted change returns `202`.
3. The handler waits for each learner being promoted to catch up with OpenRaft's blocking `add_learner` operation, then calls `change_membership`.
   OpenRaft commits the joint configuration and then its uniform target configuration.
   txBASE uses `retain=true`, so voters removed from the target set become learners and remain cluster members; their metadata is not erased and replication is not stopped.
4. Poll `GET /raft/v1/membership` until `effective_voter_configs` contains one set and `membership_change_in_progress` is false.

Stale requests, unknown nodes, requests sent to a non-leader, and conflicting joint changes return `409`.
While a joint configuration is effective, a request for its target voter set resumes the change.
If the leader stops before the uniform configuration commits, the surviving voters retain the joint configuration; after electing a leader, resubmit the same target voter set to resume the change.
Editing local configuration alone cannot change voter authority.

The CLI exposes `raft membership status`, `add-learner`, and `change-voters` without opening a local catalog directory.
All three commands require `TXBASE_REPLICATION_TOKEN`; the shared HTTP client refuses to send it over non-loopback HTTP and verifies HTTPS certificates and host names.
Status may be read from any peer and reflects that node's local metrics.
Learner addition and voter changes must target the current leader.
The voter-change command requires the membership log index and voter IDs reported by a preceding status request, then sends them as compare-and-swap preconditions.
The startup CLI still rejects duplicate node IDs, duplicate peer URLs, missing local membership, conflicting replication modes, conflicting node identities, and attempts to run two nodes against one data directory.

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

Token-free catalog reads call `Raft::ensure_linearizable()` before reading the local applied state. If the node cannot satisfy the barrier, the server returns `503` instead of serving a possibly stale read.

Current-state read responses and Raft mutation responses include `X-TXBASE-Raft-Read-Token` when an applied Raft position is available. A client can send that value as `X-TXBASE-Raft-Min-Read-Token` on a later current-state read. The node waits, for at most the configured Raft RPC timeout, until its local applied index reaches the token's index, then reads locally and returns its current token. If it does not catch up in time, the server returns `503 raft_unavailable`.

Tokens use the form `v1.<cluster_id>.<applied_index>`. A malformed token returns `400`; a token for another cluster returns `409`. A token cannot be combined with the historical `?at=` parameter. Tokens are consistency metadata, not credentials.

This provides session-monotonic reads when a client carries the token forward: a later read will not move behind the applied index observed by an earlier response. It does not guarantee the latest quorum-committed state or make a token-bearing read linearizable. Reads without a token retain the existing linearizable barrier.

## 6. Peer transport and security

Peer RPC uses a version 1 internal envelope for voting, log replication, and snapshot installation at `/raft/v1/vote`, `/raft/v1/append`, and `/raft/v1/snapshot`. The authenticated `POST /raft/v1/learner/prepare`, `POST /raft/v1/learner`, and `GET`/`POST /raft/v1/membership` control routes use the same peer listener and bearer token.
The preparation route accepts a known initial member before the candidate has a committed membership. It permits a fingerprint change only for a pristine, uninitialized candidate.
The learner workflow sends a chunked snapshot through the normal snapshot route before adding the candidate to membership; OpenRaft then replicates later log entries.
The listener is separate from the public catalog listener, caps each request at 2 MiB, applies a 10-second RPC timeout, and checks Raft RPC senders against the active membership, cluster ID, and shared genesis fingerprint. It also checks that the OpenRaft vote identifies the same sender.

Every Raft peer request requires the `TXBASE_REPLICATION_TOKEN` bearer credential. Non-loopback peer URLs must use HTTPS with `--raft-peer-cert` and `--raft-peer-key`; the client verifies the server certificate and host name with the operating system's trust facilities. Plain HTTP with a bearer token is allowed only for loopback URLs.

Pass `--raft-peer-client-ca PEM` to require every HTTPS peer client to present a certificate issued by that CA. All initial-member URLs must use HTTPS when this option is enabled. Each node presents its `--raft-peer-cert` and `--raft-peer-key` pair as its outgoing client identity, so the certificate must be valid for both server and client authentication. The client CA controls inbound peer identity verification; clients continue to use the operating system's trust facilities to verify peer server certificates.

Raft peer mTLS is independent of public catalog mTLS. `--raft-peer-client-ca` applies only to the peer listener; public catalog clients use `--tls-client-ca` as described in [public catalog listener transport security](catalog-listener-security.md). The `txbase raft membership` commands can present a separate client identity with `--tls-client-cert` and `--tls-client-key`.

Rustls [`WebPkiClientVerifier`](https://docs.rs/rustls/0.23.45/rustls/server/struct.WebPkiClientVerifier.html) requires and validates client certificates when configured with trusted roots. [`ConfigBuilder::with_client_auth_cert`](https://docs.rs/rustls/0.23.45/rustls/struct.ConfigBuilder.html#method.with_client_auth_cert) configures the client identity.

## 7. Snapshots, recovery, and migration

A Raft snapshot contains the catalog MVCC image, the last-applied Raft log ID, the effective membership, and the application deduplication state.
The current adapter stores application state in `.txbase.raft-state` and the versioned snapshot in `.txbase.raft-snapshot`.
Snapshot payloads use the `TXRF` version 1 header and embed the `TXRA` version 2 application state.
Installation publishes the catalog image and both sidecars together.

The existing `ReplicationSnapshot` contains catalog replication position but no Raft membership or committed Raft log ID. It is not a complete Raft snapshot and must not be installed as one.

Startup validates the node identity and durable Raft state, restores the snapshot, and replays committed entries in order before the node serves reads or writes. A node whose catalog image and applied position disagree fails closed and requires recovery rather than choosing one copy silently.

Migration from a fixed-term `TXRP` authority is manual. Stop the old writers, choose and validate one authoritative catalog image, back it up, and prepare every initial Raft voter from that same image. Bootstrap one node with the declared initial membership and initialize the other nodes' Raft state explicitly. The peer API can add a learner that already has the matching genesis identity or safely initialize a blank learner from a non-empty cluster snapshot. It must not infer a voter set or promote an old follower log automatically.

## 8. Verification and acceptance

The state-machine CI tests cover catalog commit atomicity, restart-safe retries, sequence rejection, no-op and membership entries, and snapshot installation.
The storage adapter's tests run OpenRaft's `testing::Suite` and restart-recovery checks.
The membership integration test exercises a quorum commit, blank-learner snapshot transfer, promotion and demotion, and quorum writes after a voter is demoted.
The five-node failover integration test repeats the same partition and recovery scenario for all 24 release-order permutations of the four held non-empty `AppendEntries` requests, one to each peer.
After each held request is delivered, it verifies that the target peer retains transaction 3 and the `Failover` record.
Each run isolates the current leader from all four peers, verifies that its write does not reach the catalog and its read barrier fails, commits the next client sequence on the remaining quorum, selects a replacement only after its read barrier succeeds, releases the held requests after that commit, heals the partition, and restarts the isolated node before checking catalog and membership convergence.
It sends a client-facing `GET /catalog` to the isolated former leader and verifies `503 raft_unavailable`.
After observing the replacement election, it blocks RPC between the remaining voters and verifies that `GET /catalog` on the reported leader also fails closed with `503`; after restoring connectivity, it waits for a successful read barrier and verifies `200` from the elected leader.
The three-node `/transaction` retry test uses a local TCP proxy to discard the successful HTTP response after commit.
It verifies that an exact retry returns the same JSON result and transaction ID, applies the mutation once, and rejects a different request at the same client sequence with `409`.
The two-node test continues to cover blank-learner joining when the genesis catalog is empty.
It also exercises the typed membership client for status, learner addition, promotion, idempotent retry, and demotion; CLI argument tests cover command routing and voter-ID validation.
The crash-recovery test terminates its child process after a normal log entry is synced but before the in-memory log changes, after a commit marker is synced but before the in-memory committed position changes, after one catalog-journal commit publishes the mutation with its applied position and client result, or after OpenRaft returns the application result but before the `/transaction` handler constructs its HTTP response.
For each point, the parent restarts all three node directories, retries the same client ID and sequence, and checks that every catalog reaches transaction 2 with exactly one mutation.
The three logical nodes share the child process, so this test does not model an independent process crash for a single voter.
The failover test also delays two successive non-empty `AppendEntries` requests from the current leader to one peer and releases them in sequence.
Each delay matches one command's log index and stays armed across RPC retries until release, so membership traffic or a timed-out attempt cannot bypass it.
The test blocks every other sender to the target while it holds the two requests, so another replication stream cannot satisfy the catch-up check.
After all nodes catch up, it restores the original voter set.
OpenRaft 0.9.25 runs one replication task per target and awaits each `append_entries` future, so the test does not claim to hold overlapping calls from the same leader to that peer.
The same five-node run then isolates one voter while the other four commit two commands.
It holds the second command's `AppendEntries` request after the first command has applied at that voter; every node must apply both records once and converge at transaction 7.
These cases do not cover larger catch-up batches, differing terms, or other partition conditions.

## Primary references and scope

- [In Search of an Understandable Consensus Algorithm (Raft)](https://raft.github.io/raft.pdf) defines the consensus protocol and joint-consensus membership change.
- [OpenRaft 0.9.25 documentation](https://docs.rs/openraft/0.9.25/openraft/) documents the selected implementation and its pre-1.0 API status.
- [OpenRaft feature flags](https://docs.rs/openraft/0.9.25/openraft/docs/feature_flags/) documents standard Raft mode and the temporary `storage-v2` API.
- [OpenRaft `RaftLogStorage`](https://docs.rs/openraft/0.9.25/openraft/storage/trait.RaftLogStorage.html) defines durable log-store behavior.
- [OpenRaft `RaftStateMachine`](https://docs.rs/openraft/0.9.25/openraft/storage/trait.RaftStateMachine.html) defines applied-state, entry application, and snapshot behavior.
- [OpenRaft read operations](https://docs.rs/openraft/0.9.25/openraft/docs/protocol/read/index.html) define the quorum-confirmed read index and the applied-index condition used for linearizable reads.
- [OpenRaft `Raft::wait`](https://docs.rs/openraft/0.9.25/openraft/raft/struct.Raft.html#method.wait) defines bounded waits for Raft metrics, including the applied index.
- [OpenRaft getting started and storage test suite](https://docs.rs/openraft/0.9.25/openraft/docs/getting_started/) defines the application storage and network adapters and points to `testing::Suite`.
- [OpenRaft cluster formation](https://docs.rs/openraft/0.9.25/openraft/docs/cluster_control/cluster_formation/) defines the one-time `Raft::initialize()` operation.
- [OpenRaft network traits](https://docs.rs/openraft/0.9.25/openraft/network/) define the peer RPC adapter.
- [OpenRaft replication tasks](https://docs.rs/openraft/0.9.25/openraft/docs/internal/threading/index.html) document one replication task per target node.
- [OpenRaft dynamic membership](https://docs.rs/openraft/0.9.25/openraft/docs/cluster_control/dynamic_membership/) defines learner catch-up and voter changes; [`Raft::change_membership`](https://docs.rs/openraft/0.9.25/openraft/raft/struct.Raft.html#method.change_membership) documents that a leader loss or crash before the uniform configuration commits leaves the joint configuration active.
- [OpenRaft snapshot replication](https://docs.rs/openraft/0.9.25/openraft/docs/protocol/replication/snapshot_replication/) documents chunked snapshot transfer.
- [OpenRaft `Raft::install_snapshot`](https://docs.rs/openraft/0.9.25/openraft/raft/struct.Raft.html#method.install_snapshot) defines the snapshot-install RPC used by the learner join flow.

The Raft paper is the protocol source. OpenRaft documentation is the source for the selected library's API and adapter requirements. txBASE owns the catalog command format, durable storage layout, HTTP behavior, migration procedure, and compatibility guarantees.
