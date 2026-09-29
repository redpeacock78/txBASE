# CLI command reference

The CLI uses the Rust APIs to inspect and mutate txBASE data and start its servers.

The design decisions behind the command surface are recorded in [CLI command architecture](cli-design.md).

It uses explicit subcommands so a command's read, write, recovery, or server responsibility is visible before a path is opened.

## Command shape

The top-level form is:

```text
txbase COMMAND [SUBCOMMAND] ARGUMENT...
```

The first token selects one responsibility. Nested subcommands are used when a family has its own lifecycle, such as `mvcc`, `wal`, `xbf`, and `index`.

Paths and JSON values are positional where the operation has one unambiguous target.
Options are named when they change interpretation, such as `--encoding`, `--schema`, `--bind`, `--keep`, or `--keep-rows`.

## Invocation contract

With no command, `-h`, or `--help`, txBASE prints the root usage text and exits successfully.

An unknown top-level command or option is an error.

Each command rejects missing required arguments and unexpected trailing arguments unless its contract explicitly accepts repeated arguments or options.

The current CLI has no global `--` terminator, version command, configuration file, or command-specific help mode.

The parser accepts only the options documented for the selected command.

## Command catalog

The tables group the current command contract by responsibility.

### Read and inspect files

| Command | Current behavior and write boundary |
| --- | --- |
| `txbase read FILE [--encoding NAME]` | Prints active DBF records as JSON. Loading a path may complete pending WAL or schema-export recovery before reading. |
| `txbase schema FILE [--encoding NAME]` | Verifies the loaded DBF and prints its schema and record metadata as JSON. |
| `txbase verify FILE [--encoding NAME]` | Verifies the DBF and, when present, the index sidecar. It reparses the serialized DBF and checks record boundaries. |
| `txbase catalog DIRECTORY` | Prints the discovered catalog schema as JSON. |
| `txbase verify-catalog DIRECTORY` | Verifies the discovered catalog and prints `{"valid":true}` on success. |

### Create files and update schemas

| Command | Current behavior and write boundary |
| --- | --- |
| `txbase init FILE --field NAME:TYPE:LENGTH[:DECIMALS]...` | Creates a new classic DBF from repeated field specifications and refuses to overwrite an existing DBF. |
| `txbase insert FILE JSON_OBJECT` | Appends one JSON object through the normal WAL-backed table persistence path. |
| `txbase schema apply FILE SCHEMA_JSON` | Validates a schema candidate against the current DBF and active records, then replaces only the schema sidecar. |

### Change history and WAL

| Command | Current behavior and write boundary |
| --- | --- |
| `txbase cdc FILE [--after TRANSACTION_ID]` | Prints committed single-table CDC events. `--after` is an exclusive transaction-ID cursor and does not acknowledge or retain consumer state. |
| `txbase cdc catalog DIRECTORY [--after TRANSACTION_ID]` | Prints atomic multi-table catalog CDC events with the same exclusive cursor rule. |
| `txbase wal inspect WAL` | Reads a WAL without creating or truncating it and reports complete records plus an incomplete final tail. |

### Table MVCC history

| Command | Current behavior and write boundary |
| --- | --- |
| `txbase mvcc list FILE` | Prints committed table snapshot IDs. |
| `txbase mvcc read FILE TRANSACTION_ID` | Reads one committed historical table snapshot. |
| `txbase mvcc row FILE RECORD` | Lists retained versions for one positive physical record number. |
| `txbase mvcc row-at FILE TRANSACTION_ID EPOCH RECORD` | Reads one retained row version by committed transaction, row epoch, and physical record number. |
| `txbase mvcc gc FILE --keep COUNT [--keep-rows COUNT]` | Retains the newest full-image snapshots and optionally older versions for each physical row. It replaces only the MVCC history sidecar. |

### Catalog MVCC history

