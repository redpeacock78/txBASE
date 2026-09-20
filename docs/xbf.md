# XBF v1 format draft

XBF is the proposed native txBASE snapshot format.

This document is a design contract, not a compatibility claim.

The repository now contains a bounded v1 codec at
`txbase::xbf::{encode, decode}`, a bounded DBF-to-XBF conversion helper at
`txbase::xbf::{from_dbf, XbfTable::from_dbf}`, and durable snapshot helpers at
`txbase::xbf::{read_path, write_path}`.
The codec validates the draft header, section checksums, schema, directory, typed
records, constraints, and configured size limits.
The full-snapshot `.xwl` path records base and target generations and rejects a
generation mismatch during recovery.
`read_path` and `read_path_with_limits` recover a pending `.xwl` before decoding,
so normal snapshot loads and CLI commands do not observe an older generation.
The bounded `to_dbf` helper exports representable tables to an in-memory DBF
table; it does not claim full DBF schema or type compatibility.
`to_dbf_with_schema` additionally returns a `txbase-schema` JSON value that
preserves representable `primary`, `unique`, and `not_null` constraints for a
caller-managed sidecar.
`save_dbf_with_schema` stages that DBF and sidecar, validates them, and commits
the desired DBF, schema, memo-sidecar, and durable `.txbase.state` state through a durable `TXSE`
journal. The journal records exact base bytes, applies the DBF, schema, and transaction state with
per-file sync-and-replace, removes stale memo variants, and lets normal DBF
reads resume an interrupted export. Recovery rejects a target changed by
another writer instead of overwriting it. This is a crash-recovery and
conflict-detection boundary; it does not claim that external legacy readers
observe all files as one physically atomic snapshot.
When an existing `.txidx` sidecar is present, the export refreshes it from the
committed DBF state instead of leaving a stale candidate index behind.

The codec is intentionally kept in the format layer. It does not add a second
query or HTTP implementation.

## 1. Purpose and boundary

DBF remains the import and preservation format for legacy xBase data.

XBF removes DBF limits that cannot be represented without loss:

- UTF-8 text.
- Explicit `NULL`.
- Variable-length values.
- Signed and unsigned modern integers.
- Timestamps and binary values.
- Schema metadata owned by txBASE.

The compatibility relationship is directional:

```text
DBF -> XBF    lossless when the DBF reader can represent the source value
XBF -> DBF    allowed only after a target representability check
```

XBF is not a DBF extension and does not change the meaning of existing DBF bytes.

## 2. File set

For a table named `users`, the initial file set is:

```text
users.xbf       durable snapshot
users.xwl       optional transaction log
users.xidx      optional external index sidecar
```

The snapshot is self-describing.

The transaction log is separate so a compacted snapshot does not need to retain an unbounded history.

No XBF file may claim a committed generation until the snapshot bytes and its directory entry have been durably written.

## 3. Header

All integer values are unsigned little-endian unless a field below says otherwise.

The v1 header is exactly 100 bytes:

| Offset | Size | Field |
| ---: | ---: | --- |
| `0` | 4 | ASCII magic `TXBF` |
| `4` | 2 | Major version, `1` |
| `6` | 2 | Minor version, `0` |
| `8` | 4 | Feature flags; unknown bits are an error |
| `12` | 4 | Header length, `100` |
| `16` | 8 | Schema offset |
| `24` | 8 | Schema length |
| `32` | 4 | CRC-32C of the schema section |
| `36` | 8 | Record-directory offset |
| `44` | 8 | Record-directory length |
| `52` | 4 | CRC-32C of the record directory |
| `56` | 8 | Record-data offset |
| `64` | 8 | Record-data length |
| `72` | 4 | CRC-32C of the record data |
| `76` | 8 | Record count |
| `84` | 8 | Snapshot generation |
| `92` | 4 | CRC-32C of the header with this field zeroed |
| `96` | 4 | Reserved; must be zero |

CRC-32C uses the Castagnoli polynomial, an initial value of `0xffffffff`, and a final XOR of `0xffffffff`.

The three sections must be non-overlapping, ordered, and wholly contained in the file.

