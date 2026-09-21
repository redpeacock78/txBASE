# MVCC and historical snapshots

txBASE provides persistent MVCC snapshots for DBF mutations and catalog transactions.

This document defines the current visibility contract and the remaining row-level boundary.

## 1. Current contract

Each successful single-table `save_with_wal` mutation receives a positive transaction ID.

The DBF WAL remains the crash-recovery journal, and the `*.txbase.mvcc` sidecar retains the committed DBF image for that transaction.

The sidecar also retains the memo and schema images needed to decode that snapshot.

Only a snapshot with both a durable prepare record and a durable commit record is visible through the MVCC API.

The CLI exposes the committed versions and reads one exact historical snapshot:

```bash
txbase mvcc list path/to/users.dbf
txbase mvcc read path/to/users.dbf 2
txbase mvcc gc path/to/users.dbf --keep 5
```

`mvcc read` returns active records as JSON.

The requested ID must identify a committed snapshot.

Historical snapshots are read-only and cannot be saved as a new current state.

`mvcc gc` retains the newest positive `--keep` count of committed snapshots and
rewrites only the MVCC history sidecar.

It takes the table lock, recovers a pending table WAL, writes the compacted history to a synced
temporary file, and replaces the old history file.

The current DBF, memo, schema, index, and transaction-state files are not changed.

An ID removed by GC is no longer readable, and a later commit appends after the retained IDs.

Catalog transactions retain a commit-level image of every discovered table:

```rust
use txbase::catalog::Catalog;

let versions = Catalog::mvcc_versions("database")?;
let catalog = Catalog::from_path_at("database", 2)?;
let users = catalog.open_table("users")?;
```

The catalog CLI exposes the same boundary:

```bash
txbase mvcc catalog list path/to/database
txbase mvcc catalog read path/to/database 2
txbase mvcc catalog gc path/to/database --keep 5
```

All tables opened from one historical catalog have the same catalog commit ID and are read-only.

## 2. Commit and recovery

The writer holds the existing per-table lock.

It writes and syncs the DBF WAL, writes and syncs the MVCC prepare record, replaces the DBF and changed sidecars, persists the transaction state, and writes and syncs the MVCC commit record.

If the process stops before the commit record, the next normal DBF read replays the DBF WAL and completes the MVCC commit.

An old DBF WAL without an MVCC prepare record is upgraded during recovery by recording its recovered snapshot.

The table lock keeps the local writer boundary serial.

The catalog transaction journal includes the catalog MVCC history file in the same prepared and
committed change set. A prepared journal rolls both the current table images and the new history
image back. A committed journal reapplies both before the next catalog read returns.

## 3. Isolation boundary

The current implementation provides table-scoped snapshot visibility for single-table commits and
commit-level snapshot visibility for catalog transactions.

Catalog history stores a full image of every discovered table per catalog commit. It does not
provide row-level version storage, retention or garbage collection, predicate locking, serializable
conflict detection, or a long-lived transaction object across CLI calls.

The catalog transaction ID identifies one consistent multi-table image.

`Catalog::gc_mvcc` and the catalog GC command retain the newest positive `keep_last` count of
catalog images.

Catalog GC takes the catalog write lock, recovers any prepared journal first, writes the retained
history to a synced temporary file, and replaces `.txbase.catalog.mvcc`.

It does not change DBF, memo, schema, index, or catalog transaction-state files.

The low-level transaction engine in `src/transaction/` remains a separate WAL transaction primitive.

## 4. Roadmap

The current retention boundary is count-based GC for full-image table and catalog snapshots.

The next MVCC boundary is row-level version storage with row-level retention and garbage
collection. That work must define schema-version selection, compaction, and the interaction
between row history and the existing full-image catalog commits.

Distributed snapshots, follower reads, and serializable conflict detection remain later work.

## Primary references

- [PostgreSQL transaction isolation](https://www.postgresql.org/docs/current/transaction-iso.html)
- [PostgreSQL concurrency control](https://www.postgresql.org/docs/current/mvcc.html)
- [SQLite isolation](https://sqlite.org/isolation.html)
- [SQLite write-ahead logging](https://sqlite.org/wal.html)
- [Git command-line interface conventions](https://git-scm.com/docs/gitcli)
