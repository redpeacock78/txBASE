# SQLite testing and quality model

SQLite is a useful reference for test breadth and failure discipline.

txBASE is not claiming SQLite's coverage, test volume, or release process.

This document records the practices worth carrying over at the current project scale.

## 1. What SQLite tests

SQLite's official [testing overview](https://sqlite.org/testing.html) describes several independently developed test systems and several classes of failure.

The published list includes:

- Parser and behavior regression tests.
- Boundary-value tests and defined-limit tests.
- Malformed database-file tests.
- Out-of-memory and I/O-error injection.
- Crash and power-loss tests.
- Fuzz testing of SQL and database files.
- Disabled-optimization comparison tests.
- Resource-leak checks, assertions, Valgrind, and undefined-behavior checks.

SQLite also uses differential testing through SQL Logic Test, which compares results across database engines.

The exact harnesses are different, but the categories expose a useful test matrix for any storage engine.

## 2. SQLite's test harnesses

The SQLite page describes four major families:

| Harness | Purpose described by SQLite |
| --- | --- |
| TCL tests | Primary development tests with a large parameterized suite |
| TH3 | Portable C tests using published interfaces, with branch and MC/DC coverage for the covered core configuration |
| SQL Logic Test | Differential comparison of SQL results across engines |
| Fuzzers | Discovery of unexpected behavior from malformed or unusual SQL and database inputs |

