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

## 3. What the current repository tests

The current Rust test layout is already split by ownership:

| Area | Examples |
| --- | --- |
| DBF format | `src/dbf/tests/format.rs`, `format_codepages.rs`, `format_foxpro.rs` |
| Memo sidecars | `src/dbf/tests/memo.rs`, `memo_foxpro.rs`, `src/dbf/malformed_memo_tests.rs` |
| DBF mutation | `src/dbf/tests/mutation.rs`, `mutation_model_tests.rs`, `writer_tests.rs` |
| Persistence | `src/dbf/tests/persistence.rs`, `recovery_fault_tests.rs`, `src/dbf/recovery.rs` |
| Malformed input | `src/dbf/malformed_tests.rs`, `src/dbf/parser_fuzz_tests.rs`, `src/query/malformed_tests.rs`, `src/transaction/malformed_tests.rs` |
| Query and HTTP | `src/query/tests.rs`, `src/server/tests.rs`, `src/server/range.rs` |
| Transactions | `src/transaction/tests.rs`, `src/transaction/malformed_tests.rs` |

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

Recovery should be idempotent.

An already-applied target must not be applied twice.

A delta with the wrong base must be rejected.

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
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

This is the current green gate.

It is intentionally smaller than SQLite's full release process.

Long-running fuzz, soak, differential, and fault-injection jobs should be added only when their input, timeout, artifact, and failure-reproduction contracts are defined.

## 6. Proposed next checks

The order below keeps the feedback loop short:

1. Add fixtures for every newly accepted DBF descriptor or sidecar variant.
2. Add reference-model cases for multi-step mutation sequences.
3. Extend malformed corpora and no-panic checks.
4. Add deterministic recovery tests for every WAL record kind.
5. Add bounded fuzz targets for parsers and query validation.
6. Add differential checks only after a reference query evaluator exists.

No coverage percentage is a substitute for these contracts.

## Primary references

- [How SQLite Is Tested](https://sqlite.org/testing.html)
- [SQLite TH3](https://sqlite.org/th3.html)
- [SQLite limits](https://sqlite.org/limits.html)
