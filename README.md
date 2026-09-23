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
- A deterministic XBF object-store manifest boundary with generation CAS and recovery, backed by memory or a durable filesystem store.
- Explicit CJK codec selection without changing legacy DBF bytes.
- A host-independent DBF WASM core with the shared bounded query and single-operation or atomic-batch mutation contracts.
- A runtime-neutral asynchronous object-store and manifest contract for future worker and WASI hosts.
- An executor-neutral asynchronous query-stream polling contract for in-memory stream adapters.
- A native threaded asynchronous query-stream adapter with bounded backpressure and drop cancellation.
- A committed single-table change-data-capture sidecar with WAL recovery and an inspection CLI.
- A committed catalog change-data-capture sidecar with atomic multi-table events and an inspection CLI.
- An opt-in coarse-grained serializable transaction boundary for one DBF table.
- An opt-in coarse-grained serializable transaction boundary for a catalog's discovered tables.

The detailed compatibility and behavior contracts live in the [documentation index](docs/README.md).

## How to use

### Quick start

Read active records from a DBF file:

```bash
cargo run -- read path/to/users.dbf
```

Start the single-table HTTP server:

```bash
cargo run -- serve path/to/users.dbf
```

Start the catalog server for named tables and bounded joins:

```bash
cargo run -- serve-catalog path/to/database
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

`page_size` returns an opaque cursor.
Physical cursors follow record order.
Sorted cursors use the declared sort and a physical-record tie-breaker.
New cursors are bound to the current table representation, while legacy unbound cursor formats remain accepted for compatibility.

`QUERY /records/stream` returns filter/projection/skip/limit results as chunked `application/x-ndjson`.
`QUERY /explain` reports the selected table-scan or index plan and, when a valid index sidecar is available, its deterministic row-equivalent cost breakdown.
The library exposes the corresponding borrowed, snapshot, and bounded streaming iterators.

The catalog server adds `GET /catalog`, named-table read and mutation routes, `QUERY /{table}/records`, `QUERY /{table}/explain`, `QUERY /{table}/records/stream`, and the bounded local join at `QUERY /join`.
Catalog `POST /transaction` commits named-table mutations through a catalog journal.

`QUERY /join` supports bounded `inner`, `left`, `right`, `full`, `semi`, `anti`, and `cross` joins.
Large direct equality joins can use fresh compatible ordered indexes for merge execution.
The planner falls back to bounded hash or index-probe paths when that merge path is unavailable or more expensive.

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

curl -i -X PATCH \
  -H 'Content-Type: application/merge-patch+json' \
  -d '{"NAME":"Alicia","AGE":null}' \
  http://127.0.0.1:8080/records/3

curl -i -X PATCH \
  -H 'Content-Type: application/json-patch+json' \
  -d '[{"op":"replace","path":"/NAME","value":"Alicia"}]' \
  http://127.0.0.1:8080/records/3

curl -i -X DELETE http://127.0.0.1:8080/records/3
```

`POST`, `PUT`, `PATCH`, and `DELETE` create, replace, update, and logically delete records.
`PATCH` accepts the local `application/json` update document, `application/merge-patch+json` object patches, and bounded `application/json-patch+json` operation arrays.
Merge Patch recursively merges objects, treats `null` as field removal, and replaces arrays or scalar values.
JSON Patch supports the RFC 6902 `add`, `remove`, `replace`, `test`, `move`, and `copy` operations with RFC 6901 JSON Pointer paths.
It also accepts typed `$set`, `$unset`, and `$inc` operators in the local JSON document.

Single-table `POST /transaction` applies multiple operations to a private copy and commits one snapshot/WAL boundary.
The Rust API also exposes `DbfTransaction::begin_serializable` for an opt-in coarse-grained serializable boundary that holds the exclusive table lock from begin through commit or rollback.
`Catalog::begin_serializable` provides the corresponding catalog-wide Rust boundary: it holds the catalog write lock and every discovered table lock, applies named operations to private copies, and commits them through one catalog journal.
It captures the DBF name and file-name set at begin and rejects a changed set at commit.
Single-table mutations retain table-scoped historical snapshots through the `mvcc` CLI.
Catalog `POST /transaction` also retains one full-image historical snapshot for every discovered
table. `Catalog::from_path_at` and `mvcc catalog` read one consistent catalog commit. Catalog
HTTP read routes can select the same retained image with `?at=TRANSACTION_ID`; historical
mutations remain rejected.