| Command | Current behavior and write boundary |
| --- | --- |
| `txbase mvcc catalog list DIRECTORY` | Prints committed catalog snapshot IDs. |
| `txbase mvcc catalog read DIRECTORY TRANSACTION_ID` | Reads one historical catalog snapshot and returns its tables and records. |
| `txbase mvcc catalog gc DIRECTORY --keep COUNT` | Retains the newest catalog snapshots and replaces only the catalog MVCC history sidecar. |

### Indexes

| Command | Current behavior and write boundary |
| --- | --- |
| `txbase index build FILE FIELD... [--collation NAME]` | Builds and persists one scalar index sidecar for the named fields. `NAME` may be `unicode-lowercase`, `unicode-nfkc-lowercase`, or an ICU4X 2.1.1 Japanese, Chinese, or Korean identifier for ordered queries. |
| `txbase index build-compound FILE NAME FIELD[:DIRECTION]... [--collation NAME]` | Builds one named compound index. A direction is `1` or `asc` for ascending and `-1` or `desc` for descending order. `NAME` accepts the same five collations. |
| `txbase index verify FILE` | Validates an index sidecar and prints its schema as JSON. |
| `txbase index rebuild FILE` | Rebuilds and persists an index sidecar from the current table. |

### XBF conversion

| Command | Current behavior and write boundary |
| --- | --- |
| `txbase xbf import DBF XBF [--encoding NAME]` | Converts a DBF into a bounded XBF snapshot. |
| `txbase xbf export XBF DBF [--schema]` | Exports a representable XBF table. `--schema` preserves representable schema metadata through the recoverable export boundary. |
| `txbase xbf report XBF` | Reports DBF representability without writing a DBF or schema sidecar. |

### DBF maintenance and copying

| Command | Current behavior and write boundary |
| --- | --- |
| `txbase pack FILE [--encoding NAME]` | Removes logically deleted records, compacts referenced memo blocks, refreshes an existing index, and persists the related snapshots through WAL. |
| `txbase recall FILE RECORD [--encoding NAME]` | Restores one logically deleted record through the normal persistence boundary. |
| `txbase backup SOURCE DEST` | Validates and copies a DBF with its supported memo, schema, CDC, state, MVCC, and valid index sidecars. |
| `txbase restore SOURCE DEST` | Uses the same validated copy protocol with the backup as the source. |

### Servers and replication

