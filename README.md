# dbase-ng

A small Rust workspace for a transactional dBASE-compatible database.

The first slice keeps the DBF file format at the center and exposes a JSON
read path. It also provides an HTTP server with `GET` and standards-based
`QUERY` routing. WAL, MVCC, mutation operations, and xBase compatibility are
interfaces for later phases, not pretend implementations.

## What works now

- Reads classic DBF tables with 32-byte field descriptors.
- Detects dBASE Level 7 tables with 48-byte field descriptors.
- Reads header metadata, field descriptors, active records, and deleted-record flags.
- Converts common character, date, numeric, logical, integer, and double fields to JSON.
- Prints active records as JSON from the command line.
- Serves `GET /records`, `GET /records/{id}`, and a validated `QUERY /records` route.
- Provides storage, operation-IR, WAL, MVCC, and transaction traits without claiming that they are complete.

The DBF decoder currently treats text as UTF-8 with replacement for invalid
bytes. The language-driver byte is retained in the parsed header, but full OEM
and Windows code-page conversion is deliberately not implemented yet.

## Quick start

Read a DBF table:

```bash
cargo run -- path/to/users.dbf
```

Start the HTTP server:

```bash
cargo run -- --serve path/to/users.dbf
```

Fetch the active record with the one-based DBF record number:

```bash
curl -s http://127.0.0.1:8080/records/1 | jq
```

List active records:

```bash
curl -s http://127.0.0.1:8080/records | jq
```

The `QUERY` route follows RFC 10008 at its boundary. It requires an
`application/json` content type, validates the current query document shape,
and returns `501 Not Implemented` until the query executor is added.

```bash
curl -i -X QUERY \
  -H 'Content-Type: application/json' \
  -d '{"filter":{"AGE":{"$gte":20}},"limit":10}' \
  http://127.0.0.1:8080/records
```

Use `--bind ADDRESS` to select another listener address.

## Repository layout

```text
src/
├── dbf.rs          DBF headers, descriptors, records, and JSON conversion
├── query.rs        JSON query document and executor boundary
├── server.rs       minimal HTTP routing and RFC 10008 boundary checks
├── storage.rs      range-based storage boundary
├── transaction.rs  WAL, MVCC, and transaction traits
├── xbase.rs        shared operation IR boundary
├── lib.rs
└── main.rs
tests/fixtures/     hex fixture used by parser tests
docs/research.md    specification and design decisions
.github/workflows/ci.yml
```

The workspace intentionally has one package while the boundaries are still
small. A crate split should follow real ownership or build needs.

## Query boundary

The query document is shaped after the useful part of MongoDB predicates, not
after a promise of MongoDB compatibility:

```json
{
  "filter": {
    "AGE": {"$gte": 20, "$lt": 30},
    "COUNTRY": {"$in": ["JP", "TW"]}
  },
  "sort": {"AGE": 1},
  "projection": {"NAME": 1, "AGE": 1},
  "limit": 100,
  "skip": 0
}
```

The initial operator vocabulary is `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte`,
`$in`, `$nin`, `$and`, `$or`, and `$not`. Execution semantics, missing-field
behavior, array traversal, indexing, and update operators must be specified
before they are advertised as compatible behavior.

## Quality gates

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

The testing roadmap follows the useful lessons from SQLite without copying its
scale: parser boundary tests first, then reference-model and property tests,
malformed-input and fuzz tests, crash and recovery tests, compatibility
fixtures, and cross-platform CI.

## Research

The decisions and source links are recorded in [docs/research.md](docs/research.md).
The Japanese overview is [README.ja.md](README.ja.md).

## License

MIT.