Offsets and lengths are checked with overflow-safe arithmetic before any allocation.

An unknown major version is rejected.

A newer minor version is accepted only when all feature flags are known and the reader can skip every declared section; otherwise it is rejected rather than silently downgraded.

## 4. Schema section

The schema section starts with a `u32` field count.

Each descriptor is encoded in field order:

| Size | Field |
| ---: | --- |
| 2 | UTF-8 field-name length, `1..=255` |
| variable | UTF-8 field name |
| 1 | Type tag |
| 1 | Constraint flags |
| 2 | Reserved; must be zero |

Constraint flags are:

| Bit | Meaning |
| ---: | --- |
| `0x01` | Nullable |
| `0x02` | Primary-key member |
| `0x04` | Unique |

Duplicate names, invalid UTF-8, unsupported type tags, and unknown constraint bits are errors.

The first type-tag set is:

| Tag | Type |
| ---: | --- |
| `0x01` | Boolean |
| `0x10` | Signed 32-bit integer |
| `0x11` | Signed 64-bit integer |
| `0x12` | Unsigned 64-bit integer |
| `0x20` | IEEE-754 32-bit float |
| `0x21` | IEEE-754 64-bit float |
| `0x30` | UTF-8 string |
| `0x31` | Arbitrary bytes |
| `0x40` | UTC date, signed days from Unix epoch |
| `0x41` | UTC timestamp, signed milliseconds from Unix epoch |
| `0x50` | Decimal, reserved until precision and scale are fixed |
| `0x60` | UUID, 16 bytes |
| `0x70` | UTF-8 JSON document |

The `0x50` decimal tag is reserved in v1 and must be rejected until its wire precision is specified.

At most one field may be marked as a primary key in the first implementation.

A primary-key field is implicitly non-null and unique.

Composite keys remain a later format version.

## 5. Record directory

The directory contains exactly `record_count` entries.

Each entry is 24 bytes:

| Offset | Size | Field |
| ---: | ---: | --- |
| `0` | 8 | Absolute record-data offset |
| `8` | 8 | Record payload length |
| `16` | 4 | Record flags |
| `20` | 4 | Reserved; must be zero |

Record flag `0x01` marks a logically deleted record.

Unknown record flags are errors.

Directory entries must point wholly inside the record-data section and must not overlap.

Record order is physical order, preserving the identity model used by the current DBF layer.

## 6. Record payload

The payload contains one value for every schema field, in schema order.

Each value has:

```text
value tag:    u8
payload size: u32
payload:      payload size bytes
```

Tag `0x00` represents `NULL` and must have a zero payload size.

Non-null tags must match the field type, except that an integer field may use a narrower signed integer tag when the value is representable.

Boolean payloads are exactly one byte, `0` or `1`.

UTF-8 values must be valid UTF-8.

The reader rejects a payload that exceeds the configured per-record or per-value limit before allocation.

Trailing bytes, missing fields, duplicate values, and invalid scalar widths are errors.

## 7. Validation and corruption handling

A reader validates in this order:

1. Header size, magic, version, flags, and header checksum.
2. Section bounds and non-overlap.
3. Schema checksum and descriptors.
4. Directory checksum and entries.
5. Data checksum and record payloads.
6. Constraint and record-count invariants.

No partially decoded table is exposed to the query layer.

Checksum failure is corruption, not an empty table.

An invalid snapshot must not be replaced by a best-effort rewrite.

The codec exposes explicit limits for file, section, record, field-name, and value sizes.

## 8. Generations and transaction log

`generation` identifies the committed snapshot generation.

The optional `.xwl` log may contain the existing txBASE operation IR and snapshot records, but its record format must name the XBF generation it was based on.

The current library provides `save_with_wal` and `recover_path` for a full-snapshot record.
Recovery validates the embedded snapshot before applying it, accepts an already-installed target generation idempotently, and rejects a different current generation.
The WAL record, including its XBF-specific header, must fit the transaction layer's 16 MiB record limit.
`write_path` can still persist a larger snapshot within the ordinary XBF file limit, but `save_with_wal` rejects an oversized WAL record before creating the `.xwl` file.

