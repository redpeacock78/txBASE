# Secondary-index sidecar

The current index slice stores a rebuildable secondary index outside the DBF file.

The sidecar path for `users.dbf` is `users.txidx`.

The DBF remains readable by legacy xBase tools because the index is not embedded in the DBF bytes.

## Version 3 contract

An index file is JSON with this top-level shape:

```json
{
  "format": "txbase-index",
  "version": 3,
  "source": {
    "dbf": {"length": 0, "hash": 0},
    "memo": null
  },
  "statistics": {
    "active_record_count": 0,
    "indexes": [{"name": "NAME"}]
  },
  "indexes": [
    {
      "definition": {"name": "NAME", "field": "NAME"},
      "entries": [
        {
          "key": {"kind": "scalar", "value": "Alice"},
          "records": [1]
        }
      ]
    }
  ]
}
```

A compound definition uses an ordered `fields` array instead of `field`:

```json
{"name": "by_name_age", "fields": ["NAME", "AGE"]}
```

A version 3 compound definition may add a `directions` array with one `1` or `-1` value per field:

```json
{"name": "by_score_name", "fields": ["SCORE", "NAME"], "directions": [-1, 1]}
```

When `directions` is absent, every field is ascending for compatibility with older sidecars.

Single-field indexes remain ascending because range lookup and histogram boundaries use ascending key order.

The real `length` and `hash` values are generated from the DBF and detected memo sidecar.

The hash is an FNV-1a freshness check, not an authenticity or tamper-proof signature.

Single-field keys accept strings, numbers, booleans, and explicit `null`.

A compound key is an ordered tuple of those scalar domains, with one component for each declared field.

Arrays and objects are rejected because their equality, ordering, and multikey behavior are not yet part of the contract.

Missing fields and explicit `null` have different index keys.

Records use one-based physical DBF record numbers.

Deleted records are not indexed.

Entries are grouped by the canonical JSON encoding of the typed key, then ordered lexicographically by the declared field order.

Each component uses the order `Missing`, `null`, boolean, number, and string; numeric values use exact integer or floating-point comparison where representable.

Record numbers within one key remain strictly increasing so equal sort keys preserve DBF physical order.

A current sidecar stores the active-record count and per-index statistics.

Each single-field index stores an equi-depth histogram split by typed value domain, with a lower key, upper key, distinct-key count, and indexed record count for each bucket.

Compound indexes do not store histograms yet.

## Lifecycle

Build one single-field index per argument:

```bash
txbase index build path/to/users.dbf NAME AGE
```

Build one ascending compound index in the declared field order:

```bash
txbase index build-compound path/to/users.dbf by_name_age NAME AGE
```

Add per-field directions with `:1`, `:-1`, `:asc`, or `:desc`:

```bash
txbase index build-compound path/to/users.dbf by_score_name SCORE:-1 NAME:1
```

Verify the sidecar against the current DBF and memo bytes:

```bash
txbase index verify path/to/users.dbf
```

Rebuild the existing definitions after a table mutation:

```bash
txbase index rebuild path/to/users.dbf
```

`index rebuild` also migrates a version 1 sidecar to the current format when its definitions can be read.

A version 1 or version 2 sidecar is not used for query planning until it has been rebuilt.

`load` and `verify` recover the DBF first, then compare the stored source fingerprint.

When a `.txidx` file already exists, normal DBF persistence records its target contents in the same durable WAL as the DBF or memo target.

The target sidecar is replaced after the WAL state records are synced.

If the replacement is interrupted, startup recovery replays the DBF or memo target and the index target before clearing the WAL.

If target generation or application cannot be completed, the DBF remains the source of truth and the next `load` or `verify` rejects the stale or invalid sidecar.

A DBF or memo change returns a stale-index error instead of returning potentially wrong record numbers.

Rebuild preserves the definitions in the existing sidecar and regenerates entries from active records.

Writes use a synced temporary file followed by replacement of the `.txidx` path.

