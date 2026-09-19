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

### Recover and inspect

The command-line interface is intentionally small:

```text
Usage:
  txbase FILE
  txbase --serve FILE [--bind ADDRESS]
```

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
src/dbf/            DBF parsing, codecs, memo sidecars, mutation, WAL, and tests
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
- [MongoDB query model and txBASE query behavior](docs/query-model.md)
- [Firebase data-model and synchronization lessons](docs/firebase-model.md)
- [SQLite testing and quality model](docs/testing-quality.md)
- [HTTP method semantics and QUERY](docs/http-semantics.md)
- [Roadmap and explicit non-goals](docs/roadmap.md)
- [Research index and source policy](docs/research.md)

## Todo

The roadmap is research-led and does not turn every compatibility idea into code.

Near-term work is to harden the current DBF, memo, WAL, query, and HTTP contracts with fixtures and failure tests.

Later phases may add secondary indexes, catalog metadata, joins, aggregation, cursors, stronger multi-record transactions, CJK encodings, and an XBF native format.

See [docs/roadmap.md](docs/roadmap.md) for the phase boundaries and acceptance conditions.

## License

MIT. See [LICENSE](LICENSE).
