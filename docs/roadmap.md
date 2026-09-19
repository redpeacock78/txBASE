# Roadmap and explicit non-goals

This roadmap follows the supplied design note, `txBASE Roadmap: CJK Compatibility, XBF, and Database Evolution`.

It preserves the proposed phase order and marks implementation status explicitly.

Items marked future are not current features.

## 1. Positioning

txBASE is a file-native transactional database descended from the dBASE and xBase family.

DBF remains the compatibility and preservation surface.

The proposed XBF format is a native format for types and metadata that DBF cannot represent cleanly.

HTTP, JSON, MCP, and WASM are access or execution surfaces above the storage formats.

The long-term design point is portability for existing xBase data, not replacement of SQLite or PostgreSQL.

## 2. Current baseline

The repository currently provides:

- DBF parsing and writing for selected classic and Visual FoxPro fields.
- DBT and FPT memo or binary sidecar paths for the supported formats.
- JSON query execution with a small MongoDB-inspired predicate vocabulary.
- Bounded `$expr` field-to-field comparisons that remain on the table-scan reference path.
- HTTP `GET`, `QUERY`, `POST`, `PUT`, `PATCH`, and `DELETE` routes.
- File or memory WAL types with `TXOP`, `TXDP`, `TXDB`, and `TXDM` persistence paths.
- Startup recovery, stale-snapshot rejection, and an Ubuntu/macOS/Windows CI gate.
- Schema introspection, DBF verification, and validated DBF plus memo-sidecar copy commands.
- A directory catalog that discovers direct-child DBF tables, loads named tables, and verifies all discovered tables.
- A single-table transaction endpoint that applies multiple record operations through one snapshot/WAL commit.
- Strong table representation ETags on successful reads, GET/HEAD If-None-Match validation, and optional If-Match protection for single-table mutations and transactions.
- A bounded aggregation pipeline with zero or more `$match` stages before one `$group` stage using `$count`, integer `$sum`, numeric `$avg`, `$min`, and `$max`, plus final `$sort` and `$limit` stages over group output.
- A bounded local `inner`, `left`, `semi`, or `anti` equality join plus a bounded `cross` join over two catalog tables with qualified filtering and projection.
- A catalog HTTP server exposing table schemas, named-table records and plans, independent named-table mutations, and the bounded local join.
- Physical and sorted keyset cursors with a 1,000-record page cap.
- Borrowed and owned-snapshot query streams for incremental filter and projection over an in-memory table snapshot.
- Declared Visual FoxPro CJK driver support for Windows-31J/CP932, GBK/CP936, EUC-KR/CP949, and Big5/CP950.
- An optional `*.txschema.json` sidecar with one-field `primary`, `unique`, and `not_null` enforcement.
- Explicit sidecar and per-invocation overrides for those four CJK codecs plus strict Shift_JIS, EUC-JP, GB18030, and ISO-2022-JP, with normalized schema output.
- A rebuildable external scalar and compound-key index sidecar with equality and range candidate lookup, histogram-estimated range ordering, single-field and ordered-prefix traversal, per-field-direction compound-prefix sort traversal, equality-prefix candidate counting, uniform-statistics-ordered equality candidate intersection, path-aware planning, and DBF/memo freshness checks.
- A bounded XBF v1 codec, DBF-to-XBF conversion helper, bounded in-memory and schema-sidecar XBF-to-DBF export, durable snapshot path, generation-checked full-snapshot WAL recovery, and journaled schema-preserving file export with base-state conflict detection and DBF-read recovery.

The baseline intentionally does not include a full cost-based index model, an asynchronous streaming backpressure protocol, multiple or planned joins, transaction IDs or MVCC visibility, aggregation stages beyond bounded `$match`, `$group`, `$project`, final `$sort`, and final `$limit`, composite or cross-table constraints, strict multi-file reader atomicity for XBF export, object-storage commits, or distributed replication.

## 3. Phase 1: complete the small local DBMS

This phase keeps the database local and makes its operational boundary useful before adding edge or distributed behavior.

### Candidate scope

- Schema introspection.
- A multi-table catalog boundary.
- Secondary-index maintenance and query planning.
- An asynchronous backpressure protocol for long-lived streams.
- Transaction IDs and independently visible multi-record snapshots.
- `PACK` and `RECALL` maintenance operations.
- `verify`, `backup`, and `restore` tooling.

