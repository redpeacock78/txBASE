# Snapshot transactions

`txbase::dbf::DbfTransaction` is the public optimistic transaction boundary for one path-backed DBF table.

It loads one table snapshot, applies operations to a private copy, and publishes all successful mutations through one WAL-backed save.

This document defines the current API boundary and its deliberate isolation limits.

## 1. Lifecycle

### Begin

`DbfTransaction::begin(path)` loads the current DBF table and records the source state used for the later commit check.

The transaction owns that table copy until it is committed or rolled back.

### Apply

`apply(&OperationIr)` accepts the same operation representation used by the HTTP transaction route.

The operation changes only the private table copy until `commit` succeeds.

### Query

`query(&QueryRequest)` and the `QueryExecutor` implementation evaluate a query against the private snapshot.

Uncommitted changes are therefore visible to the transaction caller but not to a separate table load.

### Commit

`commit()` saves the private table through the existing table lock, WAL, MVCC, sidecar, and transaction-state persistence path.

The result is one table commit and the returned `DbfTable` contains its durable transaction ID.

If the DBF, memo, schema, or transaction-state source changed after `begin`, the commit is rejected instead of overwriting the newer state.

### Rollback

`rollback()` drops the private copy without writing the table path.

Dropping a transaction without calling `commit` has the same persistence effect.

## 2. Visibility and conflict handling

The transaction is optimistic.

It does not hold a table lock from `begin` through `commit`.

The commit-time source check is the write-write conflict boundary for independently loaded table snapshots.

On a conflict, the caller must discard the failed transaction, reload the current table, and decide whether to retry the operations.

The API does not merge concurrent operations automatically and does not promise exactly-once effects across a lost network response.

## 3. Relationship to other transaction boundaries

The single-table HTTP `POST /transaction` route reuses this apply-and-commit boundary.

The catalog `POST /transaction` route remains a separate cross-table journal boundary because it coordinates DBF and sidecar images under the catalog lock.

The `src/transaction/` engine remains a lower-level WAL transaction primitive and does not provide DBF visibility or this table API.

Historical table and catalog snapshots remain read-only MVCC views.

## 4. Isolation boundary

The current API provides a private snapshot for one DBF table and optimistic stale-source rejection at commit.

It does not provide predicate locking, serializable conflict detection, row-level write-write merging, or a cross-table transaction object.

Independent row-retention policies and long-lived distributed transactions remain future work.

## 5. Example

```rust
use txbase::dbf::DbfTransaction;
use txbase::query::{parse, QueryExecutor};

let mut transaction = DbfTransaction::begin("users.dbf")?;
transaction.apply(&operation)?;
let rows = transaction.execute(&parse(br#"{}"#)?)?;
let committed_table = transaction.commit()?;
```

The HTTP route and this Rust API share the same table mutation and persistence path, so a behavior change must update both the API tests and the HTTP contract tests.

## Primary references

- [PostgreSQL transaction isolation](https://www.postgresql.org/docs/current/transaction-iso.html)
- [PostgreSQL concurrency control](https://www.postgresql.org/docs/current/mvcc.html)
- [SQLite isolation](https://sqlite.org/isolation.html)
- [SQLite write-ahead logging](https://sqlite.org/wal.html)
- [MVCC and historical snapshots](mvcc.md)
- [Mutation model](mutation-model.md)
- [HTTP method semantics and QUERY](http-semantics.md)
