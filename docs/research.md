# Specification research index

This directory records the specifications used to define txBASE's current boundary and future work.

The documents distinguish three statuses:

- **Current** means the behavior exists in this repository and is covered by code or tests.
- **Reference** means an external product or protocol is being studied for a design lesson.
- **Future** means a proposal that requires a separate contract before implementation.

This repository does not claim compatibility merely because it uses a familiar name or JSON shape.

## Topic documents

| Topic | Document | Status |
| --- | --- | --- |
| dBASE and Visual FoxPro file structure | [DBF compatibility](dbf-compatibility.md) | Current plus future encoding work |
| Query document, predicates, cursors, and streams | [Query model](query-model.md) | Current subset |
| Runtime-neutral asynchronous query streaming | [Asynchronous query streaming](async-streaming.md) | Current polling boundary and native threaded adapter plus future worker/WASI adapters |
| Bounded aggregation contract | [Aggregation model](aggregation.md) | Current subset |
| Bounded local join contract | [Join model](joins.md) and [Catalog](catalog.md) | Current boundary |
| MongoDB predicates and query planning | [Query planning](query-planning.md) | Current subset plus reference |
| Mutation operators and atomicity | [Mutation model](mutation-model.md) | Current subset plus future boundary |
| Local change data capture | [Change data capture](change-data-capture.md) | Current single-table and explicit multi-table catalog committed-event boundaries plus bounded read-only HTTP transport; delivery and replay remain future work |
| Single-table snapshot transactions and isolation | [Snapshot transactions](transactions.md) and [MVCC](mvcc.md) | Current optimistic boundary plus an opt-in coarse-grained serializable table lock, catalog HTTP historical reads, and future predicate-level or cross-table serializable work |
| Firestore and Realtime Database design | [Firebase model](firebase-model.md) | Reference |
| SQLite test breadth and quality | [Testing and quality](testing-quality.md) | Current test map plus reference |
| Contract-to-test traceability | [Quality contract matrix](quality-matrix.md) | Current evidence map |
| HTTP methods, PATCH, and QUERY | [HTTP semantics](http-semantics.md) | Current routes plus protocol reference |
| Relational schema constraints over DBF | [Schema metadata](schema-metadata.md) | Current local subset plus future relational work |
| Native XBF storage format | [XBF v1 draft](xbf.md) | Draft codec, DBF conversion/export, snapshot path, generation-checked WAL, and journaled schema export |
| Multi-table DBF discovery and bounded local equality join | [Catalog](catalog.md) and [Join model](joins.md) | Current boundary plus future relational work |
| External secondary-index lifecycle | [Indexes](indexes.md) | Current sidecar maintenance, scalar and compound equality, compound equality-prefix candidates, compound equality-prefix range candidates, histogram-estimated range ordering, ordered-prefix traversal, mixed-direction compound-prefix sorting, equality-prefix candidate choice, uniform-statistics ordering for equality candidates, single-index versus intersection cost choice, bounded record, traversal, and sort cost choice, and future full I/O-aware cost model |
| CJK, indexes, XBF, storage, and concurrency | [Roadmap](roadmap.md) | Current boundary plus future work |
| Edge and object-storage commits | [Edge storage](edge-storage.md) | Current local boundary plus future cloud work |
| WASM and worker host boundary | [WASM](wasm.md) | Current host-independent core plus future host adapters |
| Distributed replication and authority | [Distributed evolution](distributed-evolution.md) | Future architecture |

## Research method

Primary specifications and vendor documentation are preferred.

Implementation behavior is checked against the current source and tests before it is called current.

Design notes are labeled future when they are not implemented.

The research pass for this index was refreshed on 2026-09-22.