The [TH3 documentation](https://sqlite.org/th3.html) says that its coverage subset reaches 100% branch coverage and 100% MC/DC for the SQLite core configuration it covers.

That is a specialized, long-lived quality target.

It should not be copied as a superficial percentage target for txBASE.

Coverage numbers without format fixtures, crash tests, and malformed-input tests would leave the important storage risks unmeasured.

SQLite's [quality-management plan](https://sqlite.org/qmplan.html) treats requirements, test coverage, release checklists, and fault-injection evidence as related quality controls rather than as one percentage.

Its [requirements page](https://sqlite.org/requirements.html) turns testable statements of documented behavior into stable, traceable requirement identifiers.

For txBASE, the practical adaptation is a small contract table linking each format or protocol rule to a fixture, a failure test, and the command that reproduces it.

The current repository-local table is [Quality contract matrix](quality-matrix.md).
It is intentionally a traceability aid, not a coverage score.

## 3. What the current repository tests

The current Rust test layout is already split by ownership:

| Area | Examples |
| --- | --- |
| DBF format | `src/dbf/tests/format.rs`, `format_codepages.rs`, `format_foxpro.rs` |
| Memo sidecars | `src/dbf/tests/memo.rs`, `memo_foxpro.rs`, `src/dbf/malformed_memo_tests.rs` |
| DBF mutation | `src/dbf/tests/mutation.rs`, `mutation_model_tests.rs`, `writer_tests.rs` |
| Persistence | `src/dbf/tests/persistence.rs`, `src/dbf/tests/recovery.rs`, `recovery_fault_tests.rs`, `src/dbf/recovery.rs` |
| Maintenance | `src/dbf/tests/maintenance.rs`, `src/dbf/schema.rs`, `src/dbf/maintenance.rs` |
| Malformed input | `src/dbf/malformed_tests.rs`, `src/dbf/parser_fuzz_tests.rs`, `src/query/malformed_tests.rs`, `src/transaction/malformed_tests.rs`, `src/xbf/malformed_tests.rs`, `tests/corpus/xbf/` |
| Query and HTTP | `src/query/tests.rs`, `src/query/array_predicate_tests.rs`, `src/query/cursor_tests.rs`, `src/query/aggregation_tests.rs`, `src/server/tests.rs`, `src/server/catalog_tests.rs`, `src/server/range.rs` |
| Transactions | `src/transaction/tests.rs`, `src/transaction/malformed_tests.rs`, `src/server/tests.rs`, `src/server/catalog_tests.rs` |

The repository also keeps external-format fixtures under `tests/fixtures/` and malformed corpora under `tests/corpus/`.

## 4. Quality layers for txBASE

New storage behavior should enter the smallest layer that can prove its contract.

### Format and parser tests

Test valid headers, descriptor widths, field flags, byte order, record boundaries, deletion markers, sidecar pointers, and trailing markers.

Test truncated and inconsistent lengths as errors.

### Model tests

Generate mutation sequences against a small in-memory reference model.

Compare active records, deleted records, auto-increment values, memo pointers, and rejected writes after each operation.

This catches interactions that one fixture cannot cover.

### Malformed-input tests

Keep deterministic corpora for DBF, memo, WAL, and JSON inputs.

The property is no panic, bounded parsing, and a useful error or safe rejection.

Malformed input must not cause an out-of-bounds read, a partial successful write, or an accidental WAL truncation beyond an incomplete final record.

### Persistence and recovery tests

Place failures at each boundary:

1. Before WAL append.
2. After WAL append but before sync.
3. After sync but before DBF replacement.
4. After DBF replacement but before sidecar replacement.
5. After all replacements but before cleanup.
6. After index refresh but before WAL cleanup.

Recovery should be idempotent.

An already-applied target must not be applied twice.

A delta with the wrong base must be rejected.

`src/dbf/tests/recovery.rs::recovery_replays_the_dbf_and_index_target_from_one_wal` covers the DBF-replaced/index-not-yet-replaced boundary with a durable `TXDI` target.

`src/query/planner_tests.rs::uses_a_valid_equality_index_and_preserves_scan_results` covers single-index equality, equality intersection, empty intersections, range, ordered traversal, and table-scan equivalence.

`src/query/planner_tests.rs::orders_equality_intersection_by_index_statistics` covers uniform distinct-key estimates and compares the statistics-ordered result with the table scan.

`src/query/planner_tests.rs::chooses_the_lowest_cost_equality_candidate` covers choosing a single equality index when the intersection adds a traversal term, while preserving table-scan results.

`src/index_tests.rs::builds_and_loads_an_external_scalar_index` covers the entry-derived traversal estimate used by planner costs.

`src/query/planner_tests.rs::chooses_the_lowest_cost_range_candidate` covers histogram-ordered candidate construction, exact range-candidate cost choice, and equivalence with the table scan.

`src/query/planner_compound_tests.rs::uses_a_compound_range_after_an_equality_prefix` covers range candidates after an exact compound equality prefix and compares the selected result with the table scan.

`src/query/planner_compound_tests.rs::uses_a_compound_index_for_exact_equality` covers an exact multi-field equality lookup through a compound index and compares the result with the table scan.

`src/query/planner_compound_tests.rs::uses_a_compound_index_for_an_equality_prefix` covers an equality-prefix lookup through a compound index and compares the result with the table scan.

`src/query/planner_compound_tests.rs::uses_a_compound_index_for_a_multi_field_equality_prefix` covers a leading multi-field equality-prefix lookup with a residual predicate, rejects a non-leading prefix, and compares both results with the table scan.

`src/query/planner_tests.rs::uses_an_ordered_index_prefix_for_multi_key_sort` covers first-key index traversal, secondary-key tie sorting, and table-scan equivalence.

`src/query/planner_tests.rs::uses_a_compound_index_for_multi_key_sort` covers ascending compound-key prefix order, complete reverse traversal, mixed-direction fallback, and table-scan equivalence.

`src/query/planner_compound_tests.rs::uses_a_mixed_direction_compound_index` covers per-field directions, complete reverse traversal, unsupported direction combinations, and table-scan equivalence.

`src/query/planner_compound_tests.rs::chooses_a_compound_sort_index_with_the_smallest_equality_prefix` covers equality-prefix candidate counting and compound-plan equivalence with the table scan.

`src/query/planner_compound_tests.rs::chooses_a_table_scan_for_a_non_selective_index` covers the bounded cost tie that rejects an index when it returns every active record.

`src/query/aggregation_tests/accumulator_tests.rs::groups_filtered_records_with_count_and_integer_sum` covers group `$count` and integer `$sum`.

`src/query/aggregation_tests/pipeline_tests.rs::filters_group_output_before_projection_and_sorting` covers a bounded post-group `$match`.

`src/query/aggregation_tests/accumulator_tests.rs::sums_fractional_and_integer_numbers` covers mixed numeric `$sum` input and preserves integer output for all-integral input.

`src/query/aggregation_tests/accumulator_tests.rs::groups_numeric_average_and_returns_null_for_missing_values` covers `$avg`.

`src/query/aggregation_tests/accumulator_tests.rs::groups_comparable_extremes_and_returns_null_for_missing_values` covers `$min` and `$max`.

`src/query/aggregation_tests/input_stage_tests.rs::unwinds_array_values_before_grouping` covers array expansion, empty arrays, missing fields, and explicit `null` values.

`src/query/aggregation_tests/input_stage_tests.rs::preserves_unwind_order_for_array_accumulators` covers input and array order across `$unwind`.

`src/query/aggregation_tests/input_stage_tests.rs::applies_input_stages_in_listed_order` covers input `$limit`, `$sort`, and `$skip` execution order before grouping.

`src/query/aggregation_tests/input_stage_tests.rs::filters_unwound_records_before_grouping` covers filtering expanded records with an input `$match` after `$unwind`.

`src/query/aggregation_tests/input_stage_tests.rs::rejects_non_array_unwind_values` and `bounds_unwound_records` cover strict scalar rejection and the 10,000-record expansion bound.

`src/query/join_strategy.rs::chooses_nested_loop_for_small_join_inputs`, `chooses_hash_for_large_unindexed_inputs`, `chooses_index_nested_loop_when_the_outer_side_is_small`, `chooses_hash_when_index_fanout_is_expensive`, and `chooses_merge_for_large_dual_indexed_inputs` cover the bounded equality-join cost choices.

`src/query/join_merge.rs::scans_equal_key_runs_and_maps_them_to_outer_positions` covers the linear ordered-key merge scan.

`src/query/join_index_tests.rs::large_single_key_join_uses_fresh_ordered_indexes` covers the direct ordered-index path.

`src/query/join_index_tests.rs::large_full_join_preserves_unmatched_rows_with_ordered_indexes` covers the full-join merge path and both unmatched sides.

The compound-key branch in the same test covers the direct compound ordered-index path.

`src/query/join_index_tests.rs::chained_single_key_join_uses_a_fresh_foreign_index`, `chained_right_single_key_join_uses_a_fresh_foreign_index`, `chained_compound_join_uses_a_fresh_foreign_index`, and `chained_right_compound_join_uses_a_fresh_foreign_index` cover indexed paths in chained stages.

`src/query/join_tests.rs` covers output order and chained-stage semantics.

`src/query/field_expression_tests.rs` covers dotted field references, missing or nonnumeric operands, bounded numeric `$abs`, `$add`/`$subtract`/`$multiply`/`$divide`/`$mod`, zero divisors, and malformed or unsupported `$expr` documents.

`src/query/array_predicate_tests.rs` covers bounded `$all`, `$elemMatch`, and exact `$size` matching, same-element condition binding, and malformed array predicate documents.

`src/query/aggregation_tests/accumulator_tests.rs::evaluates_bounded_numeric_accumulator_expressions` covers the shared bounded numeric expressions in `$sum` and `$avg`, including missing and nonnumeric values.

### Compatibility tests

Fixtures should come from independent readers or writers when possible.

The current repository includes dBASE III, dBASE IV, and Visual FoxPro coverage for the implemented field and memo paths.

Each new type or code page needs a byte-level fixture and a JSON round-trip assertion.

### HTTP tests

Test method, path, content type, response status, headers, body, malformed JSON, unsupported operators, range requests, and stale-writer failures.

HTTP tests must not substitute for direct WAL and DBF recovery tests.

## 5. CI contract

The workflow in `.github/workflows/ci.yml` runs on Ubuntu, macOS, and Windows.

Every matrix job runs:

```bash
bash scripts/check-doc-translations.sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

The documentation check also verifies that concrete `src/` and `tests/` paths written in Markdown still exist after a source refactor.

Wildcard examples remain descriptive and are not expanded by this check.

This is the current green gate.

It is intentionally smaller than SQLite's full release process.

Long-running fuzz, soak, differential, and fault-injection jobs should be added only when their input, timeout, artifact, and failure-reproduction contracts are defined.

## 6. Proposed next checks

The order below keeps the feedback loop short:

1. Add fixtures for every newly accepted DBF descriptor or sidecar variant.
2. Add reference-model cases for multi-step mutation sequences.
3. Extend malformed corpora and no-panic checks.
4. Extend deterministic recovery tests for every WAL record kind and replacement boundary.
5. Add bounded fuzz targets for parsers and query validation.
6. Add differential checks only after a reference query evaluator exists.

No coverage percentage is a substitute for these contracts.

The matrix should be updated in the same change as a new format, query, persistence, or HTTP boundary.

## Primary references

- [How SQLite Is Tested](https://sqlite.org/testing.html)
- [SQLite TH3](https://sqlite.org/th3.html)
- [SQLite quality management](https://sqlite.org/qmplan.html)
- [SQLite requirements](https://sqlite.org/requirements.html)
- [SQLite limits](https://sqlite.org/limits.html)
