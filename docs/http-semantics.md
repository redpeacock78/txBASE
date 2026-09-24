# HTTP method semantics

txBASE uses HTTP method names for their defined semantics.

It does not treat every method as an arbitrary RPC verb.

The current API is intentionally small and JSON-only.

## 1. Core method properties

[RFC 9110](https://www.rfc-editor.org/rfc/rfc9110.html) defines three properties that affect clients, intermediaries, retries, and caches.

**Safe** means the client did not request a state-changing action.

**Idempotent** means repeating the same request has the same intended effect as making it once.

**Cacheable** means a response may be stored and reused when the method and response metadata permit it.

Safe does not mean that the server has no incidental side effects such as logging.

The common method properties are:

| Method | Safe | Idempotent | txBASE role |
| --- | --- | --- | --- |
| `GET` | Yes | Yes | Read records or one record |
| `HEAD` | Yes | Yes | Read headers for records or one record |
| `OPTIONS` | Yes | Yes | Advertise supported methods and the JSON query media type |
| `TRACE` | Yes | Yes | Not implemented |
| `POST` | No | No | Create a physical DBF record |
| `PUT` | No | Yes | Replace an active record |
| `PATCH` | No by default | No by default | Partially modify an active record |
| `DELETE` | No | Yes | Write the DBF deletion marker |
| `QUERY` | Yes | Yes | Execute a query with request content |

The properties describe method intent.

They do not by themselves provide transaction isolation, deduplication, or a retry-safe network implementation.

## 2. Current txBASE routes

`OPTIONS` returns `204 No Content`, an `Allow` header for the server surface, `Accept-Query: "application/json"`, and `Accept-Patch: application/json, application/merge-patch+json, application/json-patch+json`.

The response does not authorize a method on a resource that its route rules would otherwise reject.

| Route | Request contract | Response contract |
| --- | --- | --- |
| `GET /records` | No JSON body | Active records as JSON |
| `GET /records/{id}` | One-based physical DBF record number | One active record or `404` |
| `HEAD /records` and `HEAD /records/{id}` | Same target selection as `GET` | Same status and representation headers without response content |
| `QUERY /records` | `Content-Type: application/json` and a query document | Filtered JSON result with `Accept-Query`; paged queries return `records` and `cursor` |
| `QUERY /records/stream` | `Content-Type: application/json` and a stream-compatible query document | Chunked `application/x-ndjson`, one record per line |
| `GET /cdc` and `HEAD /cdc` | Optional `after` and `limit` query parameters | Bounded committed `TXCD` event page with `next_after` |
| `QUERY /explain` | `Content-Type: application/json` and a query document | Selected table-scan or index plan with `Accept-Query`; a valid index sidecar also adds deterministic row-equivalent cost fields |
| `GET /catalog` and `HEAD /catalog` (catalog server) | No JSON body | Discovered table schemas with a strong catalog `ETag`; conditional requests may return `304` |
| `GET /cdc` and `HEAD /cdc` (catalog server) | Optional `after` and `limit` query parameters | Bounded committed `TXCC` event page with `next_after` |
| `GET`/`HEAD /{table}/records[/{id}]` (catalog server) | No JSON body | Named-table records |
| `QUERY /{table}/records` (catalog server) | `Content-Type: application/json` and a query document | Filtered named-table records with `Accept-Query` |
| `QUERY /{table}/records/stream` (catalog server) | `Content-Type: application/json` and a stream-compatible query document | Chunked `application/x-ndjson`, one named-table record per line |
| `QUERY /{table}/explain` (catalog server) | `Content-Type: application/json` and a query document | Named-table query plan with `Accept-Query` |
| `QUERY /join` (catalog server) | `Content-Type: application/json` and a bounded join document | Joined JSON result with `Accept-Query` |
| `POST /{table}/records` (catalog server) | JSON object with known fields | `201 Created`, table-qualified `Location` |
| `PUT`/`PATCH`/`DELETE /{table}/records/{id}` (catalog server) | Same body and precondition rules as single-table routes | Independent named-table mutation |
| `POST /transaction` (catalog server) | JSON object containing named-table mutation operations | `200` with the new catalog `ETag` after catalog-journal commit, or `412` without mutation for a failed `If-Match` or matching `If-None-Match` |
| `GET`/`HEAD /replication/status` (catalog server) | No JSON body | Versioned replication position, catalog representation, follower count, and safe compaction index |
| `GET`/`HEAD /replication/snapshot` (catalog server) | No JSON body | Validated `ReplicationSnapshot` JSON |
| `POST /replication/entry` (catalog server) | Versioned `ReplicationEntry` JSON | `200` apply or duplicate result |
| `POST /replication/snapshot` (catalog server) | Versioned `ReplicationSnapshot` JSON | `200` install or duplicate result |
| `POST /replication/progress` (catalog server) | Versioned `ReplicationProgress` JSON | `200` acknowledgement with the current safe compaction index |
| `POST /records` | JSON object with known fields | `201 Created` and `Location` |
| `POST /transaction` | JSON object containing a non-empty `operations` array | `200` after one-table atomic snapshot commit |
| `PUT /records/{id}` | JSON object replacing fields | Resulting record |
| `PATCH /records/{id}` | `application/json` update document, `application/merge-patch+json` object, or `application/json-patch+json` array | Resulting record |
| `DELETE /records/{id}` | No JSON body | `204 No Content` |

`PUT` requires `application/json`, while `PATCH` accepts `application/json`, `application/merge-patch+json`, and `application/json-patch+json`.

Unknown fields, malformed JSON, unsupported update operators, and invalid field values are rejected before persistence.

Every JSON request body except `POST /replication/snapshot` is capped at
`MAX_JSON_INPUT_BYTES`, currently 1 MiB, before parsing.
The replication snapshot route is capped at the 64 MiB encoded snapshot bound.
The HTTP boundary returns `413 Payload Too Large` when the applicable byte limit is exceeded.

Replication delivery validates already-constructed entries, snapshots, or follower progress.
Malformed documents return `422`, term, position, schema, progress, conflicting-duplicate, or snapshot-state conflicts return `409`, and storage failures return `500`.
In the default `authority` role, `/transaction` and named-table mutation routes append their catalog change and `TXRP` state atomically.
The `follower` role rejects direct catalog mutations and progress acknowledgements with `409`; replication delivery remains available.

### Replication authentication

[RFC 6750](https://www.rfc-editor.org/rfc/rfc6750.html) defines the Bearer authorization scheme for HTTP requests.
When `TXBASE_REPLICATION_TOKEN` is configured for `serve-catalog`, all replication routes require `Authorization: Bearer <token>`.
The configured value must be an ASCII `b64token`; txBASE validates it before the server starts and compares the received value without exposing it in an error response.
Missing, malformed, or non-matching credentials return `401 Unauthorized` with `WWW-Authenticate: Bearer` and a JSON error code.
The other catalog routes are not covered by this token boundary.
When the environment variable is absent, the replication routes remain unauthenticated for local development compatibility.
This is application-layer authentication only; the catalog server still does not provide TLS, so a bearer token must not be sent over an untrusted plain-HTTP network.

`DELETE` is a logical DBF deletion.

### Historical catalog reads

Catalog read routes accept an optional `at` query parameter with a positive committed catalog
transaction ID.

The parameter applies to `GET` and `HEAD /catalog`, named-table record reads, named-table
`QUERY` routes including streams and explain, and `QUERY /join`.

Every table read by one request comes from the same retained catalog image.

Historical query and join execution does not use current index sidecars, so historical explain
responses use a table-scan plan.

`at` is not a long-lived session or a row-level transaction.

Catalog mutations with `at` return `405`; zero, malformed, or repeated values return `400`; an
ID that is not a retained committed catalog snapshot returns `422`.

The parameter is not accepted by `/cdc`, whose `after` and `limit` parameters read the current
CDC sidecar, and it is not part of the single-table server.

The physical record number is not reused by the current mutation layer.

Successful `GET /records` and `GET /records/{id}` responses expose a strong `ETag` for the
current table representation. `POST /records`, `PUT /records/{id}`, `PATCH /records/{id}`,
`DELETE /records/{id}`, and `POST /transaction` accept an optional `If-Match` header. A strong
current tag, or `*` when the target exists, permits the mutation; a supplied weak, malformed, or
non-matching condition returns `412 Precondition Failed` without changing the table. An omitted
header preserves the existing behavior. A comma
separated list succeeds when any strong tag matches. Successful mutations return the new `ETag`.
WAL-backed successful mutations also return `X-Txbase-Transaction-Id`. `POST /transaction` repeats
the same positive DBF commit ID as its `transaction_id` JSON member.

The validator is opaque and is not an authenticity or authorization token. It is derived from the
current in-memory DBF bytes and resolved active JSON values, so memo-backed values participate in
the representation identity.

`GET /records` and `HEAD /records` (and their `/{id}` forms) also accept `If-None-Match`. A matching strong or weak tag,
or `*` for an existing resource, returns `304 Not Modified` with the current `ETag` and no body;
an unmatched value returns the normal representation. This cache-validation behavior is currently
limited to `GET` and `HEAD` for `304`; mutation routes use the same weak comparison and return `412 Precondition Failed` when the condition matches.

`POST /records`, `PUT`, `PATCH`, `DELETE`, and single-table `POST /transaction` accept `If-None-Match`.
A matching strong or weak tag, or `*` for an existing target, returns `412 Precondition Failed` with the current `ETag` and performs no mutation.
An unmatched condition permits the operation.
The `/records` collection and single-table transaction target are existing resources, so `*` rejects those operations.
`GET` and `HEAD /catalog` expose a strong catalog representation `ETag`.
A matching strong or weak `If-None-Match`, or `*`, returns `304 Not Modified` with the current
`ETag` and no body.
The catalog-wide `POST /transaction` accepts optional `If-Match` and `If-None-Match` conditions.
They are evaluated under the catalog write lock.
`If-Match` requires the current strong tag or `*`; a weak or non-matching value returns `412
Precondition Failed` with the current `ETag` without mutating any DBF or sidecar.
A matching strong or weak `If-None-Match`, or `*`, has the same result.
An unmatched condition permits the transaction, and a successful commit returns the new catalog
representation `ETag`.

## 3. PATCH

[RFC 5789](https://www.rfc-editor.org/rfc/rfc5789.html) defines `PATCH` as applying a patch document to a resource.

The patch document media type and its processing rules are part of the API contract.

`PATCH` is not safe or idempotent by default.

An API can make a particular patch idempotent through its document semantics or through a conditional request.

For collision-sensitive patches, the RFC recommends conditional requests such as `If-Match` with a strong entity tag.

txBASE currently accepts `application/json` plain field patches and the typed `$set`, `$unset`, and `$inc` subset described in [the mutation model](mutation-model.md).

It also accepts `application/merge-patch+json` for record `PATCH` requests.

The merge-patch document must have an object root.

Object members are merged recursively, while `null` removes a member and arrays or scalar values replace the current value.

DBF records have a fixed scalar schema, so removing a known field is persisted as its normalized `null` value, and an object value for a scalar field is rejected.

It also accepts `application/json-patch+json` for record `PATCH` requests.

The JSON Patch document is an array of at most 100 RFC 6902 operations.

txBASE supports `add`, `remove`, `replace`, `test`, `move`, and `copy` with RFC 6901 JSON Pointer paths.

The record root must remain an object.

Removing a known DBF field is persisted as normalized `null`, and an operation or final DBF validation failure returns `422` without applying the mutation.

txBASE implements strong table and catalog representation tags, optional `If-Match` protection for
the state-changing routes described above, and `If-None-Match` validation for reads, single-table
mutations, and catalog-wide transactions.

The MongoDB-shaped update document is an application format inside the JSON body.

It is not the definition of HTTP `PATCH`.

## 4. QUERY

[RFC 10008](https://www.rfc-editor.org/rfc/rfc10008.html), published as an IETF Standards Track RFC in June 2026, defines the HTTP `QUERY` method for safe, idempotent requests whose query semantics are carried in request content.

The request content type identifies the query syntax.

A server must not silently reinterpret a missing or inconsistent `Content-Type`.

The RFC defines the following useful failure boundaries:

| Condition | Status boundary |
| --- | --- |
| Missing required query media type | `400 Bad Request` |
| Unsupported query media type | `415 Unsupported Media Type` |
| Understood media type but invalid query content | `422 Unprocessable Content` |
| No acceptable response representation | `406 Not Acceptable` |

`Accept-Query` advertises supported query media types in the response.

It is a Structured Fields list, not an unstructured comma-separated string.

RFC 10008 also defines optional `Location` and `Content-Location` uses for stored queries and query results, but txBASE does not currently create those resources or return those fields.

txBASE advertises `Accept-Query: "application/json"` and accepts only the JSON query document.

The query body is part of the request semantics.

A future cache must not key a QUERY response only by the URI while ignoring the body.

The current server supports one byte range on a successful QUERY response.

It returns `206 Partial Content` or `416 Range Not Satisfiable` for supported single byte ranges and ignores unsupported or multiple ranges.

Range handling is a representation transfer feature and does not change QUERY's method semantics.

When a query includes `page_size`, the successful JSON representation is an object with
`records` and a nullable `cursor` member.
Without `sort`, the emitted cursor is a versioned token containing the physical DBF record
position and a table-representation snapshot tag.
With `sort`, it is a versioned keyset token bound to the same sort fields, directions, and
snapshot tag.
Reusing an emitted cursor after the table changes returns `422` instead of mixing pages from
different representations. Legacy untagged cursors remain accepted for compatibility, and
neither form can be combined with `skip`.

`QUERY /records/stream` and `QUERY /{table}/records/stream` use the same JSON request document as
the pull-based query route but only allow `filter`, `projection`, `skip`, and `limit`.

The successful response has media type `application/x-ndjson` and emits one compact JSON record
per line without a wrapper array or cursor.

The server omits `Content-Length`, so HTTP/1.1 uses chunked transfer while a bounded snapshot
producer supplies records through a fixed-capacity channel.

Malformed input is rejected before streaming with the existing `400`, `415`, or `422` boundary.

The stream has no ETag, byte-range, or resume-token contract.

If evaluation fails after the response headers are sent, the connection terminates and the client
must retry the complete query.

## 5. CDC read routes

`GET /cdc` reads committed change events without changing the table or catalog.

The single-table server returns `TXCD` events from the table sidecar.

The catalog server returns `TXCC` events from the catalog sidecar so one page preserves the atomic multi-table transaction boundary.

Both routes accept an optional exclusive `after` transaction-ID cursor and a `limit` from 1 through 1,000.

The default limit is 100.

The response is `{"events": [...], "next_after": 12}`.

`next_after` is `null` when no later event exists, and the client can pass that value as the next `after` cursor.

The route returns `400` for a zero or non-numeric cursor, an invalid limit, a repeated parameter, or an unknown parameter.

The cursor is not an acknowledgement, lease, or persisted consumer position.

`HEAD /cdc` has the same status and representation headers as `GET /cdc` without response content.

The routes do not expose independent named-table `TXCD` events through the catalog server because those events are not one catalog transaction.

## 6. Persistence and retries

HTTP idempotence does not make the DBF write path crash-safe.

The current mutation path records a durable `TXOP` intent before its state payload, syncs the WAL, and then replaces the DBF or memo sidecar.

Startup recovery replays a supported intent when no state payload exists.

`POST /transaction` applies all operations to a private table copy and persists one snapshot/WAL
commit through `DbfTransaction`. If validation or any operation fails, the copy is discarded and
the current DBF is not changed. The commit ID is durable for that DBF and resumes after restart
or WAL recovery. The current boundary is one DBF table; it does not provide cross-table atomicity,
catalog-wide transaction IDs, or MVCC visibility. The Rust API boundary is defined in
[Snapshot transactions](transactions.md).

The single-table and catalog transaction routes share the `OperationBatch` decoder with WASM.
They reject empty batches and batches larger than 1,000 operations before applying any operation.

The catalog server's `POST /transaction` accepts `/table/records` and
`/table/records/{id}` mutation paths. It prepares every affected table under one catalog lock,
commits the DBF and changed sidecars through a directory journal, and rolls back an incomplete
prepare on the next catalog read. It provides cross-table atomic commit and crash recovery, and
returns a durable catalog-journal transaction ID in JSON and `X-Txbase-Transaction-Id`. The ID is
an ordering and identification boundary, and it can select a retained one-request HTTP snapshot
with `at`; it does not create a long-lived transaction.

The table lock serializes save paths, and a stale independently loaded table is rejected.

Automatic network retry, request deduplication, and multi-writer merge are not implemented.

Clients must not infer exactly-once effects from a successful TCP exchange alone.

## 6. Future HTTP work

The following require explicit contracts before implementation:

- `Content-Location` and cache-key rules for QUERY bodies.
- Runtime-specific async traits for long-lived query streams.
- CORS and authentication policy.

## Primary references

- [RFC 9110: HTTP Semantics](https://www.rfc-editor.org/rfc/rfc9110.html)
- [RFC 5789: PATCH Method](https://www.rfc-editor.org/rfc/rfc5789.html)
- [RFC 10008: The HTTP QUERY Method](https://www.rfc-editor.org/rfc/rfc10008.html)
