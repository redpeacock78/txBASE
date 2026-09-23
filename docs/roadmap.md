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
- JSON query execution with a bounded MongoDB-inspired predicate vocabulary covering comparison, membership, logical, array, and field-expression predicates.
- Bounded `$expr` boolean trees over field-to-field comparison leaves, including numeric `$abs`, `$add`, `$subtract`, `$multiply`, `$divide`, and `$mod` operands, that remain on the table-scan reference path.
- HTTP `GET`, `QUERY`, `POST`, `PUT`, `PATCH`, and `DELETE` routes, including bounded JSON Merge Patch and JSON Patch for record updates.
- File or memory WAL types with `TXOP`, `TXTI`, `TXDP`, `TXDB`, and `TXDM` persistence paths.
- A read-only `wal inspect` command that reports complete record boundaries and torn tails without mutating the WAL.
- A low-level snapshot transaction engine whose `TransactionId` sequence resumes from retained
  memory or file WAL commit/rollback records, plus durable DBF single-table WAL commit IDs exposed
  by the HTTP mutation boundary.
- Startup recovery, stale-snapshot rejection, and an Ubuntu/macOS/Windows CI gate.
- Schema introspection, DBF verification, sidecar-aware backup and restore, and validated DBF plus memo-sidecar copy commands.
- A directory catalog that discovers direct-child DBF tables, loads named tables, and verifies all discovered tables.
- A durable catalog-journal commit ID for multi-table mutation transactions.
- Persistent table-scoped MVCC snapshots for single-table mutations, with `mvcc list`, `mvcc read`, `mvcc row`, and `mvcc row-at` CLI commands.
- Catalog-wide commit-level MVCC snapshots for all discovered tables, with `Catalog::from_path_at` and `mvcc catalog` CLI commands.
- Catalog HTTP schema, named-table, query, stream, explain, and join reads can select one retained catalog image with `?at=<transaction ID>`; historical execution does not reuse current index sidecars and catalog mutations remain rejected.
- A `CatalogReadTransaction` API that captures all discovered tables into one process-local, read-only cross-table image, releases its locks after capture, and executes the existing bounded join contract without live index sidecars.
- A single-table transaction endpoint that applies multiple record operations through one snapshot/WAL commit.
- A public `DbfTransaction` API that exposes the same optimistic single-table snapshot, query, stale-source check, commit, and rollback boundary used by the HTTP transaction route.
- An opt-in coarse-grained serializable `DbfTransaction::begin_serializable` boundary that holds the exclusive table lock from begin through commit or rollback.
- An opt-in coarse-grained serializable `Catalog::begin_serializable` boundary that holds the catalog write lock and every discovered table lock from begin through commit, rollback, or drop, then publishes private table copies through one catalog journal.
- Strong table and catalog representation ETags on successful reads, GET/HEAD If-None-Match validation, mutation-side If-None-Match validation for single-table, named-table, and catalog-wide transaction routes, and optional If-Match protection for single-table mutations, named-table mutations, and catalog-wide transactions.
- A bounded aggregation pipeline with zero or more input `$match` and top-level-array `$unwind` stages, at most one input `$set` or `$addFields` stage in total, at most one input `$project`, `$sort`, `$skip`, and `$limit` stage each, and one terminal `$count` or `$distinct` stage, or one `$group` stage using `$count`, bounded numeric-expression `$sum`, `$avg`, `$stdDevPop`, and `$stdDevSamp`, `$min`, `$max`, `$first`, `$last`, `$push`, and `$addToSet`, followed by bounded group-output `$match` stages, one optional `$project`, and final `$sort`, `$skip`, and `$limit` stages. Input stages execute in listed order. Input `$set` and `$addFields` preserve existing fields and compute top-level fields from bounded field, literal, null-coalescing, and numeric expressions against the stage-input snapshot. Input `$project` reuses the 0/1 query projection contract and materializes fields before later stages. `$unwind` supports the top-level document options `includeArrayIndex` and `preserveNullAndEmptyArrays`, preserves input and array order, rejects non-array values, and caps all emitted records at 10,000.
- A bounded local `inner`, `left`, `right`, `full`, `semi`, or `anti` equality join plus a bounded `cross` join over one or more catalog tables with qualified filtering and projection.
- Direct and chained equality-join cost models that compare hash and index-nested-loop paths with exact pre-filter key-cardinality estimates, materialized row-width work, logical input page reads, and logical index-sidecar page reads, plus ordered-merge paths for direct joins and eligible chained stages.
- A catalog HTTP server exposing table schemas, named-table records and plans, independent named-table mutations, and the bounded local join.
- Physical and sorted keyset cursors with a 1,000-record page cap.
- A bounded `unicode-lowercase` sort collation with cursor-boundary validation and a safe table-scan fallback.
- Borrowed and owned-snapshot query streams for incremental filter and projection over an in-memory table snapshot.
- A bounded thread-backed snapshot stream whose producer applies channel backpressure and stops when its consumer is dropped.
- A runtime-neutral `AsyncQueryStream` polling boundary for long-lived query streams, immediate in-memory implementations, and a native `ThreadedQueryStream` adapter with bounded backpressure, waker notification, and drop cancellation.
- Declared Visual FoxPro CJK driver support for Windows-31J/CP932, GBK/CP936, EUC-KR/CP949, and Big5/CP950, plus the legacy dBASE aliases `0x13`, `0x4d`, `0x4e`, and `0x4f`.
- An optional `*.txschema.json` sidecar with one-field `primary`, `unique`, and `not_null` enforcement, bounded composite `primary` and `unique` keys, scalar defaults for omitted inserts, bounded table-level `checks` predicates, and catalog-scoped scalar and composite foreign-key validation with restrict, cascade, and set-null actions.
- Explicit sidecar and per-invocation overrides for those four CJK codecs plus strict Shift_JIS, EUC-JP, GB18030, and ISO-2022-JP, with normalized schema output.
- A rebuildable external scalar and compound-key index sidecar with scalar and compound equality, compound equality-prefix, range, and compound equality-prefix range candidate lookup, histogram-estimated range ordering, single-field and ordered-prefix traversal, per-field-direction compound-prefix sort traversal, equality-prefix candidate counting, uniform-statistics ordering for equality candidates, single-index versus intersection cost choice, bounded cost choice based on candidate rows, index traversal, logical 4 KiB index and DBF page reads, and sort work with non-selective-index table-scan fallback, plus deterministic row-equivalent explanation fields for candidate record reads and filter evaluations, path-aware planning, and DBF/memo freshness checks.
- Durable table-local row history stored with MVCC prepare/commit records, epoch-separated physical row IDs, retained row reads, baseline reconstruction during full-image GC, and optional independent per-row retention through `mvcc gc --keep-rows`.
- A bounded XBF v1 codec, a DBF-to-XBF conversion helper that preserves representable field-level schema constraints and rejects unsupported metadata, bounded in-memory and schema-sidecar XBF-to-DBF export, durable snapshot path, generation-checked full-snapshot WAL recovery, and journaled schema-preserving file export with base-state conflict detection, index-sidecar recovery, and DBF-read recovery.
- A versioned host-independent DBF WASM core with byte-in/byte-out snapshots, the shared bounded query and single-operation or atomic-batch mutation contracts, a `wasm-bindgen` wrapper, and a `wasm32-unknown-unknown` CI compile check.
- A runtime-neutral `AsyncObjectStore` primitive contract, `AsyncObjectTable` manifest protocol, and synchronous-store adapter that exposes the five object operations as futures without selecting an executor.
- A committed single-table change-data-capture sidecar with ordered `TXCD` events, WAL recovery, idempotent publication, torn-tail repair, backup and restore support, a read-only API and CLI cursor, and a bounded HTTP read route.
- A committed catalog change-data-capture sidecar with ordered `TXCC` envelopes for explicit multi-table catalog transactions, journal recovery, idempotent publication, a read-only API and CLI cursor, and a bounded HTTP read route.

