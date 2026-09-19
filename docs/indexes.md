# Secondary-index sidecar

The current index slice stores a rebuildable secondary index outside the DBF file.

The sidecar path for `users.dbf` is `users.txidx`.

The DBF remains readable by legacy xBase tools because the index is not embedded in the DBF bytes.

## Version 2 contract

An index file is JSON with this top-level shape:

```json
{
  "format": "txbase-index",
  "version": 2,
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

Entries are grouped by the canonical JSON encoding of the typed key, then ordered by the typed scalar order used by query sorting.

The order is `Missing`, `null`, boolean, number, and string; numeric values use exact integer or floating-point comparison where representable.

Record numbers within one key remain strictly increasing so equal sort keys preserve DBF physical order.

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

`index rebuild` also migrates a version 1 sidecar to the current format when its definitions can be read.

A version 1 sidecar is not used for query planning until it has been rebuilt.

`load` and `verify` recover the DBF first, then compare the stored source fingerprint.

When a `.txidx` file already exists, normal DBF persistence and WAL recovery attempt to refresh it from the committed table state.

This refresh is best effort: a refresh error does not make the DBF mutation fail, and the next `load` or `verify` rejects the stale or invalid sidecar.

A DBF or memo change returns a stale-index error instead of returning potentially wrong record numbers.

Rebuild preserves the definitions in the existing sidecar and regenerates entries from active records.

Writes use a synced temporary file followed by replacement of the `.txidx` path.

The current `backup` and `restore` commands copy DBF and memo files only.

Rebuild the index at the destination instead of treating a missing `.txidx` file as a backup artifact.

## Current boundary

The sidecar currently supports build, exact equality lookup, range candidate lookup, single-field ordered traversal, stale detection, validation, rebuild, and best-effort refresh after normal persistence or WAL recovery.

DBF insert, update, logical delete, `PACK`, and `RECALL` refresh an existing sidecar when their DBF save completes normally.

Direct DBF edits, unsupported sidecar definitions, and refresh I/O failures leave the sidecar stale; `index rebuild` is the explicit repair path.

The path-aware query executor uses equality, range, or single-field ordered sidecar traversal when it can prove that the lookup is valid, then applies the normal filter pipeline to the candidate records.

The path-less `QueryExecutor` implementation remains a table-scan reference path.

The range candidate lookup still scans the validated sidecar entries; it does not yet promise binary range seeks.

Multi-key ordered traversal, multi-index selection, and crash-atomic DBF/index commits require separate contracts.

The equality, range, and single-field ordered planners are tested alongside mutation, recovery, stale-index, and rebuild behavior; broader index support still needs multi-key and crash-atomic contracts.