`PACK` must define whether memo blocks and indexes are rebuilt or left as reclaimable orphan space.

`RECALL` must define whether it restores only the deletion marker or also participates in an indexed or transactional update.

### Completion conditions

Schema introspection, verification, copy tooling, `PACK`, `RECALL`, and the first directory-catalog boundary are implemented as the first Phase 1 slice.

The catalog currently derives table identity from direct-child DBF filenames and does not persist a separate manifest.

It provides table discovery, named table loading, schema output, per-table verification, and
independent named-table HTTP mutations that reuse the single-table persistence boundary.

It does not yet provide relationships, cross-table index coordination, transaction IDs, or MVCC visibility.

The index sidecar foundation is implemented for scalar and per-field-direction compound keys, exact equality and range candidate lookup, histogram-estimated range ordering, single-field ordered traversal, ordered-prefix traversal for multi-key sorts, compound-prefix traversal for compatible mixed or uniform directions, equality-prefix candidate counting, uniform-statistics-ordered equality candidate intersection, path-aware planning, stale detection, explicit rebuild, and WAL-backed DBF/index recovery after normal persistence.

It does not yet support a full cost model, collation-aware planning, transaction IDs, or MVCC visibility.

The remaining items need a public contract, malformed-input behavior, crash behavior, and a fixture or deterministic test.

The current cursor slice supports physical-record pagination and sorted keyset pagination with
`page_size` and `cursor`.
It rejects `skip`, caps pages at 1,000 records, and physical pages scan only until the requested
page and one look-ahead record are found.
Sorted pages materialize matching record references before applying the keyset boundary.
The public `query::stream_query` iterator covers unsorted filter and projection without
materializing matching records.
`query::stream_query_snapshot` owns a clone of the loaded table to keep records stable while the
caller consumes the pull-based iterator.
An asynchronous backpressure protocol remains a later contract.

An index is not complete for the broader roadmap until insert, update, logical delete, recovery, stale-index detection, rebuild behavior, cost-model limits, direction compatibility, and crash behavior are specified and tested together.

A multi-record transaction is not complete until commit, rollback, crash recovery, and visibility rules are tested together.

The single-table transaction slice covers one DBF table through one snapshot/WAL persistence path.
The catalog transaction slice prepares named operations across multiple DBFs under a catalog
lock, commits DBF and changed-sidecar images through a directory journal, and recovers an
incomplete prepare before the next catalog read. Transaction IDs and MVCC visibility remain
future work.

## 4. Phase 2: expand the query model

The current record scan remains the reference execution path while the query model grows.

### Candidate scope

- Additional aggregation stages and accumulator expressions.
- Full expression evaluation beyond the bounded `$expr` comparison form.
- Joins.
- Constraints.
- Full range, histogram-based, and mixed-direction compound cost planning.

The first join slice is local and bounded.
It implements one or more equality conditions and uses an in-memory right-side map.
Future join work must define planner selection, streaming, multiple joins, and broader null or
missing field semantics before adding broader query surfaces.

Distributed joins and distributed transactions remain later features.

Aggregation must define missing, null, numeric overflow, and memory-limit behavior before it is added to the HTTP API.

The current aggregation slice also permits one bounded `$project` over group output before the
final sort and limit. It reuses the existing include/exclude projection contract.

The planner explanation boundary is implemented by `explain_query_at` and `QUERY /explain`.
Full cost-based choice remains future work.

The first constraint slice is an optional schema sidecar.
It enforces one-field `primary`, `unique`, and `not_null` properties on active records and mutation candidates without changing legacy DBF bytes.
Composite keys, `CHECK`, `FOREIGN KEY`, `DEFAULT`, and schema migration remain future work.

## 5. Phase 3: legacy international compatibility

Character decoding and sorting are separate contracts.

The phase must preserve DBF byte widths and reject ambiguous or unrepresentable writes.

### Candidate scope

- CP932 or Windows-31J.
- A strict Shift_JIS distinction.
- EUC-JP.
- GBK and GB18030.
- Big5.
- CP949 and EUC-KR.
- Explicit encoding overrides.
- Collation.
- Additional external fixtures.