Successful mutations return `X-Txbase-Transaction-Id` and persist the commit ID in a state sidecar.
Strong `ETag`, `If-Match`, and `If-None-Match` conditions reject stale writes without partial mutation.
The [mutation model](docs/mutation-model.md) describes the WAL and recovery contract.
The [MVCC document](docs/mvcc.md) describes historical snapshot visibility.

### Inspect and maintain

Read-only schema, catalog, index, XBF, and WAL inspection:

```bash
txbase schema path/to/users.dbf
txbase schema apply path/to/users.dbf path/to/users.txschema.candidate.json
txbase verify path/to/users.dbf
txbase catalog path/to/database
txbase verify-catalog path/to/database
txbase mvcc list path/to/users.dbf
txbase mvcc read path/to/users.dbf 1
txbase mvcc row path/to/users.dbf 1
txbase mvcc row-at path/to/users.dbf 1 1 1
txbase mvcc gc path/to/users.dbf --keep 5 --keep-rows 10
txbase mvcc catalog list path/to/database
txbase mvcc catalog read path/to/database 1
txbase mvcc catalog gc path/to/database --keep 5
txbase index verify path/to/users.dbf
txbase xbf report path/to/users.xbf
txbase wal inspect path/to/users.txbase.wal
txbase cdc path/to/users.dbf
txbase cdc path/to/users.dbf --after 10
txbase cdc catalog path/to/database --after 10
```

`wal inspect` never creates or truncates the WAL. It reports the file size, valid byte boundary, complete record LSN and payload length, and whether the final record is torn.

`cdc` prints committed table or atomic catalog change events as JSON. `--after` is an exclusive transaction-ID cursor and does not acknowledge or retain consumer state.

`mvcc gc` and `mvcc catalog gc` retain the newest positive count of full-image snapshots and
rewrite only their MVCC history sidecar through a synced temporary file. `mvcc gc --keep-rows COUNT`
also retains up to `COUNT` older versions for each physical row before the oldest retained full snapshot.
The current DBF and catalog state are unchanged; removed full-snapshot IDs are no longer readable.

Index lifecycle and maintenance:

```bash
txbase index build path/to/users.dbf NAME AGE
txbase index build-compound path/to/users.dbf by_name_age NAME AGE
txbase index rebuild path/to/users.dbf
txbase pack path/to/users.dbf
txbase recall path/to/users.dbf 2
```

`index verify` rejects stale sidecars; `index rebuild` is the explicit repair path. `pack` removes logically deleted records, compacts referenced DBT/FPT memo blocks, refreshes an existing index sidecar, and persists the DBF and memo snapshot through one WAL boundary. `recall` restores one logically deleted record by physical record number.

XBF conversion and sidecar-aware file transfer:

```bash
txbase xbf import path/to/users.dbf path/to/users.xbf
txbase xbf export path/to/users.xbf path/to/users.dbf
txbase xbf export path/to/users.xbf path/to/users.dbf --schema
txbase backup path/to/users.dbf backups/users.dbf
txbase restore backups/users.dbf path/to/users.dbf
```

`xbf report` checks representability without writing.
`xbf export --schema` journals the DBF, schema, memo, and state sidecars through a recoverable `TXSE` boundary.
It does not promise one physically atomic snapshot to external legacy readers.
`backup` and `restore` validate and copy supported memo, schema, state, and valid index sidecars.

### File and sidecar boundaries

For `users.dbf`, txBASE may discover sibling `.dbt` or `.fpt` memo data, `users.txschema.json` constraint and encoding metadata, `users.txbase.wal`, `users.txbase.state`, and the valid `users.txidx` scalar or compound index sidecar.

