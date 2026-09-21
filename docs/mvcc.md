# MVCC and historical snapshots

txBASE provides persistent, table-scoped MVCC snapshots for DBF mutations.

This document defines the current visibility contract and the remaining multi-table boundary.

## 1. Current contract

Each successful single-table `save_with_wal` mutation receives a positive transaction ID.

The DBF WAL remains the crash-recovery journal, and the `*.txbase.mvcc` sidecar retains the committed DBF image for that transaction.

The sidecar also retains the memo and schema images needed to decode that snapshot.

Only a snapshot with both a durable prepare record and a durable commit record is visible through the MVCC API.

The CLI exposes the committed versions and reads one exact historical snapshot:

```bash
txbase mvcc list path/to/users.dbf
txbase mvcc read path/to/users.dbf 2
```

`mvcc read` returns active records as JSON.

The requested ID must identify a committed snapshot.

Historical snapshots are read-only and cannot be saved as a new current state.

## 2. Commit and recovery

The writer holds the existing per-table lock.

It writes and syncs the DBF WAL, writes and syncs the MVCC prepare record, replaces the DBF and changed sidecars, persists the transaction state, and writes and syncs the MVCC commit record.

If the process stops before the commit record, the next normal DBF read replays the DBF WAL and completes the MVCC commit.

An old DBF WAL without an MVCC prepare record is upgraded during recovery by recording its recovered snapshot.

The table lock keeps the local writer boundary serial.

## 3. Isolation boundary

The current implementation is table-scoped snapshot visibility.

It does not provide row-level version storage, predicate locking, serializable conflict detection, or a long-lived transaction object across CLI calls.

The catalog transaction ID orders a multi-table commit, but the catalog does not yet expose a historical multi-table snapshot.

The low-level transaction engine in `src/transaction/` remains a separate WAL transaction primitive.

## 4. Roadmap

The next MVCC boundary is a catalog-wide snapshot that resolves a consistent version for every table in one catalog commit.

That work needs explicit rules for schema versions, missing table versions, catalog journal recovery, retention, and garbage collection.

Distributed snapshots, follower reads, and serializable conflict detection remain later work.

## Primary references

- [PostgreSQL transaction isolation](https://www.postgresql.org/docs/current/transaction-iso.html)
- [PostgreSQL concurrency control](https://www.postgresql.org/docs/current/mvcc.html)
- [SQLite isolation](https://sqlite.org/isolation.html)
- [SQLite write-ahead logging](https://sqlite.org/wal.html)
- [Git command-line interface conventions](https://git-scm.com/docs/gitcli)
