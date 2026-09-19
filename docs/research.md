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
| MongoDB predicates and query planning | [Query model](query-model.md) | Current subset plus reference |
| Firestore and Realtime Database design | [Firebase model](firebase-model.md) | Reference |
| SQLite test breadth and quality | [Testing and quality](testing-quality.md) | Current test map plus reference |
| HTTP methods, PATCH, and QUERY | [HTTP semantics](http-semantics.md) | Current routes plus protocol reference |
| Multi-table DBF discovery and bounded local equality join | [Catalog](catalog.md) and [Query model](query-model.md) | Current boundary plus future relational work |
| External secondary-index lifecycle | [Indexes](indexes.md) | Current sidecar maintenance, equality, histogram-estimated range ordering, ordered-prefix traversal, mixed-direction compound-prefix sorting, equality-prefix candidate choice, uniform-statistics-ordered equality intersection, and future full cost model |
| CJK, indexes, XBF, storage, and concurrency | [Roadmap](roadmap.md) | Future |

## Research method

Primary specifications and vendor documentation are preferred.

Implementation behavior is checked against the current source and tests before it is called current.

Design notes are labeled future when they are not implemented.

The research pass for this index was refreshed on 2026-09-19.

The README organization follows the section shape of [texenv's README](https://github.com/redpeacock78/texenv/blob/master/README.md), while the content is specific to txBASE.

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

### HTTP

- [RFC 9110: HTTP Semantics](https://www.rfc-editor.org/rfc/rfc9110.html)
- [RFC 5789: PATCH Method](https://www.rfc-editor.org/rfc/rfc5789.html)
- [RFC 10008: The HTTP QUERY Method](https://www.rfc-editor.org/rfc/rfc10008.html)

### File-system commit primitives

- [POSIX `rename()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/rename.html)
- [POSIX `fsync()`](https://pubs.opengroup.org/onlinepubs/009695399/functions/fsync.html)
- [POSIX file-system cache and directory durability rationale](https://pubs.opengroup.org/onlinepubs/9799919799/xrat/V4_xbd_chap01.html)

## Review rule

When a new feature crosses a format, query, transaction, or HTTP boundary, update the relevant topic document and add the smallest fixture or failure test that proves the new contract.

Do not add a broad compatibility claim to the README without an implementation path and a reproducible check.
