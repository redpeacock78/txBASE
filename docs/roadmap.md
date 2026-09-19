# Roadmap and explicit non-goals

This roadmap follows the supplied design note, `txBASE Roadmap: CJK Compatibility, XBF, and Database Evolution`.

It preserves the proposed phase order and marks implementation status explicitly.

Items marked future are not current features.

## 1. Positioning

txBASE is a file-native transactional database descended from the dBASE and xBase family.

DBF remains the compatibility and preservation surface.

The proposed XBF format is a native extension for types and metadata that DBF cannot represent cleanly.

HTTP, JSON, MCP, and WASM are access or execution surfaces above the storage formats.

The long-term design point is portability for existing xBase data, not replacement of SQLite or PostgreSQL.

## 2. Current baseline

The repository currently provides:

- DBF parsing and writing for selected classic and Visual FoxPro fields.
- DBT and FPT memo or binary sidecar paths for the supported formats.
- JSON query execution with a small MongoDB-inspired predicate vocabulary.
- HTTP `GET`, `QUERY`, `POST`, `PUT`, `PATCH`, and `DELETE` routes.
- File or memory WAL types with `TXOP`, `TXDP`, `TXDB`, and `TXDM` persistence paths.
- Startup recovery, stale-snapshot rejection, and an Ubuntu/macOS/Windows CI gate.
- Schema introspection, DBF verification, and validated DBF plus memo-sidecar copy commands.
- A directory catalog that discovers direct-child DBF tables, loads named tables, and verifies all discovered tables.
- A rebuildable external scalar and compound-key index sidecar with equality and range candidate lookup, single-field and ordered-prefix traversal, compound-prefix sort traversal, equality candidate intersection, path-aware planning, and DBF/memo freshness checks.

The baseline intentionally does not include collection-statistics-based index choice, mixed-direction compound indexes, cursors, multi-record transactions, aggregation, joins, constraints, XBF, object-storage commits, or distributed replication.

## 3. Phase 1: complete the small local DBMS

This phase keeps the database local and makes its operational boundary useful before adding edge or distributed behavior.

### Candidate scope

- Schema introspection.
- A multi-table catalog boundary.
- Secondary-index maintenance and query planning.
- Cursor or streaming query execution.
- Multi-record transactions.
- `PACK` and `RECALL` maintenance operations.
- `verify`, `backup`, and `restore` tooling.

`PACK` must define whether memo blocks and indexes are rebuilt or left as reclaimable orphan space.

`RECALL` must define whether it restores only the deletion marker or also participates in an indexed or transactional update.

### Completion conditions

Schema introspection, verification, copy tooling, `PACK`, `RECALL`, and the first directory-catalog boundary are implemented as the first Phase 1 slice.

The catalog currently derives table identity from direct-child DBF filenames and does not persist a separate manifest.

It provides table discovery, named table loading, schema output, and per-table verification.

It does not yet provide shared locks, relationships, cross-table index coordination, or cross-table transactions.

The index sidecar foundation is implemented for scalar and ascending compound keys, exact equality and range candidate lookup, single-field ordered traversal, ordered-prefix traversal for multi-key sorts, compound-prefix traversal for all-ascending or all-descending sorts, equality candidate intersection ordered by exact candidate cardinality, path-aware planning, stale detection, explicit rebuild, and WAL-backed DBF/index recovery after normal persistence.

It does not yet support collection-statistics-based selectivity choice, mixed-direction compound definitions, or cross-table atomic commits.

The remaining items need a public contract, malformed-input behavior, crash behavior, and a fixture or deterministic test.

An index is not complete for the broader roadmap until insert, update, logical delete, recovery, stale-index detection, rebuild behavior, collection-statistics-based choice, mixed-direction definitions, and crash behavior are specified and tested together.

A multi-record transaction is not complete until commit, rollback, crash recovery, and visibility rules are tested together.

## 4. Phase 2: expand the query model

The current record scan remains the reference execution path while the query model grows.

### Candidate scope

- Aggregation.
- Field-to-field comparisons such as `$field` expressions.
- Joins.
- Constraints.
- Range, collection-statistics-based, and mixed-direction compound query planning.

Joins should begin as local bounded operations.

Distributed joins and distributed transactions remain later features.

Aggregation must define missing, null, numeric overflow, and memory-limit behavior before it is added to the HTTP API.

The planner must explain when it uses an index and when it scans.

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

An override must be visible in schema or command output so a reader can reproduce the same interpretation.

## 6. Phase 4: native XBF

XBF is a separately versioned native format, not a silent DBF extension.

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
- Collection-statistics-backed planners, joins, aggregation, MVCC, XBF, object-storage, or distributed code without a contract and end-to-end test.

The next index slice is intentionally local: collection-statistics-based selectivity choice, after its contract is written.