The README organization follows the section shape of [texenv's README](https://github.com/redpeacock78/texenv/blob/master/README.md), while the content is specific to txBASE.

## Primary-source audit

The following audit separates normative sources from product documentation, implementation references, and project-specific design inspiration.

| Source group | Authority | txBASE alignment | Where the material belongs |
| --- | --- | --- | --- |
| dBASE and Visual FoxPro format pages | The dBASE page is vendor-owned; the Visual FoxPro pages are archived or mirrored vendor help, not a current standards registry. | The parser, memo reader, code-page handling, and field-width rules implement only the documented subset covered by fixtures. Unsupported formats remain errors; the repository does not claim complete FoxPro compatibility. | Format and compatibility details belong in [DBF compatibility](dbf-compatibility.md). Keep the mirror caveat in this index and require a fixture for each compatibility claim. |
| MongoDB manual | Official product documentation. | Query, index, aggregation, and join documents borrow vocabulary and selected behavior, then add explicit txBASE bounds. They do not claim MongoDB wire, planner, or pipeline compatibility. | Keep operator meaning and comparison vocabulary in the query, aggregation, join, and index documents. Do not copy unimplemented MongoDB behavior into the current contract. |
| Rust standard-library task documentation | Official Rust API documentation. | The `AsyncQueryStream` boundary reuses the `Context`, `Poll`, `Waker`, and `Pin` task model without selecting an executor or claiming runtime compatibility. The native `ThreadedQueryStream` adapter applies that contract with a bounded standard-library channel; worker/WASI adapters remain host-specific. | Keep the polling contract and host-adapter responsibilities in [asynchronous query streaming](async-streaming.md); keep txBASE query semantics in [query model](query-model.md). |
| Firestore and Realtime Database documentation | Official product documentation. | The Firebase document is architecture reference only. txBASE does not implement Firebase transactions, offline queues, security rules, or event synchronization. | Keep these comparisons in [Firebase model](firebase-model.md), not in the DBF, HTTP, or transaction contracts. |
| SQLite documentation | Official project documentation. | Constraint terminology, WAL and atomic-commit rationale, query-planning vocabulary, and test-quality practices are references. txBASE uses DBF sidecars and its own WAL and does not claim SQLite file, SQL, or durability compatibility. | Keep the rationale in schema, MVCC, query-planning, and testing documents; describe txBASE behavior separately. |
| PostgreSQL transaction isolation and MVCC documentation | Official project documentation. | The transaction and MVCC documents distinguish the default optimistic stale-source check and the optional coarse-grained serializable table lock from predicate-level and cross-table serializable guarantees that are not implemented. | Keep the distinction in [Snapshot transactions](transactions.md) and [MVCC](mvcc.md); do not copy PostgreSQL isolation guarantees into the current contract. |
| SQLite session extension and PostgreSQL logical decoding | Official project documentation. | The CDC document borrows changeset and committed-WAL-consumer vocabulary, but defines local physical-record state-diff sidecars: `TXCD` for single-table commits and `TXCC` for explicit multi-table catalog commits. It does not provide consumer slots, replay, or replication compatibility. | Keep the external comparison and scope boundary in [Change data capture](change-data-capture.md); keep DBF commit mechanics in [DBF compatibility](dbf-compatibility.md) and catalog-journal mechanics in [Catalog](catalog.md). |
| Git command-line interface documentation | Official project documentation used as a CLI design reference. | It informs txBASE's explicit subcommand and option shape only. txBASE does not copy Git's command set, repository model, or option semantics. | Keep the design decision in [CLI command design](cli.md), not in MVCC or storage contracts. |
| RFC 9110, RFC 5789, and RFC 10008 | IETF standards-track specifications. | HTTP method safety, PATCH meaning, and QUERY safety/idempotency inform the HTTP contract. txBASE still defines its own supported media types, response shapes, range limits, and route bounds. | Normative HTTP semantics belong in [HTTP semantics](http-semantics.md); txBASE-specific restrictions belong beside the implementation contract. |
| POSIX `rename()` and `fsync()` | The Open Group specifications. | Unix code uses rename-based replacement and `sync_all`; Windows has a separate replacement path and must not be described as having identical POSIX directory-durability guarantees. | Keep filesystem durability assumptions in persistence and XBF documents, with the Unix-only qualification. |
| WebAssembly, WASI, and the Component Model | Standards-track or standards-community specifications; host pages are vendor implementation references. | The repository has a versioned host-independent DBF core and a `wasm32-unknown-unknown` compile check. Worker/WASI runtime adapters and asynchronous storage remain future. | Keep the core ABI and host-boundary decisions in [WASM](wasm.md). Add a host-specific source only when that host receives an adapter and a smoke test. |
| Raft consensus | A primary research paper and its project documentation. | Raft is one candidate for a future authority protocol; no replication or distributed execution exists in the current repository. | Keep it in [Distributed evolution](distributed-evolution.md) as a candidate, not as a current implementation dependency. |
| XBF v1 draft | Repository-owned specification. | The current codec and edge object-store tests are checked against this draft; XBF remains explicitly draft until its external-reader atomicity gate is met. | Keep wire details in [XBF v1 draft](xbf.md), and do not treat the draft as an external interoperability standard. |
| `encoding_rs` API and the pinned `dbf` compatibility table | Dependency API documentation and a third-party implementation reference, not encoding standards. | They help identify Rust codec behavior and legacy aliases. Pinned byte fixtures, not the third-party table, are the compatibility evidence. | Keep them as implementation aids in [DBF compatibility](dbf-compatibility.md), never as normative format sources. |
| texenv README | Project-specific style inspiration, not a format or protocol source. | It affects only README organization; no txBASE behavior depends on it. | Keep the attribution in this research index, not in feature contracts. |

The audit found two documentation corrections that are now reflected in the topic documents.

First, `txbase schema apply` is a metadata-only edit command with active-record validation and atomic sidecar replacement; DBF layout migration remains future work.

Second, POSIX durability language is limited to the Unix path; cross-platform replacement behavior is tested separately and is not advertised as identical to POSIX directory durability.

## Primary source groups

### dBASE and Visual FoxPro

- [dBASE Level 7 file format](https://www.dbase.com/Knowledgebase/INT/db7_file_fmt.htm)
- [Visual FoxPro table file structure](https://techshelps.github.io/MSDN/FOXHELP/html/contable_file_structure_lp.dbfrp.htm)
- [Visual FoxPro variable-length fields](https://vfphelp.com/help/html/465e7a94-51b7-4e0c-98f9-432864fe5bcc.htm)
- [Visual FoxPro memo file structure](https://vfphelp.com/help/html/74f53aef-fd56-4f1a-a413-4f045922db21.htm)
- [Visual FoxPro auto-increment fields](https://www.vfphelp.com/vfp9/html/bd6eff0c-2ce5-43b7-ab29-f5360cd2f90e.htm)
- [Visual FoxPro code pages](https://www.vfphelp.com/help/html/a3d7b0e0-8320-44b1-8983-17c30a78c6c4.htm)

### MongoDB

- [Documents](https://www.mongodb.com/docs/manual/core/document/)
- [Query predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/)
- [Find command](https://www.mongodb.com/docs/manual/reference/command/find/)
- [`$expr` field expressions](https://www.mongodb.com/docs/manual/reference/operator/query/expr/)
- [Query optimization](https://www.mongodb.com/docs/manual/core/query-optimization/)
- [Explain and execution statistics](https://www.mongodb.com/docs/manual/reference/method/db.collection.explain/)
- [MongoDB compound indexes](https://www.mongodb.com/docs/manual/core/indexes/index-types/index-compound/)
- [BSON comparison order](https://www.mongodb.com/docs/manual/reference/bson-type-comparison-order/)
- [Compound-index sort order](https://www.mongodb.com/docs/manual/core/indexes/index-types/index-compound/sort-order/)
- [Equality-sort-range guideline](https://www.mongodb.com/docs/manual/tutorial/equality-sort-range-guideline/)
- [Update operators](https://www.mongodb.com/docs/manual/reference/mql/update/)
- [Atomicity and transactions](https://www.mongodb.com/docs/manual/core/write-operations-atomicity/)
- [MongoDB cursors](https://www.mongodb.com/docs/manual/core/cursors/)
- [MongoDB `$group` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/)
- [MongoDB `$count` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/count/)
- [MongoDB `$lookup` join stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/lookup/)

### Firebase

- [Firestore data model](https://firebase.google.com/docs/firestore/data-model)
- [Firestore transactions and batched writes](https://firebase.google.com/docs/firestore/manage-data/transactions)
- [Firestore transaction contention and serializable isolation](https://firebase.google.com/docs/firestore/transaction-data-contention)
- [Firestore write-time aggregation](https://firebase.google.com/docs/firestore/solutions/aggregation)
- [Realtime Database save data](https://firebase.google.com/docs/database/admin/save-data)
- [Realtime Database security](https://firebase.google.com/docs/database/security)
- [Realtime Database offline capabilities](https://firebase.google.com/docs/database/android/offline-capabilities)
- [Firestore query cursors](https://firebase.google.com/docs/firestore/query-data/query-cursors)

### SQLite

- [How SQLite Is Tested](https://sqlite.org/testing.html)
- [Atomic Commit In SQLite](https://sqlite.org/atomiccommit.html)
- [Write-Ahead Logging](https://sqlite.org/wal.html)
- [SQLite database file format](https://sqlite.org/fileformat.html)
- [SQLite query planning](https://sqlite.org/queryplanner.html)
- [SQLite ANALYZE](https://www.sqlite.org/lang_analyze.html)
- [SQLite requirements](https://sqlite.org/requirements.html)
- [SQLite quality management](https://sqlite.org/qmplan.html)
- [SQLite TH3](https://sqlite.org/th3.html)
- [SQLite limits](https://sqlite.org/limits.html)
- [SQLite `CREATE TABLE` constraints](https://sqlite.org/lang_createtable.html)
- [SQLite foreign-key support](https://www.sqlite.org/foreignkeys.html)

### Transactions and concurrency

- [PostgreSQL transaction isolation](https://www.postgresql.org/docs/current/transaction-iso.html)
- [PostgreSQL concurrency control](https://www.postgresql.org/docs/current/mvcc.html)
- [PostgreSQL logical decoding](https://www.postgresql.org/docs/current/logicaldecoding.html)
- [SQLite isolation](https://sqlite.org/isolation.html)
- [SQLite write-ahead logging](https://sqlite.org/wal.html)
- [SQLite session extension](https://www.sqlite.org/sessionintro.html)

### HTTP

- [RFC 9110: HTTP Semantics](https://www.rfc-editor.org/rfc/rfc9110.html)
- [RFC 5789: PATCH Method](https://www.rfc-editor.org/rfc/rfc5789.html)
- [RFC 10008: The HTTP QUERY Method](https://www.rfc-editor.org/rfc/rfc10008.html)

### CLI design

- [Git command-line interface conventions](https://git-scm.com/docs/gitcli)

### Rust task system

- [Rust `Context`](https://doc.rust-lang.org/std/task/struct.Context.html)
- [Rust `Poll`](https://doc.rust-lang.org/std/task/enum.Poll.html)
- [Rust `Pin`](https://doc.rust-lang.org/std/pin/index.html)

### File-system commit primitives

- [POSIX `rename()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/rename.html)
- [POSIX `fsync()`](https://pubs.opengroup.org/onlinepubs/009695399/functions/fsync.html)
- [POSIX file-system cache and directory durability rationale](https://pubs.opengroup.org/onlinepubs/9799919799/xrat/V4_xbd_chap01.html)

### WebAssembly and host boundaries

- [WebAssembly Core Specification](https://webassembly.github.io/spec/core/)
- [WASI](https://wasi.dev/)
- [WebAssembly Component Model](https://component-model.bytecodealliance.org/)
- [Cloudflare Workers WebAssembly](https://developers.cloudflare.com/workers/runtime-apis/webassembly/)
- [Node.js WASI](https://nodejs.org/api/wasi.html)

### Distributed systems

- [In Search of an Understandable Consensus Algorithm (Raft)](https://raft.github.io/raft.pdf)
- [Raft consensus algorithm](https://raft.github.io/)

## Review rule

When a new feature crosses a format, query, transaction, or HTTP boundary, update the relevant topic document and add the smallest fixture or failure test that proves the new contract.

Do not add a broad compatibility claim to the README without an implementation path and a reproducible check.
