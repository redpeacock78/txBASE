# Multi-table catalog

The catalog boundary maps one database directory to the DBF tables stored directly inside it.

This is the first multi-table slice in the roadmap.

Catalog-journal commits also retain a consistent historical image of every discovered table.

It does not add relationships, cross-table index definitions, or a second storage format.

## Filesystem contract

`Catalog::from_path` accepts an existing directory.

Each regular file whose extension is `.dbf`, case-insensitively, becomes one table.

The table name is the filename stem.

For example:

```text
database/
├── comments.dbf
├── posts.dbf
├── posts.dbt
└── users.dbf
```

The catalog exposes `comments`, `posts`, and `users`.

Memo sidecars, WAL files, lock files, and unrelated files are not catalog entries.

Only direct children are discovered.

Table names are matched exactly by the public API.

The catalog does not write a manifest, rename files, or claim ownership of sidecars.

The `.txbase.catalog.mvcc` history is an internal versioned snapshot sidecar, not a table-discovery manifest.

This keeps the DBF files readable by existing xBase tools and leaves per-table recovery on the existing DBF path.

## Rust API

```rust
use txbase::catalog::Catalog;

let catalog = Catalog::from_path("database")?;
let users = catalog.open_table("users")?;
let names = catalog.table_names();
catalog.verify()?;

let versions = Catalog::mvcc_versions("database")?;
let historical = Catalog::from_path_at("database", 1)?;
let old_users = historical.open_table("users")?;
let read = catalog.begin_read()?;
let stable_users = read.open_table("users")?;
let stable_rows = read.execute_join(&join_request)?;
let mut transaction = catalog.begin_serializable()?;
transaction.apply(&operation)?;
let transaction_id = transaction.commit()?;
```

`open_table` loads one DBF through the existing recovery and memo-sidecar path.

`table_path` returns the path without loading the table.

`schema_json` loads every discovered table and returns its name, filename, and DBF schema.

`transaction_id` returns the catalog commit ID represented by the catalog. For a current catalog,
the ID is the last durable catalog-journal commit ID, if one exists. For a historical catalog, it
is the snapshot ID used to open that catalog.

`mvcc_versions` lists catalog commits that have a retained image, and `from_path_at` opens one
read-only image. Every table opened from that value belongs to the same catalog commit.

Historical tables use their retained DBF, memo, and schema images and do not reuse current index
sidecars.

`Catalog::begin_read` captures every discovered current table as one in-memory, read-only image.
It first recovers pending catalog and table state, then takes the catalog read lock and all table
read locks in deterministic table order while loading DBF, memo, and schema bytes.
The locks are released after capture, so the read object does not block later writers.

`CatalogReadTransaction::transaction_id` reports the catalog commit ID observed at capture time,
or `None` when no catalog-journal commit exists yet.
`open_table` returns an independent read-only table copy, and `execute_join` runs the existing
bounded join contract, including chained stages, against the captured tables.
The captured join cannot use live index sidecars; it retains the same result and stage bounds and
uses the scan/hash execution paths.

`Catalog::begin_serializable` opens an opt-in coarse-grained serializable transaction over every
discovered table.
It holds the catalog write lock and all per-table exclusive locks from begin through commit,
rollback, or drop.
`CatalogTransaction::apply` changes private table copies, and `commit` validates cross-table
constraints and publishes one catalog journal transaction.
`open_table` exposes a read-only copy of the private image.
This boundary is catalog-wide for the discovered table set.
The table set is captured when the transaction begins, and commit rejects a changed DBF name or
file name instead of silently omitting or including a table.
It does not provide predicate-level concurrency or distributed coordination.

This in-memory transaction is not a durable history pin and does not extend MVCC retention.
Use `Catalog::from_path_at` when a retained commit must be reopened after the process exits.

The ID advances only through the multi-table `POST /transaction` journal boundary. Independent
named-table mutations retain their per-table DBF transaction IDs and do not create a new catalog
image.

`verify` loads and verifies every discovered table and any present index sidecar, reporting the
table name when a table or its sidecar fails.

