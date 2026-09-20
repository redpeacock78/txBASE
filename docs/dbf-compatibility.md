# DBF and dBASE compatibility

This document separates the published file format from the subset currently implemented by txBASE.

The goal is readable, recoverable DBF access first.

It is not a claim of full dBASE or Visual FoxPro compatibility.

## 1. Physical DBF structure

The [dBASE Level 7 file format](https://www.dbase.com/Knowledgebase/INT/db7_file_fmt.htm) defines a header followed by field descriptors, records, and an optional end-of-file marker.

The important header positions are:

| Offset | Size | Meaning |
| ---: | ---: | --- |
| `0` | 1 | File version and memo-related flags |
| `1..3` | 3 | Last-update date as year, month, and day bytes |
| `4..7` | 4 | Record count, little-endian |
| `8..9` | 2 | Header length, little-endian |
| `10..11` | 2 | Record length, little-endian |
| `28` | 1 | Production MDX flag |
| `29` | 1 | Language-driver identifier |
| `32..` | variable | Field descriptors |
| descriptor end | 1 | Field-descriptor terminator |

Classic descriptors are 32 bytes.

dBASE Level 7 descriptors are 48 bytes and can carry extended properties after the descriptor terminator.

The descriptor contains the field name, type, byte offset, width, decimal count, and type-specific flags.

Level 7 auto-increment descriptors also carry an initial value and step information.

The record area starts at the declared header length.

Each physical record begins with a deletion flag byte, so a DBF record width includes that byte.

The declared record count and record length are boundaries, not suggestions.

A reader must reject truncated headers, descriptors, and records instead of reading beyond the declared structure.

## 2. Memo and binary sidecars

Memo fields store a block pointer in the DBF record and keep the payload in a sibling memo file.

The dBASE format uses DBT sidecars, while Visual FoxPro commonly uses FPT sidecars.

Block zero is a sidecar header in the formats supported by txBASE.

The pointer byte order and block-header byte order are format-specific, so the implementation does not treat all memo files as one generic byte stream.

The current write paths are:

| Sidecar | Current txBASE behavior |
| --- | --- |
| dBASE III DBT | Text and binary blocks are appended with the format terminator rules; the `0x1a1a` binary terminator is reserved |
| dBASE IV DBT | Text and binary blocks are appended using the block size declared by the sidecar header |
| Visual FoxPro FPT | Text blocks and type-0 binary blocks are appended with FPT block headers |

The [Visual FoxPro table file structure](https://techshelps.github.io/MSDN/FOXHELP/html/contable_file_structure_lp.dbfrp.htm) and [memo file structure](https://vfphelp.com/help/html/74f53aef-fd56-4f1a-a413-4f045922db21.htm) document the FoxPro-specific structures.

## 3. Field mapping in txBASE

The parser reads the declared field descriptors and maps supported values to JSON.

The following table is the compatibility boundary, not a general DBF type guide.

| Field family | Current behavior |
| --- | --- |
| Character and text | Decoded through the supported language-driver mapping; writes reject unrepresentable characters |
| Date | Converted to JSON date text when valid |
| Numeric and logical | Converted to JSON numbers or booleans when valid |
| Integer and double | Read and written through their fixed-width representations |
| Visual FoxPro `B` with width 8 | Treated as a double |
| Visual FoxPro `Y` | Exposed as a four-decimal fixed-point string to avoid `f64` rounding |
| Visual FoxPro `T` | Exposed as a second-precision ISO-8601 string from the Julian-day and millisecond pair |
| Visual FoxPro `V` and `Q` | Variable-length text or binary values use the fixed record slot and `_NullFlags` rules; `Q` is lowercase hex |
| Visual FoxPro `W` | FPT binary block exposed as lowercase hex |
| `M` | Text from DBT/FPT, or lowercase hex when the binary flag is set |
| `B`, `G`, and `P` | Binary sidecar payloads exposed as lowercase hex; Visual FoxPro `P` is treated as a picture block |
| Level 7 `+` and FoxPro `0x31` | Omitted insert values are assigned from the descriptor; existing values are read-only |

Visual FoxPro nullable tables use `_NullFlags` internally.

The field is hidden from the JSON document and regenerated only for affected inserts or updates.

The [Visual FoxPro variable-length field description](https://vfphelp.com/help/html/465e7a94-51b7-4e0c-98f9-432864fe5bcc.htm) is the reference for the `V` and `Q` slot rules.

## 4. Encoding and CJK boundaries

The language-driver byte declares how character bytes should be interpreted.

txBASE currently supports the code pages implemented in `src/dbf/codepages.rs`, including CP437, CP850, CP852, CP866, Windows-1250, Windows-1251, Windows-1252, Windows-1253, Windows-1254, Windows-1255, and Windows-1256 mappings used by the supported driver IDs.

The current CJK slice also decodes and encodes the Visual FoxPro driver IDs documented by
[Code Pages Supported by Visual FoxPro](https://www.vfphelp.com/help/html/a3d7b0e0-8320-44b1-8983-17c30a78c6c4.htm):

| Driver ID | Declared platform | Effective codec |
| --- | --- | --- |
| `0x7b` | Japanese Windows | Windows-31J/CP932 through `encoding_rs::SHIFT_JIS` |
| `0x7a` | Simplified Chinese Windows | GBK/CP936 |
| `0x79` | Korean Windows | EUC-KR/CP949 |
| `0x78` | Traditional Chinese Windows | Big5/CP950 |

The `encoding` member in `schema` output identifies every supported declared code page, including
the four CJK codecs above.
The `encoding_metadata` member exposes `declared`, `effective`, and `source` values so a reader
can reproduce the interpretation.
`source` is `language-driver`, `explicit-override`, or `fallback`; sidecar and invocation
overrides intentionally share the `explicit-override` value.
The codec treats malformed byte sequences as U+FFFD on read.
Writes reject unmappable characters before DBF bytes are changed.
The existing fixed-field width check remains a byte-width check, so a multibyte value that does
not fit is rejected rather than truncated.

An optional `encoding` property in the `*.txschema.json` sidecar can explicitly select one of the
four declared codecs or the explicit-only `Shift_JIS`, `EUC-JP`, `GB18030`, and `ISO-2022-JP`
codecs using
`windows-31j`/`cp932`, `shift_jis`/`shift-jis`/`sjis`, `gbk`/`cp936`, `euc-kr`/`cp949`,
`big5`/`cp950`, `euc-jp`, `gb18030`, or `iso-2022-jp`/`iso2022-jp`.
The normalized selection is visible as `encoding_override` in `schema` output.
It is also visible as the `effective` member of `encoding_metadata`.
This override is applied before character decoding and encoding; it does not change the DBF header
language-driver byte.

The path-oriented read, schema, verify, pack, recall, and server commands also accept
`--encoding NAME` for the same eight explicit codecs.
The invocation override takes precedence over the sidecar override, is not written to the DBF
header or metadata sidecar, and remains visible as the effective `encoding_override` in schema
output.

The explicit `Shift_JIS` override is strict: it accepts ASCII, half-width Katakana, and JIS X 0208
characters, while CP932 extension bytes decode as U+FFFD and CP932-only characters are rejected on
write. It does not change the DBF language-driver byte.

Unknown drivers retain the existing UTF-8 or lossy fallback behavior.

Writes reject characters that the selected code page cannot represent.

DBF field width is a byte width.

A future CJK compatibility layer must therefore define all of the following together:

1. The declared driver and any explicit override.
2. The codec used for decoding and encoding.
3. The byte-width rule for truncation and validation.
4. The collation rule used by query and sort.
5. The fixture that proves round-trip behavior.

Shift_JIS and CP932 are not interchangeable labels.

The same caution applies to EUC-JP, GBK, GB18030, Big5, and Korean encodings.

The compatibility suite also round-trips the upstream Visual FoxPro `cp1251.dbf` fixture through
its declared Windows-1251 driver, including Cyrillic field values and a persisted rewrite.

The current override slice covers both explicit invocation and sidecar selection for the four
declared-driver codecs plus strict Shift_JIS, EUC-JP, GB18030, and ISO-2022-JP.
Pinned DBF fixtures cover the four declared CJK drivers and round-trip their multibyte record
values. Pinned explicit-codec byte fixtures cover DBF record decoding and write round-trips for
all eight supported explicit codec names. Query sorting has a bounded Unicode-lowercase mode; locale-aware
CJK collation and broader upstream CJK fixtures remain future work.

Classic field descriptor names use the same effective codec as character values, so CJK column names
remain usable as JSON keys and mutation targets. The descriptor width limit remains a byte limit;
XBF export still requires ASCII field names.

## 5. Persistence and recovery

DBF compatibility is coupled to the mutation boundary because a memo pointer and its sidecar payload must agree after a crash.

txBASE uses these records in its WAL:

| Record | Role |
| --- | --- |
| `TXOP` | Durable HTTP mutation intent |
| `TXTI` | Positive DBF WAL commit ID |
| `TXDP` | Byte-range delta when smaller than a full replacement |
| `TXDB` | Complete DBF snapshot |
| `TXDM` | Complete DBF and memo snapshot |

The WAL is synced before the replacement of the DBF or memo sidecar.

WAL-backed single-table commits also persist the positive commit ID in a `TXTI` WAL record and
the `*.txbase.state` sidecar. Recovery writes that sidecar before clearing the WAL. Legacy DBFs
without the sidecar start at commit ID 1 on their first WAL-backed save.

Startup recovery accepts an already-applied target, rejects a mismatched delta base, and replays a supported `TXOP` when no state payload exists.

A stale path-loaded table is rejected when the DBF, memo, schema, or transaction-state bytes changed
after the table was loaded.

This is a recoverability contract for the current prototype, not a multi-writer replication protocol.

## 6. Maintenance commands

The CLI now exposes the first local-database maintenance boundary:

| Command | Behavior |
| --- | --- |
| `txbase schema FILE` | Prints parsed DBF header metadata and field descriptors as JSON |
| `txbase verify FILE` | Loads the DBF, validates detected memo data, reparses the serialized DBF, and checks record boundaries |
| `txbase pack FILE` | Removes logically deleted records, renumbers the remaining physical records, and persists the result through the existing WAL |
| `txbase recall FILE RECORD` | Restores one logically deleted record through the existing WAL |
| `txbase backup SOURCE DEST` | Validates `SOURCE`, then copies its DBF, detected `.dbt` or `.fpt`, schema, and valid `.txidx` sidecars |
| `txbase restore SOURCE DEST` | Uses the same validated copy protocol with the backup as `SOURCE` |

The copy operation replaces each destination file through a synced temporary file.

DBF and memo sidecar replacement is still a sequence of file operations, not a new multi-file transaction protocol.

An interrupted copy should therefore be followed by `txbase verify DEST` before the destination is used.

`PACK` does not compact memo sidecars. An existing index sidecar is refreshed after the packed DBF is saved, but memo blocks remain untouched.

Deleted memo blocks can therefore remain as reclaimable orphan space until a sidecar-specific compaction contract exists.

## 7. Deliberate limits

The following are not implemented by the current DBF layer:

- Production-scale secondary-index maintenance, a full selectivity-aware cost model, and broader compound-index planning.
- OLE semantics and arbitrary external memo formats.
- Complete Visual FoxPro expression or command compatibility.
- Automatic merge and retry for concurrent writers.
- Locale-aware CJK collation and full locale-specific ordering.

These items require a contract, external fixtures, failure tests, and a clear ownership boundary before code is added.

## Primary references

- [dBASE Level 7 file format](https://www.dbase.com/Knowledgebase/INT/db7_file_fmt.htm)
- [Visual FoxPro table file structure](https://techshelps.github.io/MSDN/FOXHELP/html/contable_file_structure_lp.dbfrp.htm)
- [Visual FoxPro field descriptors and variable-length fields](https://vfphelp.com/help/html/465e7a94-51b7-4e0c-98f9-432864fe5bcc.htm)
- [Visual FoxPro memo file structure](https://vfphelp.com/help/html/74f53aef-fd56-4f1a-a413-4f045922db21.htm)
- [Visual FoxPro auto-increment fields](https://www.vfphelp.com/vfp9/html/bd6eff0c-2ce5-43b7-ab29-f5360cd2f90e.htm)
- [Visual FoxPro code pages](https://www.vfphelp.com/help/html/a3d7b0e0-8320-44b1-8983-17c30a78c6c4.htm)
- [`encoding_rs` encoding and error behavior](https://docs.rs/encoding_rs/latest/encoding_rs/struct.Encoding.html)
