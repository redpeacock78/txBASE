# Multi-table catalog

The catalog boundary maps one database directory to the DBF tables stored directly inside it.

This is the first multi-table slice in the roadmap.

It does not add joins, cross-table mutations, indexes, or a second storage format.

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

`verify` loads and verifies every discovered table, reporting the table name when a table fails.

The bounded local join boundary is separate from catalog discovery.
Call `txbase::query::join::execute` with a `Catalog` and a validated join document to read two
named tables without adding a manifest or cross-table write lock.

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

The catalog currently provides discovery, lookup, schema introspection, verification, and the
input boundary used by the bounded local join.

It does not provide a cross-table transaction.

It does not provide a shared lock across tables.

It does not infer relationships from field names.

The local join supports `inner`, `left`, `semi`, and `anti` joins with one or more equality
conditions and a hard result bound.
It does not provide a cost-based planner, streaming backpressure, multiple joins, or cross-table writes.
Cross-table transactions still require separate contracts for visibility, failure recovery, and
malformed input.