| Command | Current behavior and write boundary |
| --- | --- |
| `txbase serve FILE [--bind ADDRESS] [--encoding NAME]` | Starts the single-table HTTP server. |
| `txbase serve-catalog DIRECTORY [--bind ADDRESS] [--tls-cert PEM --tls-key PEM [--tls-client-ca PEM]] [--replication-term TERM] [--replication-role authority|follower]` | Starts the catalog HTTP server in fixed-term replication mode. The default `authority` role captures `/transaction` and named-table mutation routes in the catalog journal and `TXRP` sidecar, and persists follower progress in the metadata-only `TXRG` sidecar; `follower` rejects direct catalog mutations and follower-progress acknowledgements with `409` while accepting replication delivery. `TERM` is a positive fixed local replication term and defaults to `1`. When `TXBASE_REPLICATION_TOKEN` is set, all replication routes require an RFC 6750 `Authorization: Bearer <token>` header. TLS is optional; `--tls-client-ca` requires client certificates. See [public catalog listener security](catalog-listener-security.md). |
| `txbase serve-catalog DIRECTORY --raft-node-id ID --raft-cluster-id ID --raft-data-directory DIR --raft-peer-bind ADDRESS --raft-peer-advertise URL --raft-initial-member ID=URL ... [--tls-cert PEM --tls-key PEM [--tls-client-ca PEM]]` | Starts optional OpenRaft mode with a statically configured voter set and a separate peer listener. Raft flags cannot be combined with `--replication-*`; public listener TLS is independent of peer TLS. See [public catalog listener security](catalog-listener-security.md) and the bootstrap and peer TLS options below. |
| `txbase replicate catch-up DIRECTORY AUTHORITY_URL --replication-term TERM --follower-id ID [--limit COUNT] [--timeout-ms MILLISECONDS]` | Opens a follower catalog, pulls one bounded catch-up session from an authority, persists the applied catalog and `TXRP` position, acknowledges progress, and prints the synchronization result as JSON. `AUTHORITY_URL` accepts HTTP or HTTPS; HTTPS verifies the authority certificate and host name with the operating system's trust facilities. `TERM` must match the authority, `COUNT` is between `1` and `128`, and the optional `TXBASE_REPLICATION_TOKEN` environment variable supplies the Bearer credential. Bearer credentials require HTTPS except for loopback HTTP. |
| `txbase raft membership status PEER_URL [--timeout-ms MILLISECONDS]` | Reads that node's local effective membership as JSON. It does not open a catalog directory and is not a linearizable cluster-wide read. |
| `txbase raft membership add-learner PEER_URL --cluster-id ID --node-id ID --peer-address URL [--timeout-ms MILLISECONDS]` | Requests learner addition through the current leader. `202` means replication started, not that catch-up finished; `200` means the node is already a member. |
| `txbase raft membership change-voters PEER_URL --cluster-id ID --expected-index INDEX --expected-voter-ids ID,... --voter-ids ID,... [--timeout-ms MILLISECONDS]` | Requests a joint-consensus voter change through the current leader. Supply the membership index and voter IDs from a preceding status result as compare-and-swap preconditions. `202` means the change was accepted; poll status until one voter configuration remains and no change is in progress. Demoted voters remain learners. |

## Option ownership

### File and server options

- `--field` belongs only to `init` and may be repeated.
- `--encoding` belongs to path-loading commands that decode DBF text: `read`, `schema`, `verify`, `xbf import`, `pack`, `recall`, and `serve`.
- `--schema` belongs only to `xbf export`.
- `--bind` belongs only to `serve` and `serve-catalog`.
- `--tls-cert`, `--tls-key`, and `--tls-client-ca` belong only to `serve-catalog`. The certificate and key must be specified together; a client CA requires client certificates. The built-in `replicate catch-up` client cannot present a client certificate.

### CDC and MVCC options

- `--after` belongs only to `cdc` and `cdc catalog`.
- `--keep` belongs to table and catalog MVCC garbage collection.
- `--keep-rows` belongs only to table MVCC garbage collection.

### Replication options

- `--replication-term` belongs to `serve-catalog` and `replicate catch-up`; it selects the positive fixed local term for either operation.
- `--replication-role` belongs only to `serve-catalog`; `authority` is the default write role, while `follower` rejects direct catalog mutations and progress acknowledgements and accepts replication delivery.
- `--follower-id` belongs only to `replicate catch-up` and identifies the local follower session.
- `--limit` belongs only to `replicate catch-up`. `--timeout-ms` belongs to `replicate catch-up` and `raft membership`; it bounds one pull session or peer request and its socket operations.
- `TXBASE_REPLICATION_TOKEN` is an environment variable, not a CLI option. It is optional for fixed-term replication routes, required for every Raft peer RPC and every `raft membership` command, and read by `replicate catch-up` when configured. The client refuses to send it over non-loopback HTTP.

### Raft options

