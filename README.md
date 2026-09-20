<!-- English README -->

<div align="center">

[![CI](https://github.com/redpeacock78/txBASE/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/redpeacock78/txBASE/actions/workflows/ci.yml)
[![License](https://img.shields.io/github/license/redpeacock78/txBASE)](LICENSE)

# txBASE

A small Rust database prototype that keeps the dBASE DBF file format at its center.

</div>

<table>
<tr>
<td><a href="README.md">English</a></td>
<td><a href="README.ja.md">日本語</a></td>
</tr>
</table>

## What txBASE provides

txBASE reads and writes selected dBASE and Visual FoxPro fields while keeping the original DBF representation visible.

- JSON and HTTP access for one table or a directory of tables.
- File-backed WAL and memo sidecars for recoverable mutations.
- Bounded queries, aggregation, local catalog joins, and external scalar or compound indexes.
- XBF v1 snapshots and representability-aware DBF export.
- Explicit CJK codec selection without changing legacy DBF bytes.

The detailed compatibility and behavior contracts live in the [documentation index](docs/README.md).

## How to use

### Quick start

Read active records from a DBF file:

```bash
cargo run -- path/to/users.dbf
```

Start the single-table HTTP server:

```bash
cargo run -- --serve path/to/users.dbf
```

Start the catalog server for named tables and bounded joins:

```bash
cargo run -- --serve-catalog path/to/database
```

The default listener is `127.0.0.1:8080`. Use `--bind ADDRESS` to choose another address.

### Read and query

```bash
curl -s http://127.0.0.1:8080/records | jq
curl -s http://127.0.0.1:8080/records/1 | jq

curl -i -X QUERY \
  -H 'Content-Type: application/json' \
  -d '{"filter":{"AGE":{"$gte":20}},"sort":{"NAME":1},"limit":10}' \
  http://127.0.0.1:8080/records
```

`GET` and `HEAD` use one-based physical DBF record numbers. `QUERY /records` accepts bounded `filter`, `sort`, `projection`, `collation`, `skip`, `limit`, `page_size`, `cursor`, and `aggregate` controls.

`page_size` returns an opaque cursor. Physical cursors follow record order; sorted cursors use the declared sort and a physical-record tie-breaker. New cursors are bound to the current table representation, while legacy unbound cursor formats remain accepted for compatibility.

`QUERY /records/stream` returns filter/projection/skip/limit results as chunked `application/x-ndjson`. `QUERY /explain` reports the selected table-scan or index plan. The library exposes the corresponding borrowed, snapshot, and bounded streaming iterators.

The catalog server adds `GET /catalog`, named-table read and mutation routes, `QUERY /{table}/records`, `QUERY /{table}/explain`, `QUERY /{table}/records/stream`, and the bounded local join at `QUERY /join`. Catalog `POST /transaction` commits named-table mutations through a catalog journal.

See the [query model](docs/query-model.md), [aggregation model](docs/aggregation.md), [join model](docs/joins.md), and [query planning](docs/query-planning.md) for the exact boundary.

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

`POST`, `PUT`, `PATCH`, and `DELETE` create, replace, update, and logically delete records. `PATCH` also accepts typed `$set`, `$unset`, and `$inc` operators.

Single-table `POST /transaction` applies multiple operations to a private copy and commits one snapshot/WAL boundary. Catalog `POST /transaction` provides the corresponding cross-table catalog journal boundary. Neither boundary provides historical row versions or MVCC visibility.

Successful mutations return `X-Txbase-Transaction-Id` and persist the commit ID in a state sidecar. Strong `ETag`, `If-Match`, and `If-None-Match` conditions reject stale writes without partial mutation. The [mutation model](docs/mutation-model.md) describes the WAL and recovery contract.

### Inspect and maintain

Read-only schema, catalog, index, XBF, and WAL inspection:

```bash
txbase schema path/to/users.dbf
txbase verify path/to/users.dbf
txbase catalog path/to/database
txbase verify-catalog path/to/database
txbase index verify path/to/users.dbf
txbase xbf report path/to/users.xbf
txbase wal inspect path/to/users.txbase.wal
```

`wal inspect` never creates or truncates the WAL. It reports the file size, valid byte boundary, complete record LSN and payload length, and whether the final record is torn.

Index lifecycle and maintenance:

```bash
txbase index build path/to/users.dbf NAME AGE
txbase index build-compound path/to/users.dbf by_name_age NAME AGE
txbase index rebuild path/to/users.dbf
txbase pack path/to/users.dbf
txbase recall path/to/users.dbf 2
```

`index verify` rejects stale sidecars; `index rebuild` is the explicit repair path. `pack` removes logically deleted records, and `recall` restores one by physical record number.

XBF conversion and sidecar-aware file transfer:

```bash
txbase xbf import path/to/users.dbf path/to/users.xbf
txbase xbf export path/to/users.xbf path/to/users.dbf
txbase xbf export path/to/users.xbf path/to/users.dbf --schema
txbase backup path/to/users.dbf backups/users.dbf
txbase restore backups/users.dbf path/to/users.dbf
```

`xbf report` checks representability without writing. `xbf export --schema` journals the DBF, schema, memo, and state sidecars through a recoverable `TXSE` boundary; it does not promise one physically atomic snapshot to external legacy readers. `backup` and `restore` validate and copy supported memo, schema, state, and valid index sidecars.

### File and sidecar boundaries

For `users.dbf`, txBASE may discover sibling `.dbt` or `.fpt` memo data, `users.txschema.json` constraint and encoding metadata, `users.txbase.wal`, `users.txbase.state`, and the valid `users.txidx` scalar or compound index sidecar.

When the DBF language-driver byte is missing or untrusted, read, schema, verify, pack, recall, and server commands accept `--encoding NAME`. Supported aliases include `windows-31j`/`cp932`, `gbk`/`cp936`, `euc-kr`/`cp949`, `big5`/`cp950`, strict `shift_jis`/`shift-jis`/`sjis`, `euc-jp`, `gb18030`, and `iso-2022-jp`/`iso2022-jp`.

Malformed reads use U+FFFD. Writes reject unmappable or over-width values. The [DBF compatibility](docs/dbf-compatibility.md), [schema metadata](docs/schema-metadata.md), and [XBF v1](docs/xbf.md) documents define the supported formats and sidecar contracts.

## Install

The CI matrix covers Ubuntu, macOS, and Windows with the stable Rust toolchain.

Install Rust through [rustup](https://rustup.rs/), then build the binary:

```bash
cargo build --release
```

The resulting binary is `target/release/txbase`.

## Development

### Quality gates

CI runs the following checks on Ubuntu, macOS, and Windows:

```bash
bash scripts/check-doc-translations.sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

### Repository layout

```text
src/dbf/            DBF parsing, codecs, memo sidecars, maintenance, mutation, WAL, and tests
src/catalog.rs      Direct-child DBF discovery, table lookup, and catalog verification
src/catalog/        Catalog locks, journals, recovery, and named-table transactions
src/index.rs        External scalar and compound-key index sidecar lifecycle
src/query.rs        JSON query execution and filter evaluation
src/query/          Planner, ordering, validation, bounded joins, and query tests
src/server.rs       HTTP routing, QUERY validation, and shared responses
src/server/         Record, catalog, ETag, explain, and transaction routes
src/transaction/    File or memory WAL and snapshot transactions
src/xbf/            Bounded XBF v1 codec, DBF conversion, persistence, WAL, and tests
tests/fixtures/     External-format fixtures
tests/corpus/       Malformed DBF, memo, WAL, and JSON inputs
docs/               Topic-specific specifications and design notes
```

Files are split when ownership, failure behavior, fixtures, or change cadence differ. A crate split waits for a real build or ownership boundary.

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

## Current boundary and roadmap

The current implementation prioritizes bounded, recoverable local operations over an unbounded database server.

The roadmap still leaves runtime-specific async stream traits, full cost-based index and join planning, broader aggregation, catalog-wide MVCC, locale-aware CJK collation, broader upstream CJK fixtures, strict multi-file reader atomicity for XBF export, object-storage commits, and distributed replication as future work.

See [docs/roadmap.md](docs/roadmap.md) for acceptance conditions and [docs/research.md](docs/research.md) for the source and fixture policy.

## License

MIT. See [LICENSE](LICENSE).
