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
txbase mvcc row path/to/users.dbf 1
txbase mvcc row-at path/to/users.dbf 2 1 1
txbase mvcc gc path/to/users.dbf --keep 5
```

`mvcc read` returns active records as JSON.

`mvcc row` lists retained versions for one physical DBF record number.

`mvcc row-at` reads one retained row version by transaction ID, epoch, and physical record number.

Both commands return JSON and use the same committed-history and GC boundaries as the Rust API.

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

The catalog HTTP server accepts that commit ID as `?at=<transaction ID>` on catalog schema,
named-table records, query, stream, explain, and join reads.

One request reads one exact catalog image.

Historical HTTP queries and joins do not reuse current index sidecars, and catalog mutations with
`at` are rejected.

The Rust `Catalog::begin_read` API captures every discovered current table into one in-memory,
read-only image before releasing the catalog and table read locks.
Its `CatalogReadTransaction` keeps the captured catalog commit ID and supports table reads and the
bounded join contract without reading later filesystem changes.
It is a process-local read view, not a durable retention pin; `Catalog::from_path_at` remains the
API for reopening a retained catalog commit.

### Row-level history

The table MVCC sidecar also stores row changes inside the same prepare and commit records.

The public API exposes the retained row history and one row at a committed table snapshot:

```rust
use txbase::dbf::{DbfTable, RowId};

let history = DbfTable::mvcc_row_versions("users.dbf", 1)?;
let row = DbfTable::mvcc_read_row(
    "users.dbf",
    2,
    RowId {
        epoch: history[0].id.epoch,
        record_number: 1,
    },
)?;
```

The row number is the physical DBF record number, not a user-defined primary key.

An epoch separates row identities after `PACK`, a schema or record-layout change, or another explicit layout reset.

A logical delete is stored as a row version with `deleted: true`, so a historical read can distinguish a deleted row from a row that never existed in the retained history.

Row changes are committed only when the containing full-image MVCC record has both its prepare and commit records.

MVCC GC rebuilds the first retained snapshot as a row-history baseline and recomputes later deltas, so retained row reads do not depend on removed transactions.

Catalog MVCC continues to store complete table images per catalog commit.

Catalog historical reads therefore retain their existing commit-level contract and do not claim to expose table-local row history for a catalog snapshot.

The public [`DbfTransaction`](transactions.md) API provides an optimistic private snapshot for one path-backed table.

It applies operations and queries against the private copy, then publishes one WAL-backed commit or discards the copy.

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

Catalog history stores a full image of every discovered table per catalog commit.

Table-local row history now has count-based retention through the existing full-image MVCC GC.

The public table transaction provides optimistic stale-source rejection by default.
The explicit `DbfTransaction::commit_with_row_merge` API can merge disjoint physical-row updates or
deletes under the table lock when schema, layout, and record count are unchanged.
Inserts and different changes to the same row remain errors; an identical resulting row is a no-op.
Neither table transaction API provides predicate locking, serializable conflict detection, or a
long-lived transaction object across CLI calls.

The catalog transaction ID identifies one consistent multi-table image and can select that image
for one catalog HTTP request or identify a `CatalogReadTransaction` capture.

The HTTP selector does not create a long-lived transaction object.
The Rust read transaction is stable after capture, but it does not provide predicate locking or
serializable conflict detection.

`Catalog::gc_mvcc` and the catalog GC command retain the newest positive `keep_last` count of
catalog images.

Catalog GC takes the catalog write lock, recovers any prepared journal first, writes the retained
history to a synced temporary file, and replaces `.txbase.catalog.mvcc`.

It does not change DBF, memo, schema, index, or catalog transaction-state files.

The low-level transaction engine in `src/transaction/` remains a separate WAL transaction primitive.

## 4. Roadmap

The current retention boundary is count-based GC for full-image table and catalog snapshots.

The current row-level boundary is physical-record history with epoch-separated identities and count-based compaction.

An independent row-retention policy, schema migration history, predicate locking, and serializable conflict detection remain future work.

Distributed snapshots, follower reads, and serializable conflict detection remain later work.

## Primary references

- [PostgreSQL transaction isolation](https://www.postgresql.org/docs/current/transaction-iso.html)
- [PostgreSQL concurrency control](https://www.postgresql.org/docs/current/mvcc.html)
- [SQLite isolation](https://sqlite.org/isolation.html)
- [SQLite write-ahead logging](https://sqlite.org/wal.html)