The baseline intentionally does not include the following:

- Filesystem- and cache-aware merge join costing.
- Worker/WASI-specific timeout, transport, cancellation, and asynchronous-storage semantics, and remote object-store adapters.
- Predicate-level locking and distributed serializable coordination.
- Aggregation stages or accumulators beyond bounded input `$match`, `$unwind` with its documented top-level options, `$set`/`$addFields` with its documented expression subset, `$project`, `$sort`, `$skip`, and `$limit`, group-output `$match`, `$count`, `$distinct`, and `$group` with bounded numeric-expression `$sum`, `$avg`, `$stdDevPop`, and `$stdDevSamp`, `$min`, `$max`, `$first`, `$last`, `$push`, and `$addToSet`.
- Deferred and cross-catalog constraint semantics beyond catalog-scoped scalar and composite foreign keys and their local cascade actions.
- Strict multi-file reader atomicity for XBF export.
- Cloud object-storage adapters and retention policy.
- Distributed replication.

## 3. Phase 1: complete the small local DBMS

This phase keeps the database local and makes its operational boundary useful before adding edge or distributed behavior.

### Candidate scope

- Schema introspection.
- A multi-table catalog boundary.
- Secondary-index maintenance and query planning.
- Worker/WASI-specific `AsyncQueryStream` implementations and host-specific asynchronous object-table adapters.
- `PACK` and `RECALL` maintenance operations.
- `verify`, `backup`, and `restore` tooling.
- Read-only WAL inspection.

