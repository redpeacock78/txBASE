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

The current query surface is `filter`, `sort`, `projection`, `skip`, `limit`, `page_size`, `cursor`, and a bounded `aggregate` pipeline with `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte`, `$in`, `$nin`, `$and`, `$or`, `$not`, and bounded `$expr` field comparisons.

`page_size` enables a cursor response. Without `sort`, the cursor follows physical record order;
with `sort`, it is a keyset token and the next request repeats the same sort definition.
Neither mode combines with `skip`, and both are capped at 1,000 records.

Physical cursor pages scan in record order and stop after the page plus one look-ahead match.
Sorted cursor pages reuse the query sort comparator and provide a deterministic physical-record
tie-breaker.

The library also exposes `query::stream_query`, a borrowed iterator that applies filter,
projection, skip, and limit without materializing the matching record set.
`query::stream_query_snapshot` provides the same pull-based iterator over an owned clone of the
loaded table, so later mutations of the source table do not change the stream's records.
Both APIs reject sort, aggregate, page-size, and cursor controls, which need a blocking or
resumable result boundary; an asynchronous backpressure protocol remains future work.

`aggregate` currently accepts one `$group` stage with `$count`, integer `$sum`, `$min`, and `$max`, optionally preceded by bounded `$match` stages and followed by one `$sort` and `$limit` over group output; top-level sort, projection, pagination, and limit controls remain incompatible.

The library also exposes one bounded local `inner`, `left`, `semi`, or `anti` equality join, plus a bounded `cross` join, over two catalog tables.
It accepts qualified `from`, `join.on`, `filter`, and `projection` fields, emits qualified JSON keys,
supports multiple equality conditions, and is capped at 100,000 output rows.
`semi` and `anti` emit only qualified left-table fields, based on whether a right-side match exists.
`cross` requires an empty `on` object and caps candidate pairs at 100,000.
This join is not exposed by the single-table HTTP server yet.

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

Multiple record mutations can be committed to one DBF table through one snapshot/WAL boundary:

```bash
curl -i -X POST \
  -H 'Content-Type: application/json' \
  -d '{"operations":[{"method":"POST","path":"/records","body":{"ID":3,"NAME":"Carol","AGE":42,"ACTIVE":true}},{"method":"PATCH","path":"/records/3","body":{"$inc":{"AGE":1}}}]}' \
  http://127.0.0.1:8080/transaction
```

The batch is single-table and all operations run on a private copy before one commit. Cross-table atomicity and MVCC visibility are not provided.

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
txbase index build-compound path/to/users.dbf by_name_age NAME AGE
txbase index verify path/to/users.dbf
txbase index rebuild path/to/users.dbf
txbase pack path/to/users.dbf
txbase recall path/to/users.dbf 2
txbase xbf import path/to/users.dbf path/to/users.xbf
txbase xbf export path/to/users.xbf path/to/users.dbf
txbase xbf export path/to/users.xbf path/to/users.dbf --schema
```

Copy a DBF and its sibling `.dbt` or `.fpt` memo sidecar:

```bash
txbase backup path/to/users.dbf backups/users.dbf
txbase restore backups/users.dbf path/to/users.dbf
```

The command-line interface is:

```text
Usage:
  txbase FILE [--encoding NAME]
  txbase schema FILE [--encoding NAME]
  txbase verify FILE [--encoding NAME]
  txbase catalog DIRECTORY
  txbase verify-catalog DIRECTORY
  txbase index build FILE FIELD...
  txbase index build-compound FILE NAME FIELD[:1|-1] FIELD[:1|-1]...
  txbase index verify FILE
  txbase index rebuild FILE
  txbase pack FILE [--encoding NAME]
  txbase recall FILE RECORD [--encoding NAME]
  txbase backup SOURCE DEST
  txbase restore SOURCE DEST
  txbase --serve FILE [--bind ADDRESS] [--encoding NAME]
