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

The sidecar currently supports build, exact scalar and compound equality lookup, compound equality-prefix candidate lookup, range candidate lookup, compound equality-prefix range candidate lookup, histogram-estimated range ordering, single-field ordered traversal, ordered-prefix traversal for multi-key sorts, per-field-direction compound-key construction and prefix traversal, equality candidate intersection across multiple single-field indexes, uniform-statistics ordering for equality candidates, single-index versus intersection cost choice, stale detection, validation, rebuild, and WAL-backed refresh after normal persistence or recovery.

DBF insert, update, logical delete, `PACK`, and `RECALL` refresh an existing sidecar when their DBF save completes normally.

Mutation persistence validates an existing sidecar before replacing the DBF; a stale or malformed
sidecar therefore leaves the DBF unchanged and reports an index error.

Schema-preserving XBF-to-DBF export also refreshes an existing sidecar after its
`TXSE` journal applies the new DBF state; the XBF input does not provide an
index to copy.

Direct DBF edits, unsupported sidecar definitions, and refresh I/O failures leave the sidecar stale; `index rebuild` is the explicit repair path.

The path-aware query executor uses equality, compound equality-prefix, equality intersection, range, compound equality-prefix range, single-field ordered, or compound-prefix ordered sidecar traversal when it can prove that the lookup is valid, then applies the normal filter pipeline to the candidate records.

For a multi-key sort, a single-field index supplies the first sort-key order and the executor stably sorts only equal-key groups by the remaining keys.

When the requested sort fields match a compound definition after an exact equality prefix, the planner can consume the index order directly for the requested directions or their complete reverse.

When a range field follows an exact equality prefix in a compound definition, the planner can use the matching compound entries as a candidate prefilter.

The compound range lookup first narrows the sidecar to the contiguous equality-prefix interval and scans only that interval.

The candidate path keeps the range comparison in the normal executor, so index direction changes traversal order but not range semantics.

The planner compares the table scan and each valid equality, range, or compatible ordered path
with a bounded integer cost. A table scan costs the active-record count plus remaining sort work.
An index path costs its exact candidate count plus a bounded logarithmic traversal term derived
from the sidecar entry count, plus remaining sort work. An index intersection adds one traversal
term per selected sidecar. An ordered path that supplies the complete requested order has no sort
term. Equal costs preserve the table scan and established access paths before a compound equality-prefix prefilter.

This is a local cardinality, traversal, and sort model, not a full physical I/O, memory, or cache
cost model.

The planner now records the active-record count and derives each single-field index's distinct-key count from its entries.

It estimates an equality candidate count by assuming a uniform distribution, orders the candidate indexes by that estimate, and uses exact record lists to build the intersection candidate.

The equality estimate is a local statistic and one input to the bounded cost model.

For multiple range predicates, the planner uses overlapping histogram buckets to order candidate construction, then selects the lowest bounded cost from the exact range candidates.

An overlapped bucket is counted in full, so the estimate is intentionally coarse; exact range candidates still determine the returned records.

The path-less `QueryExecutor` implementation remains a table-scan reference path.

Compound equality-prefix candidate lookup narrows a compound index to a contiguous leading-key interval and returns physical record order.

Single-field range candidate lookup narrows the typed key domain and uses binary seeks for the lower and upper bounds.

Compound equality-prefix range lookup filters the next indexed component and returns physical record order before the normal filter pipeline.

It still materializes candidate record numbers and sorts them by physical DBF order before the normal filter pipeline.

`IndexFile::load` validates each active DBF record against the loaded sorted entries instead of rebuilding a second full entry vector.

Freshness validation still reads the DBF and memo bytes, and the query executor still materializes candidate record numbers, so this is not a claim of zero-copy or end-to-end index I/O.

A full I/O-aware cost model, collation-aware planning, and cross-table atomic commits require separate contracts.

The equality, equality-intersection, statistics-ordered, histogram-ordered range, compound-prefix range, single-field ordered, ordered-prefix, compound-prefix, and non-selective-index fallback planners are tested alongside mutation, recovery, stale-index, rebuild, and DBF/index WAL-target behavior; broader index support still needs a full I/O-aware model and cross-table contracts.