Every encoding needs a declared name, byte-width rule, round-trip fixture, invalid-byte behavior, and comparison policy.

The first declared-driver slice covers the four Visual FoxPro CJK IDs listed in the DBF
compatibility document.
It uses replacement characters for malformed reads and rejects unmappable or over-width writes.
The sidecar and path-oriented CLI now provide explicit overrides for those four codecs plus strict
Shift_JIS, EUC-JP, GB18030, and ISO-2022-JP; none of the latter four claims a DBF language-driver
mapping.
Schema output exposes declared and effective names plus the interpretation source.
Strict Shift_JIS accepts ASCII, half-width Katakana, and JIS X 0208, while CP932 extensions are
replaced on read or rejected on write. Collation and broader external fixtures remain future work.

An override must be visible in schema or command output so a reader can reproduce the same interpretation.

## 6. Phase 4: native XBF

XBF is a separately versioned native format, not a silent DBF extension.

The v1 wire contract is drafted in [XBF v1 format draft](xbf.md). A bounded codec, DBF-to-XBF conversion helper, bounded in-memory and schema-sidecar XBF-to-DBF export, representability reporting, durable snapshot path, generation-checked full-snapshot WAL recovery, and a journaled `TXSE` schema-preserving file export exist for that draft. `TXSE` records base bytes, recovers interrupted DBF/schema/memo-sidecar replacement on the next DBF read, and rejects external target changes. Strict multi-file reader atomicity and object-storage commit semantics remain future work.

The proposed magic is `TXBF`.

The proposed media type is `application/vnd.txbase.xbf` if registration remains appropriate after the format is specified.

### Candidate scope

- XBF v1 specification.
- UTF-8 text.
- Explicit `NULL`.
- Variable-length strings.
- Modern integer types.
- Timestamps.
- Binary and blob values.
- Lossless DBF to XBF conversion.
- Validated XBF to DBF export.

The first format must define headers, checksums, corruption handling, version negotiation, and downgrade behavior.

DBF compatibility must remain an explicit import or export path.

## 7. Phase 5: edge storage

The range-oriented local storage abstraction is a useful starting point, but object storage needs immutable objects and conditional manifest updates.

### Candidate scope

- Storage-backend redesign.
- A WASM-compatible core.
- An R2 or object-storage adapter.
- Manifest compare-and-swap commits.
- Immutable pages.
- Generation snapshots.

The object-storage model needs a consistency contract, orphan-page cleanup policy, retry behavior, and a deterministic local fixture.

WASM must reuse the DBF or XBF codec and query contracts instead of creating a second database implementation.

## 8. Phase 6: advanced database features

Distributed behavior comes after the local and edge contracts are stable.

### Candidate scope

- Full MVCC.
- Change data capture.
- Persistent WAL history.
- Replication.
- Raft or another explicitly selected authority protocol.
- Snapshot installation.
- Follower reads.
- Distributed partitioning.

These features require an authority model, conflict semantics, schema-version handling, recovery procedures, and observability.

No consensus or multi-region feature is implied by the current exclusive table lock.

## 9. XBF and shared upper layers

The DBF and XBF codecs should share query, transaction, HTTP, and storage interfaces where their semantics match.

The format-specific layer should own byte layout, field conversion, checksums, and recovery records.

This prevents XBF from becoming a second unrelated database implementation.

## 10. File granularity rule

A file should be split when its code has different ownership, failure behavior, test fixtures, or change cadence.

The current repository already splits DBF codecs, memo handling, WAL deltas, query validation, server ranges, and test groups along those boundaries.

Further splitting should follow a real boundary.

The number of files is not a quality metric by itself.

## 11. Explicit non-goals for the current slice

- Full dBASE command-language compatibility.
- Full Visual FoxPro runtime compatibility.
- MongoDB wire or BSON compatibility.
- Firebase authentication, security rules, listeners, or offline clients.
- SQLite-level test volume or coverage claims.
- Automatic CJK conversion when the declared encoding is ambiguous.
- Full cost-based planners, joins, aggregation, MVCC, durable XBF, object-storage, or distributed code without a contract and end-to-end test.

The current index slice is intentionally local: compatible compound directions and equality-prefix candidate choice are implemented, while a full cost model and cross-table coordination remain future work.
