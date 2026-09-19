# Quality contract matrix

This matrix turns the SQLite-inspired requirements lesson into a small, repository-local traceability map.

It links a behavior claim to implementation evidence and a deterministic check.
It is not a coverage percentage, a claim of SQLite-level quality, or a list of every test.

## How to read the matrix

Each identifier is local to txBASE.

`Current` means the contract is implemented and represented by the cited check.

`Boundary` means the behavior is intentionally limited and the limit is part of the contract.

`Future` means the topic is documented but must not be described as an implemented feature.

The authoritative CI command is:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

The workflow runs that gate on Ubuntu, macOS, and Windows.

## Current contracts

| ID | Contract | Implementation or fixture evidence | Deterministic check | Status |
| --- | --- | --- | --- | --- |
| DBF-001 | Declared headers, descriptors, record lengths, and deletion markers are bounds-checked. | `src/dbf/parser.rs`; `tests/corpus/dbf/` | `src/dbf/malformed_tests.rs::rejects_malformed_dbf_corpus` and format tests | Current |
| DBF-002 | Supported dBASE III, dBASE IV, and Visual FoxPro fixtures round-trip through the declared field boundary. | `tests/fixtures/external-*.dbf.hex`; `src/dbf/compatibility_tests.rs` | `reads_and_writes_a_pinned_external_*_fixture` | Current |
| DBF-003 | Declared CJK drivers plus explicit EUC-JP and GB18030 overrides use byte-width validation, replacement reads, rejecting writes, and visible declared/effective metadata. | `src/dbf/codec_cjk.rs`; `src/dbf/schema.rs`; `src/dbf/cjk_tests.rs`; `src/dbf/schema_metadata_tests.rs` | `decodes_and_encodes_declared_cjk_drivers`; `explicit_euc_jp_and_gb18030_overrides_round_trip`; `applies_a_supported_encoding_override_to_reads_and_writes`; `applies_a_path_encoding_override_without_persisting_it` | Current |
| MEM-001 | DBT and FPT pointers, block sizes, terminators, and malformed sidecars are handled within the selected memo format. | `src/dbf/memo_file.rs`; `src/dbf/memo_value.rs`; `tests/corpus/memo/` | `src/dbf/tests/memo.rs`, `memo_foxpro.rs`, and `malformed_memo_tests.rs` | Current |
| MUT-001 | Record mutations preserve the in-memory model, reject unknown or ambiguous writes, and keep opaque fields intact. | `src/dbf/mutation.rs`; `src/dbf/mutation_model_tests.rs` | `generated_mutation_sequence_matches_reference_model` and mutation tests | Current |
| WAL-001 | Complete WAL records are validated; torn final records are truncated or discarded without a panic. | `src/transaction.rs`; `src/dbf/wal.rs`; `tests/corpus/wal/` | `transaction::malformed_tests::rejects_malformed_wal_corpus`; `file_wal_reopens_and_truncates_a_torn_tail` | Current |
| WAL-002 | Snapshot, delta, mutation-intent, memo, and index recovery are idempotent across replacement boundaries. | `src/dbf/recovery.rs`; `src/dbf/persistence.rs` | `src/dbf/tests/persistence.rs` recovery cases and `recovery_fault_tests.rs` | Current |
| WAL-003 | Independently loaded stale writers are rejected rather than silently overwriting newer DBF or memo bytes. | `src/dbf/persistence.rs`; `src/dbf/writer_tests.rs` | `rejects_a_stale_second_writer_without_overwriting_the_first_save` | Current |
| XBF-001 | The draft XBF codec bounds allocations, validates header and section CRC-32C values, decodes all non-reserved v1 scalar types, preserves physical deletion flags, and enforces local uniqueness. | `src/xbf/`; `docs/xbf.md` | `src/xbf/tests.rs` fixture, corruption, size-limit, and constraint cases | Boundary |
| XBF-002 | The XBF snapshot path encodes before writing, syncs the snapshot bytes, replaces the target, and syncs the parent directory before returning. | `src/xbf/persistence.rs` | `writes_and_reads_a_durable_snapshot_path` | Boundary |
| XBF-003 | The full-snapshot XBF WAL names its base and target generations, enforces the transaction-layer record limit, applies a pending snapshot idempotently, and rejects a different current generation. | `src/xbf/wal.rs` | `recovers_a_generation_checked_full_snapshot_wal`; `rejects_a_generation_mismatched_xbf_wal`; `rejects_xbf_wal_records_over_the_file_wal_limit` | Boundary |
| XBF-004 | Loaded DBF records convert to bounded XBF values without dropping physical order, deletion flags, or DBF-distinguishable nulls; unsupported values fail explicitly. | `src/xbf/conversion.rs`; `src/xbf/tests.rs` | `converts_a_dbf_fixture_to_xbf_without_dropping_records` | Boundary |
| XBF-005 | A representable XBF table exports to an in-memory DBF table with deletion state preserved; unsupported types and unrepresentable constraints or values fail explicitly. | `src/xbf/export.rs`; `src/xbf/tests.rs` | `exports_a_representable_xbf_table_to_dbf`; `rejects_nonrepresentable_xbf_dbf_export_types` | Boundary |
| IDX-001 | External scalar and compound indexes are rebuildable, freshness-checked, and refreshed after supported mutations. | `src/index.rs`; `src/index/`; `src/index_tests.rs` | `mutation_refreshes_the_sidecar_after_save`; `direct_dbf_change_leaves_the_sidecar_stale_until_rebuild` | Current |
| IDX-002 | Index candidates preserve table-scan results for equality, range, ordered-prefix, compound, and equality-intersection plans. | `src/query/planner.rs`; `src/query/planner_tests.rs`; `src/query/planner_compound_tests.rs` | Planner tests named `uses_*` and `orders_*` | Current |
| QRY-001 | Query parsing validates the documented filter, path, projection, sort, and update boundaries without claiming MongoDB compatibility. | `src/query/validation.rs`; `src/query/tests.rs` | `parses_query_shape`; `rejects_unknown_query_fields`; `rejects_unknown_and_mixed_projection_operators` | Current |
| QRY-002 | Bounded `$expr` comparisons and malformed JSON query corpora fail safely and do not become index lookups. | `src/query/field_expression_tests.rs`; `src/query/malformed_tests.rs` | `compares_two_fields_with_expr`; `rejects_malformed_json_query_corpus` | Boundary |
| QRY-003 | Physical cursors stop after the requested page and look-ahead record; sorted cursors enforce a matching keyset definition; bounded aggregation stages and local inner/left/semi/anti/cross joins enforce explicit bounds. | `src/query/pagination.rs`; `src/query/aggregation.rs`; `src/query/join.rs` | `cursor_tests.rs`, `aggregation_tests.rs`, and `join_tests.rs` | Boundary |
| QRY-004 | The borrowed query stream applies filter, projection, skip, and limit incrementally and rejects blocking or resumable controls. | `src/query/stream.rs`; `src/query/stream_tests.rs` | `streams_filtered_projected_records_with_bounded_controls`; `streaming_rejects_blocking_and_resume_controls` | Boundary |
| CAT-001 | A catalog discovers only direct-child DBF files and reports table-specific load or verification failures. | `src/catalog.rs`; `docs/catalog.md` | `cargo test --all-targets --all-features` catalog and join cases | Current |
| SCH-001 | Optional schema metadata validates active records and mutation candidates without changing legacy DBF bytes. | `src/dbf/schema_metadata.rs`; `src/dbf/schema_metadata_tests.rs` | `loads_schema_metadata_and_enforces_local_constraints`; stale-sidecar and copy tests | Boundary |
| HTTP-001 | HTTP method, JSON media-type, QUERY status, response-header, and update-operator boundaries are explicit. | `src/server.rs`; `src/server/transaction.rs`; `docs/http-semantics.md` | `src/server/tests.rs::query_endpoint_enforces_json_boundary_and_executes` and mutation tests | Current |
| HTTP-002 | Supported single byte ranges and persistence failures return bounded responses without replacing the wrong table state. | `src/server/range.rs`; `src/server/tests.rs` | `query_endpoint_handles_single_byte_ranges`; `reloads_disk_state_after_persistence_failure` | Boundary |
| CI-001 | Formatting, lint, and all-target tests are required on all supported CI operating systems. | `.github/workflows/ci.yml` | GitHub Actions matrix run | Current |

## Gaps that remain explicit

The following topics have documentation or design notes but do not have a current implementation claim in the matrix:

- backpressure and stable snapshot rules for long-lived streams;
- a full cost-based planner, multiple or planned joins, and aggregation stages beyond bounded `$match` plus `$group`;
- cross-table transactions, transaction IDs, and MVCC visibility;
- strict Shift_JIS versus CP932 selection, collation, and broader external fixtures;
- Schema-preserving XBF-to-DBF representability reporting, object-storage manifests, WASM hosting, and distributed replication.

Before one of these becomes current, add its public contract, malformed-input behavior, crash or retry behavior, fixture or deterministic test, and a row here.

## Source basis

The quality model follows the categories described by SQLite's [testing overview](https://sqlite.org/testing.html), [quality-management plan](https://sqlite.org/qmplan.html), and [requirements catalog](https://sqlite.org/requirements.html).

Those sources motivate traceable requirements, independent malformed and fault tests, and reproducible release gates.

They do not imply that txBASE has SQLite's test volume, coverage, or release process.