`PACK` compacts DBT/FPT blocks referenced by surviving records, updates their DBF pointers, refreshes an existing index sidecar, and persists the DBF, memo, index, MVCC, transaction-state, and CDC changes through one WAL-backed recovery boundary.

`RECALL` restores the deletion marker after schema validation and participates in the normal indexed, MVCC, CDC, and transaction-state update path.

### Completion conditions

Schema introspection, verification, sidecar-aware backup and restore, copy tooling, `PACK`, `RECALL`, read-only WAL inspection, and the first directory-catalog boundary are implemented as the first Phase 1 slice.

The catalog currently derives table identity from direct-child DBF filenames and does not persist a separate manifest.

It provides table discovery, named table loading, schema output, per-table verification, and
independent named-table HTTP mutations that reuse the single-table persistence boundary.

It provides catalog-wide commit-level MVCC visibility through a full image of every discovered
table at each successful catalog transaction.

The catalog HTTP server exposes that visibility as one-request historical reads with `at` across
schema, named-table records, query, stream, explain, and join routes.

The selector does not create a long-lived transaction, and historical execution does not reuse
current index sidecars.

The index sidecar foundation is implemented for scalar and per-field-direction compound keys, exact scalar and compound equality, compound equality-prefix and range candidate lookup, equality-prefix compound range candidate lookup, histogram-estimated range ordering, single-field ordered traversal, ordered-prefix traversal for multi-key sorts, compound-prefix traversal for compatible mixed or uniform directions, equality-prefix candidate counting, uniform-statistics ordering for equality candidates, single-index versus intersection cost choice, bounded cost choice based on candidate rows, index traversal, logical 4 KiB index and DBF page reads, and sort work with non-selective-index table-scan fallback, plus deterministic row-equivalent explanation fields for candidate record reads and filter evaluations, path-aware planning, stale detection, explicit rebuild, and WAL-backed DBF/index recovery after normal persistence.

It does not yet support collation-aware index keys, locale-aware CJK collation, or cross-table index definitions.

The remaining items need a public contract, malformed-input behavior, crash behavior, and a fixture or deterministic test.

The current cursor slice supports physical-record pagination and sorted keyset pagination with
`page_size` and `cursor`.
It rejects `skip`, caps pages at 1,000 records, and physical pages scan only until the requested
page and one look-ahead record are found.
Sorted pages materialize matching record references before applying the keyset boundary.
Newly emitted physical and sorted cursors carry a versioned table-representation snapshot tag and
reject reuse after a table change; legacy untagged cursors remain a compatibility boundary.
The public `query::stream_query` iterator covers unsorted filter and projection without
materializing matching records.
`query::stream_query_snapshot` owns a clone of the loaded table to keep records stable while the
caller consumes the pull-based iterator.
`query::stream_query_bounded` runs the same snapshot iterator behind a bounded standard-library
channel, so the producer blocks on a full channel and stops when the consumer is dropped.
The HTTP servers expose `/records/stream` and `/{table}/records/stream` as bounded NDJSON chunked
responses over this stream.
`query::AsyncQueryStream` now defines the executor-neutral `poll_next` contract, and the borrowed and
owned in-memory streams implement it with immediate polls.
The native `query::stream_query_threaded` adapter supplies non-blocking polling, bounded producer
backpressure, waker notification, and worker cancellation on drop.
Worker/WASI timeout, transport, cancellation, and asynchronous-storage behavior remain host-specific,
as does the asynchronous object-table adapter for a worker or WASI host.

An index is not complete for the broader roadmap until insert, update, logical delete, recovery, stale-index detection, rebuild behavior, cost-model limits, direction compatibility, and crash behavior are specified and tested together.

