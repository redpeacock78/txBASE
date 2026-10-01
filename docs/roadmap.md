# Roadmap and explicit non-goals

Check the [quality contract matrix](quality-matrix.md) for current behavior and CI evidence, then use this page to choose an unfinished phase.

This roadmap records implementation status, sequencing, and open gates.
Detailed behavior belongs in the linked topic documents.
An item marked future is not an implemented feature.

## 1. Positioning

txBASE is a file-native transactional database descended from the dBASE and xBase family.

DBF remains the compatibility and preservation format.
XBF is a separate native-format draft for values and metadata that DBF cannot represent cleanly.
HTTP, JSON, MCP, and WASM are access or execution surfaces above those formats.

The design target is portability for existing xBase data, not replacement of SQLite or PostgreSQL.

## 2. Current status

The quality matrix is the detailed source for contract labels, implementation paths, and deterministic checks.
This summary shows the current boundary and the next open gate for each roadmap phase.

| Phase | Current boundary | Open gate |
| --- | --- | --- |
| Local database | The declared Phase 1 scope is implemented. | Continue correctness work through the quality matrix and topic contracts. |
| Query model | Bounded query and aggregation contracts are implemented, including shared `$map`, `$filter`, `$reduce`, and `$let` expressions. | Further expression/operator families, filesystem-aware planning, and cost models that account for cache and page reuse. |
| Legacy international compatibility | Declared and explicit codecs, upstream fixtures, and versioned locale collations are implemented. | A full locale-specific expected-order corpus. |
| Native XBF | The bounded codec, conversion, snapshot WAL, and schema-preserving export are implemented against a draft. | Strict multi-file atomicity for readers that ignore txBASE locks. |
| Edge storage | Local object stores and deterministic Worker, R2, and WASI fixtures are implemented. | Live host/provider validation and production lifecycle policy. |
| Distributed database features | Fixed-term replication and optional OpenRaft mode are implemented. | Broader failure schedules and distributed contracts listed in Phase 6. |

## 3. Phase 1: complete the small local DBMS

The declared Phase 1 candidate scope is implemented.
It established local storage and recovery boundaries before edge or distributed behavior.

- The directory catalog discovers direct-child DBF files and does not persist a separate manifest.
- Indexes are table-local; the catalog does not define a cross-table index registry or persistent join-result index.
- Single-table and catalog transactions, MVCC snapshots, sidecar-aware maintenance, verification, backup, restore, and read-only WAL inspection have documented contracts.

See [DBF compatibility](dbf-compatibility.md), [catalog](catalog.md), [indexes](indexes.md), [transactions](transactions.md), [MVCC](mvcc.md), [CLI command architecture](cli-design.md), and the [CLI reference](cli.md).

## 4. Phase 2: expand the query model

The current bounded query paths remain the reference behavior.
The query model, aggregation stages, join planner, and stream adapters are documented separately.

### Implemented boundary

The repository supports bounded JSON predicates and shared boolean/scalar expressions, including lexical variables and `$map`, `$filter`, and `$reduce`; it also supports `$exists`, ordered aggregation pipelines, local equality joins, explain output, table-local indexes, and pull-based or backpressured query streams.
Those contracts do not imply MongoDB compatibility.

### Remaining work

1. Add aggregation stages and accumulator expressions beyond the current bounded contracts.
2. Extend expression evaluation beyond the current bounded boolean/scalar subset, including `$let`, `$map`, `$filter`, and `$reduce`.
3. Add filesystem-, cache-, and page-reuse-aware merge costing.
4. Define null and missing behavior for each additional operator family before implementation.

See [query model](query-model.md), [expression model](expressions.md), [aggregation](aggregation.md), [aggregation accumulators](aggregation-accumulators.md), [joins](joins.md), [query planning](query-planning.md), and [asynchronous query streaming](async-streaming.md).

## 5. Phase 3: legacy international compatibility

The declared-codec path, explicit codec overrides, CJK field-name decoding, and pinned upstream fixtures are implemented.
Versioned ICU4X Japanese, Chinese, and Korean collations are also available.

### Remaining work

Resolve the 15,169 Chinese Pinyin-long ordering mismatches between CLDR 48 and ICU4X 2.1.1, then expand locale-specific expected-order coverage beyond starred single-character rules.
The Chinese source chain contains 44,469 adjacent relations, and ICU4X 2.1.1 fails the required strict order for 15,169 of them.
A CI baseline test pins this count; a separate test asserts the reverse ordering for U+319B1/阿 and 𥥩/锕.
The fixtures preserve Japanese and Korean reset boundaries and the complete Chinese source chain.
They do not cover every collation rule form or arbitrary strings, so they are not a full locale-conformance suite.
Benchmark inputs continue to check comparator laws; they do not establish locale conformance.