```

`schema` prints the parsed header and field descriptors.

`verify` reparses the loaded DBF and checks its internal record boundaries.

`catalog` discovers direct-child DBF tables and prints each table schema.

`verify-catalog` verifies every discovered table.

`index build` creates one external scalar-key index per field.

`index build-compound` creates one compound-key index in the declared field order; each field may use `:1`, `:-1`, `:asc`, or `:desc`.

`index verify` rejects a sidecar whose DBF or memo source is stale.

Normal DBF saves record and apply an existing sidecar's target through the WAL; a stale or invalid sidecar is rejected and `index rebuild` remains the explicit repair path.

The path-aware planner can intersect candidates from multiple valid single-field indexes for direct equality filters.

It uses the sidecar's active-record and distinct-key statistics as a uniform equality estimate and can order single-field range candidates with a bounded histogram estimate; compatible compound sort candidates use exact candidate counts, not a full cost-based planner.

For multi-key sorts, it can use a single-field index for the first sort key and sort only equal-key groups by the remaining keys; this is not compound-index support.

It can also use a per-field-direction compound index for a matching multi-key sort, including the complete reverse direction and an exact equality prefix.

`pack` removes logically deleted records and renumbers the remaining physical records.

`recall` restores one logically deleted record by its physical record number.

`xbf import` converts a loaded DBF table to a bounded XBF snapshot. `xbf export` converts only the
representable XBF subset to a DBF file and reports unsupported types or values as errors.
The library API `XbfTable::to_dbf_with_schema` additionally returns sidecar JSON for representable
`primary`, `unique`, and `not_null` constraints. `XbfTable::save_dbf_with_schema` and CLI
`xbf export --schema` write that metadata to the sibling `.txschema.json` sidecar.

`backup` and `restore` validate the source first, then copy the DBF and its detected memo sidecar.

When a DBF language-driver byte is missing or untrusted, the read, schema, verify, pack, recall,
and server commands accept `--encoding NAME` for the four declared CJK codecs plus explicit-only
EUC-JP and GB18030 overrides.
The supported aliases are `windows-31j`/`cp932`, `gbk`/`cp936`, `euc-kr`/`cp949`, and
`big5`/`cp950`, `euc-jp`, and `gb18030`.
The invocation override takes precedence over `*.txschema.json`, is not persisted, and is visible
as the effective `encoding_override` and `encoding_metadata.effective` in schema output.
`encoding_metadata` also reports the declared codec and whether the source was the language driver,
an explicit override, or the fallback.

The DBF codec recognizes the Visual FoxPro CJK driver IDs for Windows-31J/CP932, GBK/CP936,
EUC-KR/CP949, and Big5/CP950.
Malformed reads use U+FFFD and writes reject unmappable or over-width values.

An optional `users.txschema.json` sidecar adds one-field `primary`, `unique`, and `not_null`
constraints and can explicitly select one of the supported CJK codecs without changing legacy DBF bytes.
Loaded active records and every insert, replace, patch, and recall are checked against it;
`CHECK`, foreign keys, defaults, and composite keys remain future work.

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
src/index.rs        External scalar and compound-key index sidecar lifecycle
src/query.rs        JSON query execution and filter evaluation
src/query/          planner, ordering, validation, bounded join, and query-specific tests
src/query_path.rs   Dotted-path traversal and projection helpers
src/server.rs       HTTP routing, QUERY validation, and DBF mutations
src/storage.rs      Range-based storage boundary
src/transaction.rs  File or memory WAL and snapshot transactions
src/xbf/            Bounded XBF v1 codec, DBF conversion, persistence, WAL, and tests
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
- [Quality contract matrix](docs/quality-matrix.md)
- [HTTP method semantics and QUERY](docs/http-semantics.md)
- [Schema metadata and local constraints](docs/schema-metadata.md)
- [XBF v1 format draft](docs/xbf.md)
- [Roadmap and explicit non-goals](docs/roadmap.md)
- [Research index and source policy](docs/research.md)

## Todo

The roadmap is research-led and does not turn every compatibility idea into code.

Near-term work is to harden the current DBF, memo, WAL, query, and HTTP contracts with fixtures and failure tests.

Later phases may add a full cost-based index choice, streaming backpressure, planned and multiple joins, additional aggregation stages, cross-table transactions, additional CJK encodings, and a multi-file atomic commit protocol for XBF export.

See [docs/roadmap.md](docs/roadmap.md) for the phase boundaries and acceptance conditions.

## License

MIT. See [LICENSE](LICENSE).