A catalog-wide commit-level snapshot is implemented with commit, rollback, crash recovery, and
visibility rules in the shared catalog journal boundary. Table-local row history is implemented
inside the table MVCC prepare/commit records, with epoch-separated physical row IDs and baseline
reconstruction during full-image GC. The public `DbfTransaction` API provides an optimistic private
snapshot for one DBF table, and the HTTP transaction route reuses that boundary. The public
`CatalogReadTransaction` API captures all discovered tables into one process-local read image and
supports stable table reads and bounded joins after capture. `DbfTransaction::commit_with_row_merge`
explicitly merges disjoint physical-row updates or deletes under the table lock when schema, layout,
and record count are unchanged; inserts and different changes to the same row remain errors.
An identical resulting row is a no-op. Table MVCC also supports optional independent per-row
retention with `mvcc gc --keep-rows`. `DbfTransaction::begin_serializable` provides a
coarse-grained serializable one-table boundary by holding the exclusive table lock for the
transaction lifetime.
`Catalog::begin_serializable` provides a coarse-grained catalog-wide boundary by holding the
catalog write lock and every discovered table lock for the transaction lifetime, applying
operations to private copies, and publishing one catalog journal commit.
Predicate-level locking and distributed serializable coordination remain future work.
The current full-image table and catalog histories have an explicit count-based GC boundary.

The single-table transaction slice covers one DBF table through one snapshot/WAL persistence path and retains committed table snapshots in a `*.txbase.mvcc` sidecar.
The catalog transaction slice prepares named operations across multiple DBFs under a catalog
lock, commits DBF and changed-sidecar images through a directory journal, persists a catalog
journal commit ID, and records a full image of every discovered table in `.txbase.catalog.mvcc`
through that same journal. It recovers an incomplete prepare before the next catalog read and
exposes read-only historical catalog images through the Rust API and CLI. The history is
commit-level, not row-level, and `mvcc gc` retains the newest positive count of full-image
snapshots without changing current table files.

The lower-level `TransactionManager` is separate from those DBF persistence paths. Its
`TransactionId` allocator resumes after reopening a WAL that retains completed transaction
records; this does not provide historical row versions or expose IDs through HTTP.

## 4. Phase 2: expand the query model

The current record scan remains the reference execution path while the query model grows.

### Candidate scope

- Additional aggregation stages and accumulator expressions beyond the current bounded input `$match`, `$unwind` option form, `$set`/`$addFields` expression subset, `$project`, `$sort`, `$skip`, and `$limit` stages and the documented group stages and accumulators.
- Full expression evaluation beyond the bounded `$expr` boolean-tree form and its numeric `$abs`/`$add`/`$subtract`/`$multiply`/`$divide`/`$mod` operands.
- Joins.
- Constraints.
- Full physical cost planning for range and mixed-direction compound paths.

The first join slice is local and bounded.

It also supports a `full` equality join through a bounded hash fallback or a compatible ordered-index merge path, and emits unmatched rows from both sides.
It implements one or more equality conditions, compares bounded hash, index-probe, and merge costs after the small nested-loop boundary, includes exact pre-filter key-cardinality estimates, materialized row-width work, logical input page terms, and logical index-sidecar page terms, uses a fresh single-field index when that estimate wins, uses an exact field-order compound index for direct or chained probes, propagates the cost inputs through chained stages, and uses a compatible ordered-index merge path for large direct joins when that estimate wins.
Additional stages may reference earlier joined tables and keep one catalog read lock across the
pipeline.
Chained `full` stages use the bounded hash fallback rather than the direct ordered merge path.
Eligible chained non-`full` stages can use an ordered merge by sorting the materialized intermediate rows, consuming the loaded rows of the new table through a fresh exact ordered index, and accounting for the bounded sort work.
Filesystem- and cache-aware merge planning, streaming, and broader null or missing field semantics
remain future work before adding broader query surfaces.

Distributed joins and distributed transactions remain later features.

The aggregation boundary defines missing, null, numeric overflow, and memory-limit behavior for the current HTTP API and every additional stage.

The current aggregation slice permits zero or more input `$match` and top-level-array `$unwind` stages, at most one input `$set` or `$addFields` stage in total, and at most one input `$project`, `$sort`, `$skip`, and `$limit` stage each.

The input `$unwind` document form supports `includeArrayIndex` and `preserveNullAndEmptyArrays` for top-level fields while retaining strict rejection of non-array non-null values.

