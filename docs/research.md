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
| Multi-table DBF discovery and schema lookup | [Catalog](catalog.md) | Current boundary plus future relational work |
| External secondary-index lifecycle | [Indexes](indexes.md) | Current sidecar maintenance, equality, range, and single-field ordered planner plus future multi-key planner |
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
- [Query optimization](https://www.mongodb.com/docs/manual/core/query-optimization/)
- [Update operators](https://www.mongodb.com/docs/manual/reference/mql/update/)
- [Atomicity and transactions](https://www.mongodb.com/docs/manual/core/write-operations-atomicity/)

### Firebase

- [Firestore data model](https://firebase.google.com/docs/firestore/data-model)
- [Firestore transactions and batched writes](https://firebase.google.com/docs/firestore/manage-data/transactions)
- [Firestore write-time aggregation](https://firebase.google.com/docs/firestore/solutions/aggregation)
- [Realtime Database save data](https://firebase.google.com/docs/database/admin/save-data)
- [Realtime Database security](https://firebase.google.com/docs/database/security)
- [Realtime Database offline capabilities](https://firebase.google.com/docs/database/android/offline-capabilities)

### SQLite

- [How SQLite Is Tested](https://sqlite.org/testing.html)
- [SQLite query planning](https://sqlite.org/queryplanner.html)
- [SQLite requirements](https://sqlite.org/requirements.html)
- [SQLite TH3](https://sqlite.org/th3.html)
- [SQLite limits](https://sqlite.org/limits.html)

### HTTP

- [RFC 9110: HTTP Semantics](https://www.rfc-editor.org/rfc/rfc9110.html)
- [RFC 5789: PATCH Method](https://www.rfc-editor.org/rfc/rfc5789.html)
- [RFC 10008: The HTTP QUERY Method](https://www.rfc-editor.org/rfc/rfc10008.html)

## Review rule

When a new feature crosses a format, query, transaction, or HTTP boundary, update the relevant topic document and add the smallest fixture or failure test that proves the new contract.

Do not add a broad compatibility claim to the README without an implementation path and a reproducible check.
