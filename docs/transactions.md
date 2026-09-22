# Snapshot transactions

`txbase::dbf::DbfTransaction` is the public optimistic transaction boundary for one path-backed DBF table.

It loads one table snapshot, applies operations to a private copy, and publishes all successful mutations through one WAL-backed save.

This document defines the current API boundary and its deliberate isolation limits.

## 1. Lifecycle

### Begin

`DbfTransaction::begin(path)` loads the current DBF table and records the source state used for the later commit check.

The transaction owns that table copy until it is committed or rolled back.

`DbfTransaction::begin_serializable(path)` selects the coarse-grained serializable boundary.
It acquires the table's exclusive lock before loading the table and holds that lock until commit, rollback, or drop.
Other local txBASE DBF readers and writers that honor the same lock wait while the transaction is active.
The boundary applies to one DBF path and does not lock a catalog or coordinate external writers that bypass txBASE.

### Apply

`apply(&OperationIr)` accepts the same operation representation used by the HTTP transaction route.

The operation changes only the private table copy until `commit` succeeds.

### Query

`query(&QueryRequest)` and the `QueryExecutor` implementation evaluate a query against the private snapshot.

Uncommitted changes are therefore visible to the transaction caller but not to a separate table load.

### Commit

`commit()` saves the private table through the existing table lock, WAL, MVCC, sidecar, and transaction-state persistence path.

The result is one table commit and the returned `DbfTable` contains its durable transaction ID.

When the transaction was opened with `begin_serializable`, `commit()` reuses the lock already held by the transaction instead of acquiring it again.

If the DBF, memo, schema, or transaction-state source changed after `begin`, the commit is rejected instead of overwriting the newer state.

`commit_with_row_merge()` is an explicit opt-in for a narrower conflict contract.
Under the table lock, it reloads the current image and applies only this transaction's updates or
deletes to physical records where the current image has no different change.
An identical resulting row is treated as a no-op.
It rejects inserts, different changes to the same row, record-count changes, layout changes, and schema changes.
The merged result uses the same WAL, MVCC, sidecar, and transaction-state persistence path.

### Rollback

`rollback()` drops the private copy without writing the table path.

Dropping a transaction without calling `commit` has the same persistence effect.

## 2. Visibility and conflict handling

The transaction is optimistic.

It does not hold a table lock from `begin` through `commit`.

The commit-time source check is the write-write conflict boundary for independently loaded table snapshots.

With the default `commit()`, the caller must discard the failed transaction, reload the current table, and decide whether to retry the operations.

When the application contract is limited to disjoint physical-row updates or deletes, the caller may
explicitly choose `commit_with_row_merge()` instead.

The API does not select the merge policy automatically and does not promise exactly-once effects across a lost network response.

## 3. Relationship to other transaction boundaries

The single-table HTTP `POST /transaction` route reuses this apply-and-commit boundary.

The catalog `POST /transaction` route remains a separate cross-table journal boundary because it coordinates DBF and sidecar images under the catalog lock.

The `src/transaction/` engine remains a lower-level WAL transaction primitive and does not provide DBF visibility or this table API.

Historical table and catalog snapshots remain read-only MVCC views.

For a stable cross-table read image, `Catalog::begin_read` returns a
`CatalogReadTransaction` that captures all discovered tables before releasing its locks.
That API is read-only and non-durable; the catalog MVCC API remains the boundary for reopening a
retained commit.

## 4. Isolation boundary

The default API provides a private snapshot for one DBF table and optimistic stale-source rejection at commit.

`begin_serializable` provides strict serial execution for one table by excluding concurrent txBASE access from begin through commit or rollback.
This is intentionally a table-wide lock, so it does not provide predicate-level concurrency or a cross-table serializable transaction.

`CatalogReadTransaction` provides a separate in-memory snapshot boundary for cross-table reads and
bounded joins.

The default API does not provide predicate locking or predicate-level serializable conflict detection.
`commit_with_row_merge()` is limited to explicit physical-row update and delete merging and does not
provide predicate or serializable semantics.

Predicate-level locking, cross-table serializable validation, independent catalog retention, and long-lived distributed transactions remain future work.

## 5. Example

```rust
use txbase::dbf::DbfTransaction;
use txbase::query::{parse, QueryExecutor};

let mut transaction = DbfTransaction::begin("users.dbf")?;
transaction.apply(&operation)?;
let rows = transaction.execute(&parse(br#"{}"#)?)?;
let committed_table = transaction.commit()?;
```

Use `commit_with_row_merge()` only when that narrower physical-row conflict contract is sufficient.

The HTTP route and this Rust API share the same table mutation and persistence path, so a behavior change must update both the API tests and the HTTP contract tests.

## Primary references

- [PostgreSQL transaction isolation](https://www.postgresql.org/docs/current/transaction-iso.html)
- [PostgreSQL concurrency control](https://www.postgresql.org/docs/current/mvcc.html)
- [SQLite isolation](https://sqlite.org/isolation.html)
- [SQLite write-ahead logging](https://sqlite.org/wal.html)
- [MVCC and historical snapshots](mvcc.md)
- [Mutation model](mutation-model.md)
- [HTTP method semantics and QUERY](http-semantics.md)
