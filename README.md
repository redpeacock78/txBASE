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

The current query surface is `filter`, `sort`, `projection`, `skip`, `limit`, `page_size`, `cursor`, and a bounded `aggregate` pipeline with `$match`, `$count`, `$distinct`, `$group`, `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte`, `$in`, `$nin`, `$and`, `$or`, `$not`, and bounded `$expr` boolean trees over field comparisons and numeric `$abs`, `$add`/`$subtract`/`$multiply`/`$divide`/`$mod` operands.

`page_size` enables a cursor response. Without `sort`, the cursor follows physical record order;
with `sort`, it is a keyset token and the next request repeats the same sort definition.
Neither mode combines with `skip`, and both are capped at 1,000 records.

Physical cursor pages scan in record order and stop after the page plus one look-ahead match.
Sorted cursor pages reuse the query sort comparator and provide a deterministic physical-record
tie-breaker.
Emitted cursors are opaque, versioned, and bound to the current table representation. Reusing a
cursor after the table changes returns an invalid-query error instead of silently mixing snapshots.
Legacy numeric physical cursors and version-1 sorted cursors remain accepted without that binding.

The library also exposes `query::stream_query`, a borrowed iterator that applies filter,
projection, skip, and limit without materializing the matching record set.
`query::stream_query_snapshot` provides the same pull-based iterator over an owned clone of the
loaded table, so later mutations of the source table do not change the stream's records.
`query::stream_query_bounded` runs that snapshot iterator behind a bounded standard-library
channel. The producer blocks when the channel is full and stops when the consumer is dropped.
Both APIs reject sort, aggregate, page-size, and cursor controls, which need a blocking or
resumable result boundary. `QUERY /records/stream` and catalog `QUERY /{table}/records/stream`
reuse the bounded snapshot stream and return one record per `application/x-ndjson` line over
HTTP/1.1 chunked transfer. Runtime-specific async traits remain future work.

`aggregate` currently accepts one terminal `$count` or `$distinct` stage, or one `$group` stage with `$count`, numeric `$sum`, `$avg`, `$min`, `$max`, `$first`, and `$last`, optionally preceded by bounded `$match` stages. Group output may be filtered by bounded `$match` stages before one `$project`, `$sort`, and `$limit`; top-level sort, projection, pagination, and limit controls remain incompatible. `$distinct` takes a field reference such as `"$ACTIVE"` and returns a deterministic array of unique values, with missing values represented as `null`. `$project` reuses inclusion/exclusion projection rules and must precede `$sort`/`$limit`. `$sum` ignores missing, null, and nonnumeric values, preserves all-integral input as an integer, and rejects a non-finite accumulated result. `$avg` ignores missing, null, and nonnumeric values and returns null when a group has no numeric input. `$first` and `$last` use physical input order within each group.

The library also exposes bounded local `inner`, `left`, `right`, `semi`, or `anti` equality joins, plus bounded `cross` joins, over a catalog table and optional additional stages.
Direct and chained single-key equality stages compare bounded hash and index-probe costs after the 64-pair nested-loop boundary, using a fresh single-field index only when its estimate is lower.
Direct and chained stages can also use a fresh compound index when the equality fields exactly match its field order.
Large direct joins with compatible fresh ordered indexes on both inputs use a bounded merge strategy when its estimate is lowest while restoring the documented output order; I/O-aware full cost-based join planning remains future work.
The required `join` is the first stage; an optional `joins` array applies additional stages from left to right.
Each stage accepts qualified `from`, `join.on`, `filter`, and `projection` fields, emits qualified JSON keys,
supports multiple equality conditions, and is capped at 100,000 output rows.
`semi` and `anti` emit only qualified left-table fields, based on whether a right-side match exists.
`cross` requires an empty `on` object and caps candidate pairs at 100,000.
The single-table HTTP server does not expose joins. Run the catalog server to expose catalog
schema through `GET /catalog` with a strong catalog representation `ETag`, named-table reads and independent mutations through
`GET`/`HEAD`/`POST`/`PUT`/`PATCH`/`DELETE /{table}/records[/{id}]`, and the same bounded join through
`QUERY /join`:

```bash
txbase --serve-catalog path/to/database --bind 127.0.0.1:8080
```

