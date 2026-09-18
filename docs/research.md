# Specification research and design decisions

This document records the external specifications used to choose the first
implementation boundary. It is a compatibility map, not a claim that txbase
implements any of the referenced products in full.

## DBF and dBASE

The [dBASE DBF file structure](https://www.dbase.com/Knowledgebase/INT/db7_file_fmt.htm)
defines the byte-level facts that the parser must preserve:

- The header stores the version byte, last-update date, record count, header length, record length, language-driver byte, and other flags in fixed offsets.
- Multi-byte header values are little-endian.
- Classic tables use 32-byte field descriptors; the dBASE Level 7 layout uses 48-byte descriptors and can carry field-property data after the descriptor terminator.
- Field data is packed into each record without separators.
- `0x20` marks an active record and `0x2a` marks a deleted record.
- `0x1a` is the documented end-of-file marker.
- Character data is code-page data, while date and numeric fields have textual encodings; memo and binary-like fields refer to blocks in a sidecar file.

The common field encodings are also part of the compatibility contract:

| Type | On-disk representation | Initial txbase behavior |
| --- | --- | --- |
| `C` | Space-padded character bytes | Code-page text normally; binary-flagged `C` is fixed-width lowercase hex without code-page conversion |
| `D` | Eight bytes in `YYYYMMDD` form | String |
| `T` | Eight bytes: little-endian Julian day and milliseconds since midnight | Visual FoxPro `T` is a second-precision ISO-8601 string; non-FoxPro timestamp values remain hex |
| `N` and `F` | Right-justified numeric text | JSON number when finite and parseable |
| `L` | Logical marker such as `T` or `F` | Boolean or JSON null for an unknown marker |
| `I` and `+` | Four-byte integer representation | Little-endian signed integer; Level 7 `+` and Visual FoxPro `0x31` Integer AutoInc inserts use and advance their descriptor values when omitted |
| `Y` | Eight-byte little-endian fixed-point currency | Four-decimal fixed-point string; writes validate the signed 64-bit scaled representation |
| `V` and `Q` | Visual FoxPro `0x32` fixed slots with a trailing length byte selected by `_NullFlags` | `V` text and `Q` lowercase hex; nullable and variable-length bits are maintained on writes while `_NullFlags` remains hidden |
| `W` | Four-byte pointer to a Visual FoxPro `.fpt` binary block | Lowercase hex payload; FPT writes append a type-0 binary block |
| `M` | Text pointer to a memo block | Text from a sibling `.dbt` or `.fpt` sidecar; binary-flagged `M` is lowercase hex; pointer text otherwise |
| `B`, `G`, and `P` | Text pointer to a binary block; Visual FoxPro `B` width 8 is a double and `P` is a picture | Hex payload from a sibling `.dbt` or `.fpt` sidecar when present; dBASE III/IV DBT and FPT writes append a binary block, with dBASE III's `0x1a1a` terminator reserved |

The parser therefore reads the declared header and record boundaries, checks
field names and widths, uses the language-driver byte for the supported
code-page mappings, including Visual FoxPro's Windows-1250 (`0xc8`) and
Windows-1251 (`0xc9`) drivers, and excludes records marked deleted from the JSON read
path. Character writes reject values that the declared code-page mapping
cannot represent. When a sibling `.dbt` or `.fpt` exists, `M` fields are
resolved from their block pointers, while `B`/`G`/`P` payloads are exposed as hex
text. Existing pointers are retained separately
so non-memo DBF mutations do not rewrite them as text. Text changes append to
the existing `.dbt` or `.fpt` sidecar, using the dBASE III terminator, the
dBASE IV header-inclusive length, or the FPT length as appropriate, and update
the DBF pointer in a `TXDM` WAL snapshot; startup recovery replaces both files
from that snapshot. dBASE III binary writes accept hex text and append a block
terminated by the dBASE III `0x1a1a` marker; values that collide with that
marker are rejected. dBASE IV binary writes accept hex text, honor the DBT
header block size, and append a length-delimited binary block, while FPT writes
append a type-0 binary block, including the Visual FoxPro `P` picture type.
OLE semantics and other code-page conversion remain explicit future work.

The reader uses the declared record count as its boundary and does not require
the trailing `0x1a` when the declared records are complete. It still preserves
the documented marker in fixtures, and a future strict-compatibility mode can
report its absence without changing the normal read path.

The sidecar rule is deliberate: WAL, MVCC metadata, indexes, and transaction
state belong in separate files so a checkpointed DBF remains readable by older
xBase tools.

Four-byte memo pointers use big-endian order for FoxPro `0xF5` and Visual
FoxPro `0x30`–`0x32` tables, matching their FPT sidecars.

Visual FoxPro `Y` currency values are exposed as four-decimal fixed-point
strings so the signed 64-bit scaled value is not rounded through `f64`.

Visual FoxPro `T` DateTime values use a little-endian Julian day and
milliseconds-since-midnight pair. txbase exposes valid values as
second-precision ISO-8601 strings and preserves non-FoxPro timestamp fields as
hex.

Visual FoxPro `0x30`–`0x32` nullable fields use `_NullFlags`. In `0x32`, `V`
and `Q` fields additionally use fixed record slots whose final byte stores the
actual length when the corresponding variable-length bit is set. The following
nullable bit marks JSON null. `V` uses the declared code page, `Q` and
binary-flagged `V` use hexadecimal text, and the hidden system field is
regenerated only for affected inserts or updates.

Visual FoxPro `W` Blob fields are four-byte pointers to binary `.fpt` blocks and
do not undergo code-page conversion. txbase exposes those blocks as lowercase
hex and appends type-0 binary blocks on supported FPT writes.

The binary flag on Visual FoxPro `C` and `M` fields also disables code-page
translation. Binary `C` bytes remain in the DBF record; binary `M` bytes use the
same FPT binary-block path as `P` and `W`.

The mutation layer currently reuses the parsed header and field descriptors.
It supports scalar JSON values for the field types already decoded by the
reader, preserves physical record numbers, writes the deletion marker for
logical deletes, assigns omitted Level 7 `+` and Visual FoxPro `0x31`
Integer AutoInc values from their descriptor slots, preserves those read-only
values on existing records, and
replaces the DBF through a synced temporary file.
The transaction module now supplies a length-prefixed `TXWL` file WAL and a
snapshot transaction manager. Before an atomic DBF replacement, the mutation
layer appends a `TXDB` record for DBF-only changes, or a `TXDM` record
containing the complete new DBF and memo snapshots, and syncs the WAL.
`DbfTable::from_path` replays the latest complete snapshot left by an
interrupted mutation.

The transaction WAL stores a four-byte magic, a little-endian payload length,
a monotonically increasing LSN, and the payload. Opening a WAL validates
complete records and truncates only an incomplete final record. It does not
interpret arbitrary payloads. The DBF integration recognizes `TXDB` and `TXDM`
snapshots, while fine-grained mutation records and multi-writer coordination
remain future work. A table loaded from a path records the DBF and memo bytes
it read and refuses to save over an externally changed snapshot.

## MongoDB query ideas

MongoDB documents are BSON documents with field-value pairs, nested documents,
and arrays. The [document model](https://www.mongodb.com/docs/manual/core/document/)
and [query predicate reference](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/)
show why a JSON query shape is useful: field predicates and logical operators
are composable without introducing SQL syntax.

The first txbase vocabulary is intentionally smaller than MongoDB's current
operator set:

| Area | Initial vocabulary | Status |
| --- | --- | --- |
| Field comparison | `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte` | Executed for scalar and array values |
| Membership | `$in`, `$nin` | Executed |
| Boolean composition | `$and`, `$or`, `$not` | Executed |
| Result shaping | `sort`, `projection`, `limit`, `skip` | Executed |

The [comparison operator reference](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/comparison/)
defines the comparison and membership family. txbase uses exact field names,
JSON scalar comparison, explicit type ordering for sort, and deterministic
DBF record order for sort ties. Dotted paths traverse nested JSON objects and
arrays, with an exact field-name match taking precedence; collation is not
supported.

The [logical operator reference](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/logical/)
defines `$and` as requiring every clause, `$or` as requiring at least one
clause, and `$not` as inverting a predicate. Those operators are not merely
string names: the executor validates their JSON shape. An empty `$and` matches
all records and an empty `$or` matches no records.

MongoDB's sort contract uses `1` for ascending and `-1` for descending order.
It permits multiple sort keys, does not promise a stable order for equal keys,
and uses a BSON type ordering when values have different types. See the
[sort reference](https://www.mongodb.com/docs/manual/reference/method/cursor.sort/).
DBF record order is otherwise naturally reproducible, so txbase retains it as
the deterministic tie breaker instead of inheriting MongoDB's unspecified tie
order.

MongoDB projection and update documents have independent semantics. The
prototype validates projection values as `0` or `1`, rejects mixed
inclusion and exclusion, and applies the selected fields. It does not enforce
the full rules described by the
[projection reference](https://www.mongodb.com/docs/manual/reference/mql/projection/).
Update operators use a document of the form `{ "$set": { "field": value } }`.
The [update reference](https://www.mongodb.com/docs/manual/reference/mql/update/)
lists field operators such as `$set`, `$unset`, `$inc`, `$mul`, `$min`, and
`$max`, as well as array operators. txbase currently selects the small typed
subset `$set`, `$unset`, and `$inc`; it rejects the rest rather than silently
treating an unsupported operator as a field name.

MongoDB documents that a write is atomic at the single-document level and that
multi-document writes can interleave unless a transaction is used. See
[atomicity and transactions](https://www.mongodb.com/docs/manual/core/write-operations-atomicity/).
That distinction is useful for txbase: `$inc` is an engine operation, while
multi-record transaction guarantees belong to the transaction layer rather
than to the JSON parser.

The project does not target MongoDB wire compatibility, BSON, aggregation,
JavaScript predicates, or the complete update-operator set.

## Firebase design lessons

Firebase exposes two different data models that should not be conflated.

[Cloud Firestore](https://firebase.google.com/docs/firestore/data-model) uses
documents in collections, with nested objects and subcollections. The
[Realtime Database web guide](https://firebase.google.com/docs/database/web/read-and-write)
uses references into a JSON tree, asynchronous listeners, immediate local
events, and eventual synchronization with the server.

The useful architectural lessons are:

1. A stable path or document identity makes reads, writes, and subscriptions understandable.
2. Local-first behavior must state what is provisional and what is server-committed.
3. Synchronization and conflict policy are part of the data model, not a hidden transport detail.
4. Authorization and validation must run at the server boundary.

The client-first behavior needs a precise durability label. The Realtime
Database web guide says writes produce local events before server persistence,
and its web APIs do not persist offline data outside the current session. The
[offline capabilities guide](https://firebase.google.com/docs/database/android/offline-capabilities)
also states that offline transactions are queued but are not persisted across
an app restart. Firestore's offline cache can answer queries locally and later
synchronize local changes, with last-write-wins behavior for multiple local
writes to the same document. These are client synchronization policies, not
properties that a DBF file automatically provides.

The [Realtime Database security model](https://firebase.google.com/docs/database/security)
separates `.read`, `.write`, `.validate`, and `.indexOn` rules. txbase does
not copy Firebase's rules language, authentication, listener protocol, or
offline client cache. If an edge or client mode is added later, those four
concerns need separate contracts instead of an implicit "Firebase mode".

## SQLite testing and quality

SQLite's [testing overview](https://www.sqlite.org/testing.html) describes
four independently developed harnesses, large parameterized suites, out of
memory and I/O fault injection, crash and power-loss tests, malformed-database
tests, fuzzing, regression tests, and runtime assertions. SQLite also compares
results with optimizations enabled and disabled, and uses differential testing
through SQL Logic Test.

SQLite's [TH3 coverage description](https://sqlite.org/th3.html) reports 100%
branch coverage and 100% MC/DC for the covered core configuration. That is a
quality target for a widely deployed database library, not a meaningful claim
for this first prototype.

The txbase quality path is staged:

1. Keep parser tests for valid headers, field layouts, deleted records, truncated input, and invalid markers.
2. Add an in-memory reference model and compare generated mutation sequences with the DBF mutation layer.
3. Add malformed DBF, memo, index, WAL, and JSON input corpora.
4. Add crash-boundary and recovery tests around WAL sync and checkpoint publication.
5. Add compatibility fixtures produced by independent xBase implementations.
6. Add fuzzing, concurrency tests, cross-platform CI, and differential checks once those components exist.

The current CI gate is deliberately only `fmt`, `clippy`, and `cargo test`.

## HTTP method semantics

[RFC 9110](https://www.rfc-editor.org/rfc/rfc9110.html) defines the shared
HTTP semantics and the method token as case-sensitive. The initial API uses
the standard meaning of each method rather than treating method names as
arbitrary RPC verbs:

| Method | txbase role | Boundary |
| --- | --- | --- |
| `GET` | Read a representation or record | Implemented for `/records` and `/records/{id}` |
| `POST` | Create a physical DBF record | Implemented for JSON scalar fields |
| `PUT` | Full replacement of an active record | Implemented; omitted fields become null |
| `PATCH` | Partial modification of an active record | Implemented for JSON object bodies |
| `DELETE` | Logical deletion | Implemented by writing the DBF deletion marker |
| `QUERY` | Safe query with request content | JSON execution for `/records` |

`PATCH` has its own method specification in
[RFC 5789](https://www.rfc-editor.org/rfc/rfc5789.html). It is not safe by
default, and an API has to define the accepted patch media type and conflict
behavior. txbase therefore does not equate a MongoDB update document with
the HTTP method itself.

The mutation methods require `Content-Type: application/json`. `POST` returns
`201` and a `Location` header, `PUT` and `PATCH` return the resulting record,
and `DELETE` returns `204`. A successful mutation is serialized to temporary
file(s), synced, and renamed over the configured DBF path and, for memo writes,
its sidecar. Before replacement, a complete `TXDB` or `TXDM` snapshot is
appended to the transaction WAL and synced. A subsequent
`DbfTable::from_path` replays that snapshot if the process stopped before
replacement completed. The WAL is still snapshot-based and does not
coordinate concurrent writers.

The [HTTP QUERY method is now RFC 10008](https://www.rfc-editor.org/rfc/rfc10008.html).
It is safe and idempotent, carries query semantics in request content, and
requires the server to reject a missing or inconsistent `Content-Type`. The
RFC also defines `Accept-Query` as a structured response field for advertising
supported query media types. A valid query can use `400` for missing media
type, `415` for an unsupported media type, and `422` for content that is
syntactically understood but cannot be processed.

The server implements those boundary decisions for JSON and advertises
`Accept-Query: "application/json"`. It does not implement cache keys,
`Location`, `Content-Location`, range handling, or CORS policy yet. In
particular, a QUERY body belongs in any future cache key; treating it like a
GET URI alone would be incorrect.

## Sources

- [dBASE DBF File Structure](https://www.dbase.com/Knowledgebase/INT/db7_file_fmt.htm)
- [Visual FoxPro Table File Structure](https://techshelps.github.io/MSDN/FOXHELP/html/contable_file_structure_lp.dbfrp.htm)
- [Visual FoxPro Field Descriptor and Variable-Length Fields](https://vfphelp.com/help/html/465e7a94-51b7-4e0c-98f9-432864fe5bcc.htm)
- [Visual FoxPro Blob Data Type](https://www.vfphelp.com/help/_5wn12pbhl.htm)
- [Visual FoxPro Data Dictionary](https://techshelps.github.io/MSDN/BACKGRND/html/msdn_datadict.htm)
- [Visual FoxPro Memo File Structure](https://vfphelp.com/help/html/74f53aef-fd56-4f1a-a413-4f045922db21.htm)
- [Visual FoxPro Autoincrementing Field Values](https://www.vfphelp.com/vfp9/html/bd6eff0c-2ce5-43b7-ab29-f5360cd2f90e.htm)
- [libxbase dBASE III/IV Memo Implementation](https://sources.debian.org/src/libxbase/2.0.0-8.5/xbase/memo.cpp)
- [MongoDB Documents](https://www.mongodb.com/docs/manual/core/document/)
- [MongoDB Query Predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/)
- [MongoDB Logical Query Predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/logical/)
- [MongoDB Sort Results](https://www.mongodb.com/docs/manual/reference/method/cursor.sort/)
- [MongoDB Projection](https://www.mongodb.com/docs/manual/reference/mql/projection/)
- [MongoDB Update Operators](https://www.mongodb.com/docs/manual/reference/mql/update/)
- [MongoDB Atomicity and Transactions](https://www.mongodb.com/docs/manual/core/write-operations-atomicity/)
- [Firebase Cloud Firestore Data Model](https://firebase.google.com/docs/firestore/data-model)
- [Firebase Realtime Database Read and Write](https://firebase.google.com/docs/database/web/read-and-write)
- [Firebase Realtime Database Security Rules](https://firebase.google.com/docs/database/security)
- [How SQLite Is Tested](https://www.sqlite.org/testing.html)
- [SQLite TH3](https://sqlite.org/th3.html)
- [RFC 9110, HTTP Semantics](https://www.rfc-editor.org/rfc/rfc9110.html)
- [RFC 5789, PATCH Method](https://www.rfc-editor.org/rfc/rfc5789.html)
- [RFC 10008, The HTTP QUERY Method](https://www.rfc-editor.org/rfc/rfc10008.html)