A recovery procedure must reject a log that targets a different base generation.
The normal read boundary runs that procedure automatically and applies the same
configured XBF limits when validating the pending snapshot.

The first XBF implementation may use full-snapshot WAL records.

Byte deltas and logical operation replay are optimizations after the snapshot protocol is proven.

## 9. DBF conversion

The current conversion slice accepts a loaded `DbfTable` and produces an
in-memory `XbfTable`.
It preserves field names, physical record order, deletion flags, decoded values,
binary payloads represented by the DBF layer, and nulls that the DBF reader can
distinguish.
DBF system fields such as `_NullFlags` are not exposed as user fields.
The imported generation is `0` because DBF has no txBASE generation metadata.
Unsupported or malformed DBF field values fail the conversion instead of being
coerced silently.

This helper does not write an XBF file by itself; pass its result to
`write_path` or `save_to`.

`DBF -> XBF` must preserve:

- Field names and values.
- Explicit nulls where the DBF reader can distinguish them.
- Memo and binary payloads.
- Date and numeric values without avoidable precision loss.
- Logical deletion state when the target keeps physical records.

Legacy bytes are decoded before becoming XBF UTF-8 text.

The current `XBF -> DBF` boundary supports an in-memory classic DBF table with
`C`, binary `C`, `N`, `L`, `D`, and Visual FoxPro `T` fields.
The direct `to_dbf` path rejects UUID and JSON values, non-ASCII or over-wide
field names, values over the DBF field limits, unsigned integers above the
exact numeric range, primary or unique constraints, and NULL text or binary
values.
The `to_dbf_with_schema` path accepts primary and unique constraints when the
DBF values are representable and returns their sidecar metadata separately.
The returned `DbfTable` can then use its existing `save_to` or `save_with_wal`
path.

`XbfTable::dbf_export_report` checks the same base DBF representability boundary
without writing files. It reports every field or physical record with a known
conversion problem, whether the table needs a schema sidecar, and a boolean
`representable` result. The report does not claim that a direct `to_dbf` call
will preserve constraints; use the schema-aware export path when
`requires_schema_sidecar` is true.

`save_dbf_with_schema` is the file-level convenience path. The CLI exposes it
as `xbf export XBF DBF --schema`.

The file-level path writes a journal beside the destination and removes it only
after all desired targets are applied. The journal covers the DBF, schema
sidecar, and both DBT/FPT case variants, so a stale memo sidecar is removed as
part of the export. A normal `DbfTable::from_path` call recovers a pending
journal before loading records. An existing external index is refreshed at the
same export boundary; it is derived state and is not copied from the XBF input.

The CLI also exposes `xbf report XBF`, which prints the representability report
without writing a DBF or schema sidecar.

The in-memory conversion does not persist XBF generation or write the sidecar
itself. The file-level helper owns the journal and recovery boundary; callers
do not need to coordinate a second sidecar transaction.

Examples include variable-length UTF-8 text, timestamps, UUIDs, unsupported nullability, and characters outside the selected code page.

Lossy export, if added, must be a separate explicit command or flag and must report the mapping applied.

## 10. Shared upper layers

The query, mutation, planner, and HTTP layers should operate on the same internal record and operation contracts for DBF and XBF.

Only the format layer should own byte layout, checksums, type conversion, and snapshot recovery.

This prevents XBF from becoming a second unrelated database implementation.

## 11. Implementation gates

The draft codec, snapshot writer, and generation-checked full-snapshot WAL currently have a deterministic fixture
covering every non-reserved v1 type, corruption checks, constraint checks,
explicit size limits, and a sync-and-reload path round trip. Before XBF is
advertised as a complete supported format, the repository still needs:

- Malformed header, section, checksum, directory, UTF-8, and payload corpora.
- Round-trip tests for DBF to XBF and representability failures for XBF to DBF.
- Crash and recovery tests for the snapshot, `.xwl` generation boundary, and
  `TXSE` schema-export journal.
- A strict externally visible atomic snapshot contract for legacy readers; the
  current `TXSE` protocol deliberately provides recoverability and conflict
  detection rather than multi-file reader atomicity.

Until those gates exist, XBF remains a draft and is not advertised as a supported format.