`backup` and `restore` validate and copy a present `.txidx` sidecar together with the DBF,
memo, and schema sidecars. The source index must be fresh and structurally valid; a stale or
malformed source is rejected instead of creating a backup that would look queryable but use
wrong record numbers. If the source has no index, an old destination `.txidx` is removed.

The index remains derived state, so a backup without a `.txidx` can still be restored and rebuilt
explicitly at the destination.

## Crash boundary

The current contract treats the index as derived state, but gives a valid existing sidecar a durable recovery target alongside the DBF or memo target.

The WAL is synced before any target replacement.

Recovery reapplies the target records idempotently and clears the WAL only after the DBF and memo writes have completed.

An index target with a matching source fingerprint is then installed with the same synced temporary-file replacement used by normal index writes.

If a crash leaves an old or invalid sidecar visible, its source fingerprint and exact-entry validation reject it before query planning can use it.

The path-aware executor falls back to a table scan, so a stale optional index does not change query results.

This is a WAL-backed crash-atomic recovery contract for valid target records, not one filesystem rename across the DBF, memo, and index files.

It depends on the file system honoring the file and directory sync operations used by the repository.

## Current boundary

The sidecar currently supports build, exact equality lookup, range candidate lookup, histogram-estimated range ordering, single-field ordered traversal, ordered-prefix traversal for multi-key sorts, per-field-direction compound-key construction and prefix traversal, equality candidate intersection across multiple single-field indexes, uniform-statistics ordering for that intersection, stale detection, validation, rebuild, and WAL-backed refresh after normal persistence or recovery.

DBF insert, update, logical delete, `PACK`, and `RECALL` refresh an existing sidecar when their DBF save completes normally.

Schema-preserving XBF-to-DBF export also refreshes an existing sidecar after its
`TXSE` journal applies the new DBF state; the XBF input does not provide an
index to copy.

Direct DBF edits, unsupported sidecar definitions, and refresh I/O failures leave the sidecar stale; `index rebuild` is the explicit repair path.

The path-aware query executor uses equality, equality intersection, range, single-field ordered, or compound-prefix ordered sidecar traversal when it can prove that the lookup is valid, then applies the normal filter pipeline to the candidate records.

For a multi-key sort, a single-field index supplies the first sort-key order and the executor stably sorts only equal-key groups by the remaining keys.

When the requested sort fields match a compound definition after an exact equality prefix, the planner can consume the index order directly for the requested directions or their complete reverse.

The planner compares the exact candidate counts of equality, range, and compatible ordered
definitions and prefers the smallest candidate set. Equal compound candidates then prefer the
shortest definition and a stable name tie-breaker.

This is a local candidate-count heuristic, not a full I/O, memory, or statistics cost model.

The planner now records the active-record count and derives each single-field index's distinct-key count from its entries.

It estimates an equality candidate count by assuming a uniform distribution, orders the candidate indexes by that estimate, and then uses exact record lists for the intersection.

The equality estimate is a local statistic, not a histogram or a cost-based planner.

For multiple range predicates, the planner sums the record counts of overlapping histogram buckets and tries the smallest estimate first.

An overlapped bucket is counted in full, so the estimate is intentionally coarse; exact range candidates still determine the returned records.

The path-less `QueryExecutor` implementation remains a table-scan reference path.

The range candidate lookup narrows the typed key domain and uses binary seeks for the lower and upper bounds.

It still materializes candidate record numbers and sorts them by physical DBF order before the normal filter pipeline.

`IndexFile::load` still validates the sidecar by rebuilding expected entries from the current DBF, so the binary-seek contract does not yet claim an end-to-end speedup.

Full cost-based index choice, collation-aware planning, and cross-table atomic commits require separate contracts.

The equality, equality-intersection, statistics-ordered, histogram-ordered range, single-field ordered, ordered-prefix, and compound-prefix planners are tested alongside mutation, recovery, stale-index, rebuild, and DBF/index WAL-target behavior; broader index support still needs a full cost model and cross-table contracts.
