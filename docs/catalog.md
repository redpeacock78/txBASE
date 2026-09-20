# Multi-table catalog

The catalog boundary maps one database directory to the DBF tables stored directly inside it.

This is the first multi-table slice in the roadmap.

It does not add relationships, cross-table indexes, or a second storage format.

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

This keeps the DBF files readable by existing xBase tools and leaves per-table recovery on the existing DBF path.

## Rust API

```rust
use txbase::catalog::Catalog;

let catalog = Catalog::from_path("database")?;
let users = catalog.open_table("users")?;
let names = catalog.table_names();
catalog.verify()?;
```

`open_table` loads one DBF through the existing recovery and memo-sidecar path.

`table_path` returns the path without loading the table.

`schema_json` loads every discovered table and returns its name, filename, and DBF schema.

`transaction_id` returns the last durable catalog-journal commit ID, if one exists. The ID is
advanced only by the multi-table `POST /transaction` journal boundary; independent named-table
mutations retain their per-table DBF transaction IDs.

`verify` loads and verifies every discovered table, reporting the table name when a table fails.

The bounded local join boundary is separate from catalog discovery.
Call `txbase::query::join::execute` with a `Catalog` and a validated join document to read one or
more named tables without adding a persistent relationship manifest.

The optional catalog server exposes that boundary over HTTP:

```bash
txbase --serve-catalog path/to/database --bind 127.0.0.1:8080
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
DBF. `POST /transaction` accepts named-table mutation paths and commits all affected DBFs under
one catalog journal; an incomplete prepare is rolled back on the next catalog read. Successful
catalog-journal commits advance a durable catalog transaction ID and return it in the JSON body
and `X-Txbase-Transaction-Id` header. The response also returns the new catalog representation
`ETag`. Optional `If-Match` and `If-None-Match` conditions are evaluated under the catalog write
lock. `If-Match` requires the current strong tag or `*`; a weak or non-matching value returns
`412 Precondition Failed`. A matching strong or weak `If-None-Match`, or `*`, returns the same
status without changing any DBF or sidecar. This is an ordering/identification boundary, not
MVCC visibility.
The catalog is discovered once at server startup, while each request loads the named table through
the existing recovery path. Named-table mutations do not add or remove tables.

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
input boundary used by the bounded local join, and an optional HTTP surface for independent
named-table reads and mutations.

It provides a cross-table atomic transaction boundary for named record mutations.

The catalog journal persists a monotonically increasing commit ID in
`.txbase.catalog.state`. Recovery rolls that state back with a prepared journal or reapplies it
with a committed journal. It does not provide historical row versions or MVCC visibility.

When a field sidecar declares `references: "TABLE.FIELD"`, catalog named-table mutations and
catalog transactions validate non-null child values against active rows in the referenced table.
They reject parent updates or logical deletes that would leave a child dangling; nulls are allowed
and changes are not cascaded. Direct single-table routes cannot resolve these cross-table rules.

The catalog lock serializes catalog reads and writes, while per-table locks continue to protect
direct DBF persistence.

It does not infer relationships from field names.

The local join supports `inner`, `left`, `right`, `semi`, and `anti` equality joins plus a bounded
`cross` join, with a hard result bound.
It does not provide a cost-based planner, runtime-specific async stream traits,
planner-selected join strategies, or MVCC visibility.