`QUERY /{table}/records` and `QUERY /{table}/explain` accept the same query document as the
single-table routes. `QUERY /{table}/records/stream` returns the same filter/projection/skip/limit
stream as the single-table stream route. Named-table mutations reuse single-table WAL/ETag behavior and commit one
DBF at a time, including its `X-Txbase-Transaction-Id` response header. Catalog `POST /transaction`
commits named-table mutations atomically through a catalog journal and returns its durable catalog
commit ID and new catalog representation `ETag` in JSON and response headers. Catalog transactions
accept optional `If-Match` and `If-None-Match` conditions and reject a failed condition with `412
Precondition Failed` without mutation; MVCC remains future work.

The single-table server also exposes `QUERY /explain`, which returns the selected table-scan or
index plan for the same query document.

`HEAD /records` and `HEAD /records/{id}` reuse the GET status and representation headers without
transferring response content.

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

Successful WAL-backed mutations return `X-Txbase-Transaction-Id`. The single-table transaction
response also includes the same value as `transaction_id`; it is persisted in the table's
`.txbase.state` sidecar and resumes after restart or WAL recovery.

The batch is single-table and all operations run on a private copy before one commit. Cross-table atomicity and MVCC visibility are not provided.

`POST` creates a record, `PUT` replaces one, `PATCH` applies a partial update, and `DELETE` sets the DBF deletion marker.

`PATCH` also accepts the typed `$set`, `$unset`, and `$inc` operators.

Successful reads return a strong `ETag`; GET and HEAD accept `If-None-Match` for `304 Not Modified`, and
single-table state-changing routes accept optional `If-Match` and `If-None-Match` values for `412
Precondition Failed` without partial mutation.

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
txbase xbf report path/to/users.xbf
```

Copy a DBF and its detected memo, schema, transaction-state, and valid index sidecars:

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
  txbase --serve-catalog DIRECTORY [--bind ADDRESS]
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

It uses the sidecar's active-record and distinct-key statistics as a uniform equality estimate, can order single-field range candidates with a bounded histogram estimate, and can use a compound range candidate after an exact equality prefix.

When equality, range, and ordered candidates coexist, it compares table-scan and index work with a bounded integer cost based on active or exact candidate records, sidecar traversal, and remaining sort work.

This is not a full I/O, memory, cache, collation, or compound-range selectivity cost model.

For multi-key sorts, it can use a single-field index for the first sort key and sort only equal-key groups by the remaining keys; this is not compound-index support.

It can also use a per-field-direction compound index for a matching multi-key sort, including the complete reverse direction and an exact equality prefix.

`pack` removes logically deleted records and renumbers the remaining physical records.

`recall` restores one logically deleted record by its physical record number.

`xbf import` converts a loaded DBF table to a bounded XBF snapshot. `xbf export` converts only the
representable XBF subset to a DBF file and reports unsupported types or values as errors.
`XbfTable::dbf_export_report` inspects representability without writing and returns all discovered
field/record issues plus whether a schema sidecar is required.
The library API `XbfTable::to_dbf_with_schema` additionally returns sidecar JSON for representable
`primary`, `unique`, `not_null`, and bounded table-level `checks` constraints. `XbfTable::save_dbf_with_schema` and CLI
`xbf export --schema` journal the DBF, sibling `.txschema.json`, memo-sidecar state, and durable
`.txbase.state` commit ID through a recoverable `TXSE` export boundary. A later DBF read resumes an interrupted replacement and rejects
targets changed by another writer; it does not promise one physically atomic snapshot to external
legacy readers.

`backup` and `restore` validate the source first, then copy the DBF, detected memo and schema
sidecars, the durable `.txbase.state` commit ID, plus a present valid `.txidx` sidecar. A stale or malformed source index is rejected;
an absent source index removes an old destination index.

When a DBF language-driver byte is missing or untrusted, the read, schema, verify, pack, recall,
and server commands accept `--encoding NAME` for the four declared CJK codecs plus explicit-only
strict Shift_JIS, EUC-JP, GB18030, and ISO-2022-JP overrides.
The supported aliases are `windows-31j`/`cp932`, `gbk`/`cp936`, `euc-kr`/`cp949`, and
`big5`/`cp950`, `shift_jis`/`shift-jis`/`sjis`, `euc-jp`, `gb18030`, and
`iso-2022-jp`/`iso2022-jp`.
The invocation override takes precedence over `*.txschema.json`, is not persisted, and is visible
as the effective `encoding_override` and `encoding_metadata.effective` in schema output.
`encoding_metadata` also reports the declared codec and whether the source was the language driver,
an explicit override, or the fallback.

The DBF codec recognizes the four Visual FoxPro CJK driver IDs for Windows-31J/CP932, GBK/CP936,
EUC-KR/CP949, and Big5/CP950, plus the legacy dBASE aliases `0x13`, `0x4d`, `0x4e`, and `0x4f`
for CP932, CP936, CP949, and CP950.
Malformed reads use U+FFFD and writes reject unmappable or over-width values.
The explicit `Shift_JIS` override accepts ASCII, half-width Katakana, and JIS X 0208; CP932
extensions decode as U+FFFD and are rejected on write.
Pinned DBF fixtures cover the four Visual FoxPro CJK driver IDs and the legacy dBASE `0x4d` ID;
the alias tests cover the full legacy alias set. Pinned explicit-codec byte fixtures cover DBF
record decoding and write round-trips for all eight supported explicit codec names.

An optional `users.txschema.json` sidecar adds one-field `primary`, `unique`, and `not_null`
constraints, bounded composite `unique` keys, scalar `default` values for omitted inserts, and
bounded table-level query-predicate `checks`, and can explicitly select one of the supported CJK
codecs without changing legacy DBF bytes.
Loaded active records and every insert, replace, patch, and recall are checked against it;
catalog mutations also enforce `references: "TABLE.FIELD"` for non-null child values.

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
bash scripts/check-doc-translations.sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

CI runs these checks on Ubuntu, macOS, and Windows.

### Repository layout

```text
src/dbf/            DBF parsing, codecs, memo sidecars, maintenance, mutation, WAL, and tests
src/catalog.rs      Direct-child DBF discovery, table lookup, and catalog verification
src/catalog/journal.rs Catalog lock, journal, and crash recovery
src/catalog/transaction.rs Named-table transaction preparation and commit
src/index.rs        External scalar and compound-key index sidecar lifecycle
src/query.rs        JSON query execution and filter evaluation
src/query/          planner, ordering, validation, bounded join, and query-specific tests
src/query_path.rs   Dotted-path traversal and projection helpers
src/server.rs       HTTP routing, QUERY validation, and shared HTTP responses
src/server/records.rs DBF record routes and mutation persistence
src/server/etag.rs  HTTP representation validators and conditional requests
src/server/catalog.rs Catalog schema, named-table HTTP surface, and bounded join
src/server/catalog_transaction.rs Catalog cross-table transaction HTTP surface
src/server/explain.rs Query-plan explanation HTTP surface
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