The bounded local join boundary is separate from catalog discovery.
Call `txbase::query::join::execute` with a `Catalog` and a validated join document to read one or
more named tables without adding a persistent relationship manifest.

The optional catalog server exposes that boundary over HTTP:

```bash
txbase serve-catalog path/to/database --bind 127.0.0.1:8080
```

`GET` or `HEAD /catalog` returns the discovered table schemas with a strong catalog representation
`ETag`. A matching strong or weak `If-None-Match`, or `*`, returns `304 Not Modified` with no
body. `GET` or `HEAD /{table}/records`
and `/{table}/records/{id}` reuse the single-table record response semantics, including the
current representation `ETag`. `QUERY /{table}/records` accepts the same JSON query document as
the single-table route. `QUERY /{table}/records/stream` exposes the bounded `application/x-ndjson`
stream for filter, projection, skip, and limit. `QUERY /{table}/explain` returns the same selected plan as the
single-table `QUERY /explain`. `QUERY /join` accepts the same JSON join document as
`query::join::parse`, returns a JSON array, and retains the 100,000-row per-stage join bound.

`POST /{table}/records` and `PUT`/`PATCH`/`DELETE /{table}/records/{id}` reuse the single-table
mutation, WAL, ETag, validation, and constraint behavior. Each request commits only its named
DBF. `POST /transaction` accepts named-table mutation paths and commits all affected DBFs and
their changed index sidecars under one catalog journal; it also records one image of every
discovered table in the catalog MVCC history. An incomplete prepare, including its history image,
is rolled back on the next catalog read. Successful
catalog-journal commits advance a durable catalog transaction ID and return it in the JSON body
and `X-Txbase-Transaction-Id` header. The response also returns the new catalog representation
`ETag`. Optional `If-Match` and `If-None-Match` conditions are evaluated under the catalog write
lock. `If-Match` requires the current strong tag or `*`; a weak or non-matching value returns
`412 Precondition Failed`. A matching strong or weak `If-None-Match`, or `*`, returns the same
status without changing any DBF or sidecar. The transaction ID is also the selector for one-request
historical reads on the catalog HTTP server. It does not create a long-lived transaction or
provide row-level visibility.
Each successful `POST /transaction` also publishes one `TXCC` event to `.txbase.catalog.cdc`.
The event contains the physical-record state differences for every changed table in that catalog commit.
The catalog journal applies or rolls back the CDC sidecar with the DBF, index, MVCC, and transaction-state targets.
The catalog is discovered once at server startup, while each request loads the named table through
the existing recovery path. Named-table mutations do not add or remove tables.

Historical catalog images are also exposed through the catalog HTTP server.

`GET` or `HEAD /catalog`, `GET` or `HEAD /{table}/records[/{id}]`, `QUERY /{table}/records` and
`/{table}/records/stream`, `QUERY /{table}/explain`, and `QUERY /join` accept
`?at=<positive committed catalog transaction ID>`.

The request reads one exact image of every table from that catalog commit.

Historical query and join execution does not reuse current index sidecars, and historical explain
responses report a table scan.

`at` is read-only: a catalog mutation with the parameter returns `405`.

Zero, malformed, or repeated `at` parameters return `400`; an ID that is not retained as a
committed catalog snapshot returns `422`.

An invalid DBF does not prevent directory discovery because discovery only identifies files.

The invalid table is reported by `open_table`, `schema_json`, or `verify`.

## CLI

Print the catalog and all table schemas:

```bash
txbase catalog path/to/database
```

Verify every discovered table:

```bash
txbase verify-catalog path/to/database
```

List retained catalog commits and read one consistent historical image:

```bash
txbase mvcc catalog list path/to/database
txbase mvcc catalog read path/to/database 1
txbase mvcc catalog gc path/to/database --keep 5
txbase cdc catalog path/to/database
txbase cdc catalog path/to/database --after 10
```

The catalog output has this shape:

