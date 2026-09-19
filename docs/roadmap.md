# Roadmap and explicit non-goals

This roadmap distills the supplied design note, `txBASE Roadmap: CJK Compatibility, XBF, and Database Evolution`, into reviewable phases.

It is a planning document.

Items marked future are not current features.

## 1. Positioning

txBASE is a file-native transactional database prototype descended from the dBASE and xBase family.

DBF remains the compatibility surface.

The proposed XBF format is a native extension for types and metadata that DBF cannot represent cleanly.

The HTTP and JSON layers are access surfaces, not replacements for the file-format contract.

## 2. Current baseline

The repository currently provides:

- DBF parsing and writing for selected classic and Visual FoxPro fields.
- DBT and FPT memo or binary sidecar paths for the supported formats.
- JSON query execution with a small MongoDB-inspired predicate vocabulary.
- HTTP `GET`, `QUERY`, `POST`, `PUT`, `PATCH`, and `DELETE` routes.
- File or memory WAL types with `TXOP`, `TXDP`, `TXDB`, and `TXDM` persistence paths.
- Startup recovery, stale-snapshot rejection, and an Ubuntu/macOS/Windows CI gate.

The baseline intentionally does not include secondary indexes, joins, aggregation, MVCC, a multi-table catalog, or XBF.

## 3. Phase 1: compatibility and operational hardening

The first phase makes the existing DBF boundary less surprising.

### Candidate scope

- Define language-driver overrides instead of guessing unknown CJK encodings.
- Distinguish Shift_JIS from CP932 and document byte-width behavior.
- Add explicit plans for EUC-JP, GBK, GB18030, Big5, and CP949 or EUC-KR.
- Expand DBF, DBT, FPT, and FoxPro fixtures from independent implementations.
- Add `verify`, `backup`, and `restore` contracts.
- Define `PACK` and `RECALL` behavior without changing logical-delete defaults.
- Define collation and normalization behavior before adding locale-sensitive query operators.

### Completion conditions

Each encoding or command needs a byte-level fixture, a rejection case, a round-trip rule, and a documented compatibility status.

No encoding is accepted only because a lossy fallback happened to produce readable text.

## 4. Phase 2: query engine growth

This phase adds query capability only after the existing scan semantics are stable.

### Candidate scope

- Secondary indexes with defined key encoding and duplicate ordering.
- A catalog for multiple tables, fields, indexes, and schema metadata.
- Joins with bounded intermediate results.
- Aggregation with explicit null, missing-field, and numeric rules.
- Cursors or streaming responses with a stable snapshot contract.
- Query planning and explainable index selection.

### Completion conditions

Each feature needs a reference evaluator, malformed-input tests, persistence and recovery behavior, and a migration story for existing DBF files.

An index is not complete until insert, update, logical delete, recovery, stale-index detection, and rebuild behavior are specified.

## 5. Phase 3: transaction and concurrency growth

The current exclusive table lock is a safety boundary for one process path.

It is not MVCC and it is not distributed coordination.

### Candidate scope

- Multi-record atomic transactions.
- Snapshot isolation or another explicitly named isolation level.
- MVCC metadata in a separate sidecar.
- Lock ownership, timeout, and stale-lock recovery.
- Automatic merge or retry only where operation semantics make it safe.

### Completion conditions

The phase needs a state-transition model, crash matrix, concurrent-writer tests, and a clear answer for how old xBase readers see a checkpointed DBF.

## 6. Phase 4: XBF native format

The proposed XBF format is not a silent DBF extension.

It should be a separately versioned format with a recognizable magic value, such as `TXBF`, and the media type `application/vnd.txbase.xbf` if that registration remains appropriate.

### Candidate scope

- Modern scalar and nested types that DBF cannot represent directly.
- Explicit schema and encoding metadata.
- DBF to XBF and XBF to DBF conversion rules.
- Separate `.xwl` WAL and `.xidx` index sidecars when needed.
- Versioning, checksums, compatibility flags, and a recovery protocol.

### Completion conditions

The format needs a byte-level specification, a reference reader, corruption tests, downgrade behavior, and independent fixture generation before it becomes a default.

DBF compatibility must remain an explicit import or export path.

## 7. Phase 5: object storage and WASM

The design note proposes immutable pages and a manifest compare-and-swap model for object storage such as Cloudflare R2.

It also proposes a WASM boundary for browser or edge execution.

These are deployment architectures, not free extensions of the local file protocol.

### Candidate scope

- Immutable page layout and checksums.
- Manifest CAS and conflict handling.
- Partial reads and bounded memory behavior.
- WASM-compatible codec and query boundaries.
- Browser or edge authentication and capability policy.

### Completion conditions

The object-storage model needs a consistency contract, orphan-page cleanup policy, retry behavior, and a local emulator or deterministic test fixture.

## 8. Phase 6: distributed coordination

Distributed writes should come last.

They require an authority model, conflict semantics, schema versioning, and observability.

Candidate technologies or deployment targets are not commitments.

No Raft, consensus service, or multi-region replication should be introduced before the single-file and object-storage contracts are stable.

## 9. File granularity rule

A file should be split when its code has different ownership, failure behavior, test fixtures, or change cadence.

The current repository already splits DBF codecs, memo handling, WAL deltas, query validation, server ranges, and test groups along those boundaries.

Further splitting should follow a real boundary.

The number of files is not a quality metric by itself.

## 10. Explicit non-goals for the current slice

- Full dBASE command-language compatibility.
- Full Visual FoxPro runtime compatibility.
- MongoDB wire or BSON compatibility.
- Firebase authentication, security rules, listeners, or offline clients.
- SQLite-level test volume or coverage claims.
- Automatic CJK conversion when the declared encoding is ambiguous.
- Speculative XBF, index, join, MVCC, object-storage, or distributed code.

These may become future work only after a concrete contract is approved and its smallest end-to-end test exists.
