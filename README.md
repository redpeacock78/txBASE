# txbase

A small Rust workspace for a transactional dBASE-compatible database.

The first slice keeps the DBF file format at the center and exposes a JSON
read path. It also provides an HTTP server with `GET`, standards-based
`QUERY`, and DBF-backed mutation routing. The repository now contains a
file-backed WAL and snapshot transaction core. xBase compatibility and
recovery of DBF mutations from that WAL remain later phases.

## What works now

- Reads classic DBF tables with 32-byte field descriptors.
- Detects dBASE Level 7 tables with 48-byte field descriptors.
- Reads header metadata, field descriptors, active records, and deleted-record flags.
- Converts common character, date, numeric, logical, integer, and double fields to JSON.
- Prints active records as JSON from the command line.
- Serves `GET /records`, `GET /records/{id}`, and executes `QUERY /records`.
- Serves `POST /records`, `PUT /records/{id}`, `PATCH /records/{id}`, and `DELETE /records/{id}`.
- Persists supported JSON mutations by atomically replacing the DBF file.
- Provides range storage, operation IR, file or memory WAL, and snapshot transaction types.

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

Create a record:

```bash
curl -i -X POST \
  -H 'Content-Type: application/json' \
  -d '{"ID":3,"NAME":"Carol","AGE":42,"ACTIVE":true}' \
  http://127.0.0.1:8080/records
```

Patch or replace a record, then logically delete it:

```bash
curl -i -X PATCH \
  -H 'Content-Type: application/json' \
  -d '{"NAME":"Caroline"}' \
  http://127.0.0.1:8080/records/3

curl -i -X DELETE http://127.0.0.1:8080/records/3
```

The `QUERY` route follows RFC 10008 at its boundary. It requires an
`application/json` content type and executes filter, sort, projection, skip,
and limit against active DBF records.

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
├── query.rs        JSON query document and executor
├── server.rs       HTTP routing, RFC 10008 checks, and DBF mutations
├── storage.rs      range-based storage boundary
├── transaction.rs  file or memory WAL and snapshot transactions
├── xbase.rs        shared operation IR boundary
├── lib.rs
└── main.rs
tests/fixtures/     hex fixture used by parser tests
docs/research.md    specification and design decisions
.github/workflows/ci.yml
```

The workspace intentionally has one package while the boundaries are still
small. A crate split should follow real ownership or build needs.

## Query execution

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
`$in`, `$nin`, `$and`, `$or`, and `$not`. Missing fields match `$ne` and `$nin`,
array values match when any element satisfies a predicate, and sort ties retain
DBF record order. Dotted paths, indexes, and update operators are not supported.

## Mutation semantics

`POST` creates a new physical DBF record and returns `201 Created` with its
one-based record location. `PUT` replaces all fields, while `PATCH` changes
only the fields in its JSON object. Missing fields in `POST` and `PUT` become
DBF null values, and unknown fields are rejected.

`DELETE` sets the DBF deletion marker and returns `204 No Content`. Deleted
record numbers are not reused, and subsequent reads return `404 Not Found`.
The server writes a complete temporary sibling file, syncs it, and renames it
over the DBF path. The WAL and snapshot transaction core are not yet coupled
to DBF mutation records, so replay-based recovery and concurrent writer
coordination remain outside this slice.

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