It then permits one terminal `$count` or `$distinct` stage, or one `$group` with `$count`, bounded numeric-expression `$sum`, `$avg`, `$stdDevPop`, and `$stdDevSamp`, `$min`, `$max`, `$first`, `$last`, `$push`, and `$addToSet`, bounded group-output `$match` stages, and a bounded `$project` before the final sort, skip, and limit.

Input stages execute in listed order. `$unwind` preserves input and array order, drops missing, null, or empty-array fields by default, preserves one record for each of them when requested, rejects non-array values, and caps all emitted records at 10,000.

It reuses the existing include/exclude projection contract.

The input `$set` and `$addFields` aliases preserve existing fields and compute top-level fields from field references, scalar literals, `$literal`, two-operand `$ifNull`, and the bounded numeric expression subset.

All expressions in one stage read the stage-input snapshot, and missing or nonnumeric results become `null`; dotted output field names and unwrapped array literals remain unsupported.

The planner explanation boundary is implemented by `explain_query_at`, `explain_query_details_at`, and `QUERY /explain`.
The public explanation exposes deterministic row-equivalent scan and candidate-work costs when a valid index sidecar is available.
The explanation includes the logical 4 KiB index and DBF page estimates used by the local planner.
Direct join execution separately uses deterministic key-cardinality, materialization, and logical DBF-page inputs; those join costs are not part of the single-table explanation object.

The first constraint slice is an optional schema sidecar.
It enforces one-field `primary`, `unique`, and `not_null` properties, bounded composite `primary` and `unique` keys, scalar defaults for omitted inserts, plus bounded table-level query-predicate `checks` on active records and mutation candidates without changing legacy DBF bytes.
Catalog-scoped scalar `references` and composite `constraints.foreign_keys` validation covers non-null child values.
`restrict`, `cascade`, and `set_null` actions are applied recursively inside one catalog transaction and journal commit.
The schema sidecar now has a metadata-only edit command with active-record validation and atomic sidecar replacement.
DBF layout migration, deferred checks, and broader cross-table constraints remain future work.

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
- Additional upstream CJK fixtures.

Every encoding needs a declared name, byte-width rule, round-trip fixture, invalid-byte behavior, and comparison policy.

The first declared-driver slice covers the four Visual FoxPro CJK IDs listed in the DBF
compatibility document and the legacy dBASE aliases listed there.
It uses replacement characters for malformed reads and rejects unmappable or over-width writes.
The sidecar and path-oriented CLI now provide explicit overrides for those four codecs plus strict
Shift_JIS, EUC-JP, GB18030, and ISO-2022-JP; none of the latter four claims a DBF language-driver
mapping.
Schema output exposes declared and effective names plus the interpretation source.
Strict Shift_JIS accepts ASCII, half-width Katakana, and JIS X 0208, while CP932 extensions are
replaced on read or rejected on write. Pinned DBF fixtures cover the four Visual FoxPro CJK IDs
and the legacy dBASE `0x4d` ID, and round-trip their multibyte record values. Alias tests cover
the complete legacy alias set. An upstream Visual FoxPro Windows-1251 fixture also
round-trips Cyrillic record values. Pinned explicit-codec byte fixtures cover DBF record decoding
and write round-trips for all eight supported explicit codec names. The query layer now has a bounded
`unicode-lowercase` sort collation; locale-aware CJK collation and additional broader upstream CJK
fixtures remain future work.

An upstream JavaDBF GBK fixture now covers three GBK-encoded CJK field names and 28 real records,
including a read, mutation, and byte round-trip.

An upstream Rust `dbase-rs` CP936 fixture now covers the legacy dBASE `0x4d` driver ID, including
a read, mutation, and byte round-trip.

An upstream SeoulTech Korea Maps EUC-KR fixture now covers Korean field names, 16 real records, and
the legacy dBASE `0x4e` driver ID, including a read, mutation, and byte round-trip.

The same source also provides a city-level EUC-KR fixture with four Korean field names, 232 real
records, and the legacy dBASE `0x4e` driver ID, including a read, mutation, and byte round-trip.

Classic DBF field descriptor names now use the effective codec as well, so CJK column names remain
usable for JSON access and mutations. The descriptor limit is still measured in bytes, and XBF
export continues to require ASCII field names.

An override must be visible in schema or command output so a reader can reproduce the same interpretation.

## 6. Phase 4: native XBF

XBF is a separately versioned native format, not a silent DBF extension.