When the DBF language-driver byte is missing or untrusted, read, schema, verify, pack, recall, and server commands accept `--encoding NAME`.
Supported aliases include `windows-31j`/`cp932`, `gbk`/`cp936`, `euc-kr`/`cp949`, `big5`/`cp950`, strict `shift_jis`/`shift-jis`/`sjis`, `euc-jp`, `gb18030`, and `iso-2022-jp`/`iso2022-jp`.

Malformed reads use U+FFFD.
Writes reject unmappable or over-width values.
The [DBF compatibility](docs/dbf-compatibility.md), [schema metadata](docs/schema-metadata.md), and [XBF v1](docs/xbf.md) documents define the supported formats and sidecar contracts.

## Install

The CI matrix covers Ubuntu, macOS, and Windows with the stable Rust toolchain.

Install Rust through [rustup](https://rustup.rs/), then build the binary:

```bash
cargo build --release
```

The resulting binary is `target/release/txbase`.

## Development

### Quality gates

CI runs the documentation checks on Ubuntu and the Rust checks on Ubuntu, macOS, and Windows:

```bash
bun install --frozen-lockfile
bun run lint:docs
bash scripts/check-doc-translations.sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo check --lib --target wasm32-unknown-unknown
cargo test --all-targets --all-features
```

### Repository layout

```text
src/dbf/            DBF parsing, codecs, memo sidecars, maintenance, mutation, WAL, and tests
src/catalog.rs      Direct-child DBF discovery, table lookup, and catalog verification
src/catalog/        Catalog locks, journals, recovery, and named-table transactions
src/catalog/mvcc.rs Catalog-wide commit-level historical snapshots
src/index.rs        External scalar and compound-key index sidecar lifecycle
src/query.rs        JSON query execution and filter evaluation
src/query/          Planner, ordering, validation, bounded joins, and query tests
src/wasm.rs         Host-independent DBF WASM boundary and bindings
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
- [Change data capture](docs/change-data-capture.md)
- [Multi-table catalog](docs/catalog.md)
- [Secondary-index sidecar](docs/indexes.md)
- [Query model](docs/query-model.md)
- [CLI command design](docs/cli.md)
- [Asynchronous query streaming](docs/async-streaming.md)
- [Aggregation model](docs/aggregation.md)
- [Join model](docs/joins.md)
- [Query planning and external vocabulary](docs/query-planning.md)
- [Mutation model](docs/mutation-model.md)
- [Snapshot transactions](docs/transactions.md)
- [Firebase data-model and synchronization lessons](docs/firebase-model.md)
- [SQLite testing and quality model](docs/testing-quality.md)
- [Quality contract matrix](docs/quality-matrix.md)
- [HTTP method semantics and QUERY](docs/http-semantics.md)
- [Schema metadata and local constraints](docs/schema-metadata.md)
- [XBF v1 format draft](docs/xbf.md)
- [MVCC and historical snapshots](docs/mvcc.md)
- [Roadmap and explicit non-goals](docs/roadmap.md)
- [Research index and source policy](docs/research.md)
- [Edge storage and object-store commits](docs/edge-storage.md)
- [WASM and worker host boundary](docs/wasm.md)
- [Distributed evolution](docs/distributed-evolution.md)

## Current boundary and roadmap

The current implementation prioritizes bounded, recoverable local operations over an unbounded database server.

The roadmap still leaves the following areas as future work:

- Worker/WASI-specific `AsyncQueryStream` timeout, transport, cancellation, and asynchronous-storage behavior.
- Cardinality- and materialization-aware costing through chained joins, filesystem- and cache-aware merge planning.
- Broader aggregation.
- Predicate-level locking and distributed serializable coordination.
- Locale-aware CJK collation.
- Broader upstream CJK fixtures.
- Strict multi-file reader atomicity for XBF export.
- Cloud object-storage adapters and retention policy.
- Worker/WASI runtime adapters and asynchronous WASM storage.
- Distributed replication.

See [docs/roadmap.md](docs/roadmap.md) for acceptance conditions and [docs/research.md](docs/research.md) for the source and fixture policy.

## License

MIT. See [LICENSE](LICENSE).
