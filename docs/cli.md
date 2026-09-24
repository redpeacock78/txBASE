# CLI command reference

The CLI is a stable inspection and maintenance boundary over the Rust APIs.

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

The following table is the current command contract.

| Command | Current behavior and write boundary |
| --- | --- |
| `txbase read FILE [--encoding NAME]` | Prints active DBF records as JSON. Loading a path may complete pending WAL or schema-export recovery before reading. |
| `txbase init FILE --field NAME:TYPE:LENGTH[:DECIMALS]...` | Creates a new classic DBF from repeated field specifications and refuses to overwrite an existing DBF. |
| `txbase insert FILE JSON_OBJECT` | Appends one JSON object through the normal WAL-backed table persistence path. |
| `txbase schema FILE [--encoding NAME]` | Verifies the loaded DBF and prints its schema and record metadata as JSON. |
| `txbase schema apply FILE SCHEMA_JSON` | Validates a schema candidate against the current DBF and active records, then replaces only the schema sidecar. |
| `txbase verify FILE [--encoding NAME]` | Verifies the DBF and, when present, the index sidecar. It reparses the serialized DBF and checks record boundaries. |
| `txbase catalog DIRECTORY` | Prints the discovered catalog schema as JSON. |
| `txbase verify-catalog DIRECTORY` | Verifies the discovered catalog and prints `{"valid":true}` on success. |
| `txbase cdc FILE [--after TRANSACTION_ID]` | Prints committed single-table CDC events. `--after` is an exclusive transaction-ID cursor and does not acknowledge or retain consumer state. |
| `txbase cdc catalog DIRECTORY [--after TRANSACTION_ID]` | Prints atomic multi-table catalog CDC events with the same exclusive cursor rule. |
| `txbase mvcc list FILE` | Prints committed table snapshot IDs. |
| `txbase mvcc read FILE TRANSACTION_ID` | Reads one committed historical table snapshot. |
| `txbase mvcc row FILE RECORD` | Lists retained versions for one positive physical record number. |
| `txbase mvcc row-at FILE TRANSACTION_ID EPOCH RECORD` | Reads one retained row version by committed transaction, row epoch, and physical record number. |
| `txbase mvcc gc FILE --keep COUNT [--keep-rows COUNT]` | Retains the newest full-image snapshots and optionally older versions for each physical row. It replaces only the MVCC history sidecar. |
| `txbase mvcc catalog list DIRECTORY` | Prints committed catalog snapshot IDs. |
| `txbase mvcc catalog read DIRECTORY TRANSACTION_ID` | Reads one historical catalog snapshot and returns its tables and records. |
| `txbase mvcc catalog gc DIRECTORY --keep COUNT` | Retains the newest catalog snapshots and replaces only the catalog MVCC history sidecar. |
| `txbase wal inspect WAL` | Reads a WAL without creating or truncating it and reports complete records plus an incomplete final tail. |
| `txbase index build FILE FIELD...` | Builds and persists one scalar index sidecar for the named fields. |
| `txbase index build-compound FILE NAME FIELD[:DIRECTION]...` | Builds one named compound index. A direction is `1` or `asc` for ascending and `-1` or `desc` for descending order. |
| `txbase index verify FILE` | Validates an index sidecar and prints its schema as JSON. |
| `txbase index rebuild FILE` | Rebuilds and persists an index sidecar from the current table. |
| `txbase xbf import DBF XBF [--encoding NAME]` | Converts a DBF into a bounded XBF snapshot. |
| `txbase xbf export XBF DBF [--schema]` | Exports a representable XBF table. `--schema` preserves representable schema metadata through the recoverable export boundary. |
| `txbase xbf report XBF` | Reports DBF representability without writing a DBF or schema sidecar. |
| `txbase pack FILE [--encoding NAME]` | Removes logically deleted records, compacts referenced memo blocks, refreshes an existing index, and persists the related snapshots through WAL. |
| `txbase recall FILE RECORD [--encoding NAME]` | Restores one logically deleted record through the normal persistence boundary. |
| `txbase backup SOURCE DEST` | Validates and copies a DBF with its supported memo, schema, CDC, state, MVCC, and valid index sidecars. |
| `txbase restore SOURCE DEST` | Uses the same validated copy protocol with the backup as the source. |
| `txbase serve FILE [--bind ADDRESS] [--encoding NAME]` | Starts the single-table HTTP server. |
| `txbase serve-catalog DIRECTORY [--bind ADDRESS] [--replication-term TERM] [--replication-role authority|follower]` | Starts the catalog HTTP server and its bounded replication delivery routes. The default `authority` role captures `/transaction` and named-table mutation routes in the catalog journal and `TXRP` sidecar; `follower` rejects direct catalog mutations with `409` while accepting replication delivery. `TERM` is a positive fixed local replication term and defaults to `1`. When `TXBASE_REPLICATION_TOKEN` is set, the four replication routes require an RFC 6750 `Authorization: Bearer <token>` header. |

## Option ownership

- `--field` belongs only to `init` and may be repeated.
- `--encoding` belongs to path-loading commands that decode DBF text: `read`, `schema`, `verify`, `xbf import`, `pack`, `recall`, and `serve`.
- `--schema` belongs only to `xbf export`.
- `--bind` belongs only to `serve` and `serve-catalog`.
- `--replication-term` belongs only to `serve-catalog` and selects its positive fixed local replication term.
- `--replication-role` belongs only to `serve-catalog`; `authority` is the default write role, while `follower` rejects direct catalog mutations and accepts replication delivery.
- `TXBASE_REPLICATION_TOKEN` is an optional `serve-catalog` environment variable, not a CLI option; it protects the replication routes without exposing the token in the command line.
- `--after` belongs only to `cdc` and `cdc catalog`.
- `--keep` belongs to table and catalog MVCC garbage collection; `--keep-rows` belongs only to table MVCC garbage collection.
- `index build-compound` accepts `1` or `asc`, and `-1` or `desc`, for each field direction.

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

The single-table and catalog servers are long-running processes rather than one-shot inspection commands.

Their HTTP contracts are defined in [HTTP method semantics](http-semantics.md), [the query model](query-model.md), [the multi-table catalog](catalog.md), and [distributed evolution](distributed-evolution.md) for replication delivery.

## Responsibility groups

| Group | Commands | Boundary |
| --- | --- | --- |
| Read and inspect | `read`, `cdc`, `cdc catalog`, `schema`, `verify`, `catalog`, `verify-catalog`, `wal inspect`, `mvcc list`, `mvcc read`, `mvcc row`, `mvcc row-at`, `mvcc catalog list`, `mvcc catalog read`, `xbf report`, `index verify` | Read-only output; these commands do not intentionally publish a mutation. |
| Create and mutate | `init`, `insert`, `pack`, `recall`, `schema apply`, `mvcc gc`, `mvcc catalog gc`, `index build`, `index build-compound`, `index rebuild`, `xbf import`, `xbf export` | May write DBF bytes, sidecars, or durable history according to the command contract. |
| Copy and serve | `backup`, `restore`, `serve`, `serve-catalog` | Copy or expose data through a separately documented boundary. |

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