The v1 wire contract is drafted in [XBF v1 format draft](xbf.md). A bounded codec, a DBF-to-XBF conversion helper that preserves representable field-level schema constraints and rejects unsupported metadata, bounded in-memory and schema-sidecar XBF-to-DBF export, representability reporting, durable snapshot path, generation-checked full-snapshot WAL recovery, and a journaled `TXSE` schema-preserving file export exist for that draft. `TXSE` records base bytes, recovers interrupted DBF/schema/memo-sidecar/transaction-state/index replacement on the next DBF read, validates the target index before replacement, and rejects external target changes. Strict multi-file reader atomicity and cloud object-storage commit semantics remain future work.

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

The range-oriented local storage abstraction is a useful starting point, but remote object storage needs immutable objects and conditional manifest updates.

### Candidate scope

- A storage-backend redesign for remote object stores.
- Worker or WASI host adapters for the current WASM core.
- An R2 or other cloud object-storage adapter.
- Immutable pages and page-level manifests.
- Cloud generation snapshots and retention.

The local XBF object-store boundary is implemented by `edge::ObjectTable`, `MemoryObjectStore`, and `FilesystemObjectStore`.
It defines the manifest schema and committed-generation history, generation compare-and-swap, retry and recovery behavior, historical reads, explicit local retention, reader generation checks, and orphan cleanup without requiring a cloud account.
The filesystem backend persists the same contract under one directory with exclusive object creation, a store lock, and synced temporary manifest replacement.

The remaining cloud boundary needs a consistency contract, service-specific retention and orphan-page cleanup policy, retry behavior, and a remote adapter fixture.

The current WASM slice exposes DBF bytes, query execution, and record mutation through the shared implementation.
The current slice also exposes the five object-store primitives and the manifest protocol through runtime-neutral future contracts.
The native `ThreadedQueryStream` adapter is available outside `wasm32` and does not change the WASM ABI.
The WASM slice does not yet supply host-specific timeout and cancellation mapping, a remote object-store adapter, or a worker or WASI runtime adapter.

WASM must reuse the DBF or XBF codec and query contracts instead of creating a second database implementation.

The detailed future boundaries are described in [edge storage](edge-storage.md) and [WASM](wasm.md).

## 8. Phase 6: advanced database features

Distributed behavior comes after the local and edge contracts are stable.

### Candidate scope

- Cross-table or distributed long-lived snapshot transactions.
- Persistent WAL history.
- Replication.
- Raft or another explicitly selected authority protocol.
- Snapshot installation.
- Follower reads.
- Distributed partitioning.

These features require an authority model, conflict semantics, schema-version handling, recovery procedures, and observability.

No consensus or multi-region feature is implied by the current exclusive table lock.

The detailed future boundary is described in [distributed evolution](distributed-evolution.md).

The current single-table CDC boundary is described separately in [change data capture](change-data-capture.md).

## 9. XBF and shared upper layers

The DBF and XBF codecs should share query, transaction, HTTP, and storage interfaces where their semantics match.

The format-specific layer should own byte layout, field conversion, checksums, and recovery records.

This prevents XBF from becoming a second unrelated database implementation.

## 10. File granularity rule

A file should be split when its code has different ownership, failure behavior, test fixtures, or change cadence.

The current repository already splits DBF codecs, memo handling, WAL deltas, query validation, server ranges, and test groups along those boundaries.

The query contract, planner rationale, and mutation contract now live in separate documents because they have different ownership and future work.

Further splitting should follow a real boundary.

The number of files is not a quality metric by itself.

## 11. Explicit non-goals for the current slice

- Full dBASE command-language compatibility.
- Full Visual FoxPro runtime compatibility.
- MongoDB wire or BSON compatibility.
- Firebase authentication, security rules, listeners, or offline clients.
- SQLite-level test volume or coverage claims.
- Automatic CJK conversion when the declared encoding is ambiguous.
- Filesystem- and cache-aware merge planning, streaming join execution, aggregation, predicate-level serializable MVCC, durable XBF, cloud object-storage, or distributed code without a contract and end-to-end test.

The current index slice is intentionally local: compatible compound directions, equality-prefix candidate choice, bounded cost choice based on candidate rows, index traversal, logical 4 KiB page reads, and sort work, plus deterministic row-equivalent explanation fields for candidate record reads and filter evaluations, are implemented, while cross-table index definitions and filesystem- or cache-aware merge planning remain future work.
