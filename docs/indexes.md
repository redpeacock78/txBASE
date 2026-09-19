# Secondary-index sidecar

The current index slice stores a rebuildable secondary index outside the DBF file.

The sidecar path for `users.dbf` is `users.txidx`.

The DBF remains readable by legacy xBase tools because the index is not embedded in the DBF bytes.

## Version 1 contract

An index file is JSON with this top-level shape:

```json
{
  "format": "txbase-index",
  "version": 1,
  "source": {
    "dbf": {"length": 0, "hash": 0},
    "memo": null
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

The real `length` and `hash` values are generated from the DBF and detected memo sidecar.

The hash is an FNV-1a freshness check, not an authenticity or tamper-proof signature.

The initial implementation accepts scalar key values: strings, numbers, booleans, and explicit `null`.

Arrays and objects are rejected because their equality, ordering, and multikey behavior are not yet part of the contract.

Missing fields and explicit `null` have different index keys.

Records use one-based physical DBF record numbers.

Deleted records are not indexed.

Entries are grouped by the canonical JSON encoding of the typed key and record numbers are strictly increasing.

## Lifecycle

Build an index from one or more DBF fields:

```bash
txbase index build path/to/users.dbf NAME AGE
```

Verify the sidecar against the current DBF and memo bytes:

```bash
txbase index verify path/to/users.dbf
```

Rebuild the existing definitions after a table mutation:

```bash
txbase index rebuild path/to/users.dbf
```

`load` and `verify` recover the DBF first, then compare the stored source fingerprint.

A DBF or memo change returns a stale-index error instead of returning potentially wrong record numbers.

Rebuild preserves the definitions in the existing sidecar and regenerates entries from active records.

Writes use a synced temporary file followed by replacement of the `.txidx` path.

The current `backup` and `restore` commands copy DBF and memo files only.

Rebuild the index at the destination instead of treating a missing `.txidx` file as a backup artifact.

## Current boundary

The sidecar currently supports build, exact equality lookup, stale detection, validation, and rebuild.

DBF insert, update, logical delete, `PACK`, and `RECALL` do not update the sidecar automatically.

Those mutations intentionally make the sidecar stale until an explicit rebuild.

The query executor still uses the table scan reference path.

Range scans, ordered index traversal, planner selection, crash-atomic DBF/index commits, and automatic maintenance require separate contracts.

An index is not treated as complete until those mutation, recovery, stale-index, and rebuild behaviors are tested together.