The upstream review found no stable ICU4X data release based on CLDR 49; the published ICU4X 2.3.0 collation data uses CLDR 48.2.1, while CLDR 49 remains in beta.
This work remains open; existing identifiers continue to represent ICU4X 2.1.1 until a stable candidate orders the complete Chinese corpus with zero mismatches in CI.
See the [CJK collation source audit](research.md) for release details.

See [DBF compatibility](dbf-compatibility.md), [query model](query-model.md), and the [research index](research.md) for the source and fixture boundaries.

## 6. Phase 4: native XBF

XBF is a separately versioned native format, not a silent DBF extension.
The repository has a bounded codec, DBF conversion helpers, generation-checked full-snapshot WAL recovery, and journaled schema-preserving export.

XBF remains a draft until its external-reader atomicity gate is resolved.
The current path lock protects cooperating txBASE readers; readers that ignore the lock do not receive a physically atomic view of multiple files.
Cloud object-store publication belongs to the separate edge-storage contract.

### Remaining work

Specify and test the required multi-file behavior for legacy readers, or keep that behavior explicitly outside the supported contract.
Do not infer an external interoperability standard or registered media type from the draft.

See [XBF v1 format draft](xbf.md), [edge storage](edge-storage.md), and the [quality contract matrix](quality-matrix.md).

## 7. Phase 5: edge storage

The local XBF object-store protocol supports immutable page objects, versioned manifests, conditional publication, recovery, historical reads, retention, and orphan cleanup.
The synchronous and runtime-neutral asynchronous APIs use the same protocol.

Worker Fetch and R2 adapters are checked against deterministic fixtures, not deployed services.
The WASI 0.3 query-stream component reads DBF or current/retained XBF data through a writable preopened filesystem and requires a single writer.

### Remaining work

- Validate a deployed Worker and live R2 service.
- Add provider integrations beyond R2 and provider-backed WASI storage.
- Define production host lifecycle, retry, and scheduled-retention policies.

See [edge storage](edge-storage.md), [WASM](wasm.md), [WASI query streaming](wasi-query-stream.md), [Worker query streaming](worker-query-stream.md), [Worker Fetch storage](worker-object-store.md), and [R2 storage](r2-object-store.md).

## 8. Phase 6: advanced database features

The fixed-term replication path is a process-local single-authority protocol.
An optional OpenRaft mode provides quorum writes, linearizable reads, peer authentication, learner joining, and joint-consensus membership changes.
The applied-index token provides session-monotonic reads, not a guarantee of the latest quorum-committed state.

CI covers the schedules and crash points listed in [Raft consensus design](raft.md) and [distributed evolution](distributed-evolution.md).
Those checks do not establish general consensus correctness for every network schedule.

### Remaining work

- Cross-table or distributed long-lived snapshot transactions and durable history beyond the existing table, catalog, `TXRP`, and `TXRG` sidecars.
- Durable retry queues, backpressure, and authority discovery.
- Follower reads with freshness stronger than the applied-index token.
- Distributed partitioning and the network, schema-version, recovery, and observability contracts it requires.
- Additional fault schedules beyond the checked request-order cases and catch-up batches through 257 entries, including other terms and partition conditions.

## 9. XBF and shared upper layers

DBF and XBF should share query, transaction, HTTP, and storage interfaces wherever their semantics match.
The format-specific layer owns byte layout, field conversion, checksums, and recovery records.

This keeps XBF from becoming a second unrelated database implementation.

## 10. File granularity rule

Split a document when ownership, failure behavior, fixtures, or change cadence differ.
Keep related contracts together when a split would only add navigation overhead.

The repository separates query contracts, planner rationale, mutation behavior, CLI architecture, host adapters, and distributed protocols into documents with distinct responsibilities.
Further splits should follow a concrete boundary, not a line-count target.

## 11. Explicit non-goals for the current slice

- Full dBASE command-language or Visual FoxPro runtime compatibility.
- MongoDB wire, BSON, planner, or pipeline compatibility.
- Firebase authentication, security rules, listeners, or offline clients.
- SQLite-level coverage or quality claims.
- Automatic encoding selection when the DBF declaration is ambiguous.

Future work listed above remains future even when an adjacent adapter or deterministic fixture exists.
Primary-source classification and topic ownership are recorded in the [research index](research.md).