- [Documentation index](docs/README.md)
- [Japanese documentation](docs/ja/README.md)
- [DBF and dBASE compatibility](docs/dbf-compatibility.md)
- [Multi-table catalog](docs/catalog.md)
- [Secondary-index sidecar](docs/indexes.md)
- [Query model](docs/query-model.md)
- [Aggregation model](docs/aggregation.md)
- [Join model](docs/joins.md)
- [Query planning and external vocabulary](docs/query-planning.md)
- [Mutation model](docs/mutation-model.md)
- [Firebase data-model and synchronization lessons](docs/firebase-model.md)
- [SQLite testing and quality model](docs/testing-quality.md)
- [Quality contract matrix](docs/quality-matrix.md)
- [HTTP method semantics and QUERY](docs/http-semantics.md)
- [Schema metadata and local constraints](docs/schema-metadata.md)
- [XBF v1 format draft](docs/xbf.md)
- [Roadmap and explicit non-goals](docs/roadmap.md)
- [Research index and source policy](docs/research.md)

## Todo

`xbf report` checks XBF-to-DBF type/value representability and reports whether a schema sidecar is needed, without writing files.

The roadmap is research-led and does not turn every compatibility idea into code.

Near-term work is to harden the current DBF, memo, WAL, query, and HTTP contracts with fixtures and failure tests.

Later phases may add runtime-specific async stream traits, a full cost-based index choice, full index-aware or cost-based merge join planning, additional aggregation stages, catalog-wide MVCC, additional CJK encodings, strict multi-file reader atomicity for XBF export, and object-storage commits.

See [docs/roadmap.md](docs/roadmap.md) for the phase boundaries and acceptance conditions.

## License

MIT. See [LICENSE](LICENSE).