- `--raft-node-id`, `--raft-cluster-id`, `--raft-data-directory`, `--raft-peer-bind`, and `--raft-peer-advertise` are required when any `--raft-*` option selects Raft mode. The node directory must be separate from the catalog directory, and the advertised URL must match this node's initial-member entry.
- Repeat `--raft-initial-member ID=URL` for every voter and use the same set on every initial node. Exactly one node uses `--raft-bootstrap`.
- For a populated catalog, use `--raft-bootstrap` on the first node and `--raft-initialize-catalog` on each prepared peer so they start from the same catalog image.
- `--raft-peer-cert` and `--raft-peer-key` are both required when the advertised URL uses HTTPS and must be omitted for HTTP. Peer clients verify the certificate and host name with the operating system's trust facilities.
- Raft mode requires `TXBASE_REPLICATION_TOKEN` for authenticated peer RPC. `raft membership add-learner` and `change-voters` target the current leader; `status` can target any peer.
- `--cluster-id` and `--node-id` belong only to `raft membership add-learner`; `--peer-address` supplies the joining node's advertised peer URL.
- `--expected-index`, `--expected-voter-ids`, and `--voter-ids` belong only to `raft membership change-voters`. ID lists are comma-separated positive integers without duplicates; the expected values come from `raft membership status`.

### Index options

- `index build-compound` accepts `1` or `asc`, and `-1` or `desc`, for each field direction.
- `index build` and `index build-compound` accept the two Unicode key modes and `--collation icu4x-2.1.1-ja`, `--collation icu4x-2.1.1-zh`, or `--collation icu4x-2.1.1-ko` for ordered query support.

Positive transaction IDs, epochs, record numbers, and retention counts are required where the command names them as positive values.

## Read, recovery, and mutation boundaries

Read-oriented commands do not intentionally publish user mutations, but loading a live table runs the normal recovery boundary.

That boundary may consume a pending table WAL or schema-export journal before the command reads the table.

`wal inspect` is the explicit exception: it only reports complete records and a torn final tail, and never creates or truncates the WAL.

Mutation commands reuse the lock and recovery path owned by the DBF, catalog, or XBF operation.

`schema apply` changes only the schema sidecar after validation.

`mvcc gc` changes only the relevant history sidecar after validation.

`index build`, `index rebuild`, and `xbf export --schema` write derived or metadata sidecars in addition to their documented target files.

`backup` and `restore` replace destination files one by one through synced temporary files, so an interrupted copy is not a new multi-file transaction protocol.

Run `txbase verify DEST` before using a destination after an interrupted copy.

`replicate catch-up` is a mutating one-shot operation. It may install a snapshot,
apply a prefix of the requested entry pages, and write the follower's `TXRP`
sidecar before returning an error. Re-running the same command resumes from the
persisted position and acknowledges the resulting progress.

The single-table and catalog servers are long-running processes rather than one-shot inspection commands.

Their HTTP contracts are defined in [HTTP method semantics](http-semantics.md), [the query model](query-model.md), [the multi-table catalog](catalog.md), and [distributed evolution](distributed-evolution.md) for replication delivery.

`schema apply` is deliberately separate from `schema`: `schema` inspects the current metadata, while `schema apply` validates and installs a candidate sidecar.

`cdc` is a read-only event inspection command, not a mutation or consumer acknowledgement command.

Its optional `--after` argument is an exclusive transaction-ID cursor over the committed events in the table's CDC sidecar.

`cdc catalog DIRECTORY` reads the atomic events emitted by multi-table catalog transactions from the catalog CDC sidecar.

The catalog form uses the same exclusive `--after` cursor over catalog transaction IDs.

`mvcc gc FILE --keep COUNT` retains the newest full-image table snapshots.
Adding `--keep-rows COUNT` also retains up to `COUNT` older versions for each physical row before the oldest retained full snapshot.
The row history is stored in the same MVCC sidecar; the current DBF and full snapshots are not changed by GC.

`pack` removes deleted physical records, renumbers survivors, compacts referenced DBT/FPT memo blocks, refreshes an existing index sidecar, and persists the DBF, memo, index, MVCC, transaction-state, and CDC changes through one WAL-backed boundary.
`recall` restores one deleted record after schema validation through the normal persistence boundary.

## Scope

This document defines the txBASE CLI surface and command contract.
Database consistency, DBF compatibility, XBF recovery, MVCC retention, and HTTP semantics remain in their topic documents.
