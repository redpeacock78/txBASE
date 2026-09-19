<!-- English README -->

<div align="center">

[![CI](https://github.com/redpeacock78/txBASE/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/redpeacock78/txBASE/actions/workflows/ci.yml)
[![License](https://img.shields.io/github/license/redpeacock78/txBASE)](LICENSE)

# txBASE

A small Rust database prototype that keeps the dBASE DBF file format at its center.

It reads and writes selected dBASE and Visual FoxPro fields, exposes JSON and HTTP access, and uses a file-backed WAL for recoverable mutations.

</div>

<table>
<tr>
<td><a href="README.md">English</a></td>
<td><a href="README.ja.md">日本語</a></td>
</tr>
</table>

## How to use

### Quick start

Read active records from a DBF file:

```bash
cargo run -- path/to/users.dbf
```

Start the HTTP server:

```bash
cargo run -- --serve path/to/users.dbf
```

The default listener is `127.0.0.1:8080`. Use `--bind ADDRESS` to choose another address.

### Read

```bash
curl -s http://127.0.0.1:8080/records | jq
curl -s http://127.0.0.1:8080/records/1 | jq
```

Reads return active records as JSON and use the one-based physical DBF record number in the URL.

### Query

`QUERY /records` accepts an `application/json` query document shaped after a small, deliberate subset of MongoDB predicates.

```bash
curl -i -X QUERY \
  -H 'Content-Type: application/json' \
  -d '{"filter":{"AGE":{"$gte":20}},"sort":{"NAME":1},"limit":10}' \
  http://127.0.0.1:8080/records
```

The current query surface is `filter`, `sort`, `projection`, `skip`, and `limit` with `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte`, `$in`, `$nin`, `$and`, `$or`, and `$not`.

`QUERY` follows the HTTP QUERY boundary defined by [RFC 10008](https://www.rfc-editor.org/rfc/rfc10008.html), including `Accept-Query: "application/json"`.

### Mutate

```bash
curl -i -X POST \
  -H 'Content-Type: application/json' \
  -d '{"ID":3,"NAME":"Carol","AGE":42,"ACTIVE":true}' \
  http://127.0.0.1:8080/records

curl -i -X PATCH \
  -H 'Content-Type: application/json' \
  -d '{"NAME":"Caroline"}' \
  http://127.0.0.1:8080/records/3

curl -i -X DELETE http://127.0.0.1:8080/records/3
```

`POST` creates a record, `PUT` replaces one, `PATCH` applies a partial update, and `DELETE` sets the DBF deletion marker.

`PATCH` also accepts the typed `$set`, `$unset`, and `$inc` operators.

Path-loaded mutations use a `TXDP` byte-range delta when it is smaller than a complete snapshot, otherwise a `TXDB` or `TXDM` snapshot.

The WAL is synced before DBF or memo sidecar replacement, and startup recovery replays a durable unfinished mutation.

When an existing index sidecar is affected, its target is recorded in the same WAL and replayed after a crash before the WAL is cleared.

### Inspect and maintain

Inspect a table without changing its DBF bytes:

```bash
txbase schema path/to/users.dbf
txbase verify path/to/users.dbf
txbase catalog path/to/database
txbase verify-catalog path/to/database
txbase index build path/to/users.dbf NAME AGE
txbase index verify path/to/users.dbf
txbase index rebuild path/to/users.dbf
txbase pack path/to/users.dbf
txbase recall path/to/users.dbf 2
```

Copy a DBF and its sibling `.dbt` or `.fpt` memo sidecar:

```bash
txbase backup path/to/users.dbf backups/users.dbf
txbase restore backups/users.dbf path/to/users.dbf
```

The command-line interface is:

```text
Usage:
  txbase FILE
  txbase schema FILE
  txbase verify FILE
  txbase catalog DIRECTORY
  txbase verify-catalog DIRECTORY
  txbase index build FILE FIELD...
  txbase index verify FILE
  txbase index rebuild FILE
  txbase pack FILE
  txbase recall FILE RECORD
  txbase backup SOURCE DEST
  txbase restore SOURCE DEST
  txbase --serve FILE [--bind ADDRESS]
```

`schema` prints the parsed header and field descriptors.

`verify` reparses the loaded DBF and checks its internal record boundaries.

`catalog` discovers direct-child DBF tables and prints each table schema.

`verify-catalog` verifies every discovered table.

`index build` creates an external scalar-key sidecar.

`index verify` rejects a sidecar whose DBF or memo source is stale.

Normal DBF saves record and apply an existing sidecar's target through the WAL; a stale or invalid sidecar is rejected and `index rebuild` remains the explicit repair path.

The path-aware planner can intersect candidates from multiple valid single-field indexes for direct equality filters.

`pack` removes logically deleted records and renumbers the remaining physical records.

`recall` restores one logically deleted record by its physical record number.

`backup` and `restore` validate the source first, then copy the DBF and its detected memo sidecar.

The implementation currently favors a readable DBF file plus separate WAL and memo sidecars.

## Install

### Supported platforms

The CI matrix covers Ubuntu, macOS, and Windows with the stable Rust toolchain.

Install Rust through [rustup](https://rustup.rs/), then build the binary:

```bash
cargo build --release
```

The resulting binary is `target/release/txbase`.

### Project files and sidecars

For a table such as `users.dbf`, supported memo data is discovered from sibling `.dbt` or `.fpt` files.

The transaction layer keeps its WAL beside the configured table path.

## Development

### Quality gates

Run the same checks used by CI:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

CI runs these checks on Ubuntu, macOS, and Windows.

### Repository layout

```text
src/dbf/            DBF parsing, codecs, memo sidecars, maintenance, mutation, WAL, and tests
src/catalog.rs      Direct-child DBF discovery, table lookup, and catalog verification
src/index.rs        External scalar-key index sidecar lifecycle
src/query.rs        JSON query execution and validation
src/query_path.rs   Dotted-path traversal and projection helpers
src/server.rs       HTTP routing, QUERY validation, and DBF mutations
src/storage.rs      Range-based storage boundary
src/transaction.rs  File or memory WAL and snapshot transactions
src/xbase.rs        Shared operation IR boundary
tests/fixtures/     External-format fixtures
tests/corpus/       Malformed DBF, memo, WAL, and JSON inputs
docs/               Topic-specific specifications and design notes
```

Files are split when ownership or maintenance becomes clearer; a crate split should wait for a real build or ownership boundary.

### Documentation

- [DBF and dBASE compatibility](docs/dbf-compatibility.md)
- [Multi-table catalog](docs/catalog.md)
- [Secondary-index sidecar](docs/indexes.md)
- [MongoDB query model and txBASE query behavior](docs/query-model.md)
- [Firebase data-model and synchronization lessons](docs/firebase-model.md)
- [SQLite testing and quality model](docs/testing-quality.md)
- [HTTP method semantics and QUERY](docs/http-semantics.md)
- [Roadmap and explicit non-goals](docs/roadmap.md)
- [Research index and source policy](docs/research.md)

## Todo

The roadmap is research-led and does not turn every compatibility idea into code.

Near-term work is to harden the current DBF, memo, WAL, query, and HTTP contracts with fixtures and failure tests.

Later phases may add multi-key ordered planning, joins, aggregation, cursors, stronger multi-record transactions, CJK encodings, and an XBF native format.

See [docs/roadmap.md](docs/roadmap.md) for the phase boundaries and acceptance conditions.

## License

MIT. See [LICENSE](LICENSE).