```json
{
  "format": "txbase-catalog",
  "transaction_id": null,
  "tables": [
    {
      "name": "users",
      "file": "users.dbf",
      "schema": {
        "format": "dbf"
      }
    }
  ]
}
```

The full nested schema includes the DBF header, record counts, memo sidecar detection, and field descriptors.

## Boundary

The catalog currently provides discovery, lookup, schema introspection, verification, the
input boundary used by the bounded local join, an optional HTTP surface for independent
named-table reads and mutations, and read-only catalog-wide historical snapshots.

It provides a cross-table atomic transaction boundary for named record mutations.
`Catalog::begin_serializable` additionally provides an opt-in coarse-grained serializable Rust
transaction that holds the catalog write lock and every discovered table lock for its lifetime,
then commits private table copies through the same catalog journal.

It exposes those multi-table commits through `Catalog::cdc_events` and `txbase cdc catalog DIRECTORY`.
The catalog CDC stream is limited to the explicit multi-table transaction boundary; independent named-table routes keep their table-scoped CDC events.

The catalog journal persists a monotonically increasing commit ID in
`.txbase.catalog.state`. Recovery rolls that state back with a prepared journal or reapplies it
`.txbase.catalog.state`. Each successful multi-table commit stores a full image of all discovered
tables in `.txbase.catalog.mvcc` through the same journal. Recovery rolls the state and history
back with a prepared journal or reapplies both with a committed journal.

The history provides commit-level visibility for catalog images.
`Catalog::gc_mvcc` and the catalog GC command retain the newest positive count of full-image
snapshots and replace only the history sidecar through a synced temporary file.
An ID removed by GC is no longer readable, while later catalog commits append after the retained
IDs.
It does not expose table-local row history for a historical catalog snapshot; direct table MVCC
exposes that history separately.

The catalog does not provide independent row retention, distributed snapshots, or predicate-level
serializable conflict detection.

`Catalog::begin_serializable` provides coarse-grained serial execution across the discovered table
set.
It validates that the discovered DBF name and file-name set is unchanged at commit.
It does not provide predicate-level locking or distributed serializable coordination.

The Rust `CatalogReadTransaction` provides a nonblocking, stable cross-table read image after its
capture completes, but it does not provide predicate locking, serializable conflict detection, or
row-level write-write merging.

When a field sidecar declares `references: "TABLE.FIELD"`, catalog named-table mutations and
catalog transactions validate non-null child values against active rows in the referenced table.
Nulls are allowed. Optional `on_delete` and `on_update` actions default to `restrict`; `cascade`
propagates a matching parent delete or key update, while `set_null` clears the local key fields.
`set_null` requires nullable, non-primary local fields.

A schema sidecar can also declare `constraints.foreign_keys` with equal-length child and parent
field lists. The catalog compares the complete tuple, skips the check when any child value is null,
and applies the same actions to parent updates and logical deletes.

Cascades are applied recursively inside one catalog transaction and journal commit.
Constraint failures or a non-converging cascade are rejected before any table is published.
Direct single-table routes cannot resolve these cross-table rules.

The catalog lock serializes catalog reads and writes, while per-table locks continue to protect
direct DBF persistence.

It does not infer relationships from field names.

The local join supports `inner`, `left`, `right`, `full`, `semi`, and `anti` equality joins plus a bounded
`cross` join, with a hard result bound.
Direct equality joins include deterministic pre-filter cardinality, output-materialization, and logical DBF-page inputs in strategy selection.
The catalog does not provide full cardinality and materialization propagation through chained stages, filesystem- and cache-aware merge planning,
host-specific `AsyncQueryStream` scheduling, row-level MVCC versions, or distributed visibility.

## Primary references and scope

The catalog is a txBASE-owned directory and transaction contract, not an implementation of an external catalog standard.
Its DBF and sidecar rules defer to [DBF compatibility](dbf-compatibility.md), local join semantics defer to [Join model](joins.md), and field constraints defer to [Schema metadata](schema-metadata.md).

No external catalog source belongs in this document until txBASE selects a distributed catalog or an external compatibility target.
