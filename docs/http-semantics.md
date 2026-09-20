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

`OPTIONS` returns `204 No Content`, an `Allow` header for the server surface, and `Accept-Query: "application/json"`.

The response does not authorize a method on a resource that its route rules would otherwise reject.

| Route | Request contract | Response contract |
| --- | --- | --- |
| `GET /records` | No JSON body | Active records as JSON |
| `GET /records/{id}` | One-based physical DBF record number | One active record or `404` |
| `HEAD /records` and `HEAD /records/{id}` | Same target selection as `GET` | Same status and representation headers without response content |
| `QUERY /records` | `Content-Type: application/json` and a query document | Filtered JSON result with `Accept-Query`; paged queries return `records` and `cursor` |
| `QUERY /explain` | `Content-Type: application/json` and a query document | Selected table-scan or index plan with `Accept-Query` |
| `GET /catalog` and `HEAD /catalog` (catalog server) | No JSON body | Discovered table schemas with a strong catalog `ETag`; conditional requests may return `304` |
| `GET`/`HEAD /{table}/records[/{id}]` (catalog server) | No JSON body | Named-table records |
| `QUERY /{table}/records` (catalog server) | `Content-Type: application/json` and a query document | Filtered named-table records with `Accept-Query` |
| `QUERY /{table}/explain` (catalog server) | `Content-Type: application/json` and a query document | Named-table query plan with `Accept-Query` |
| `QUERY /join` (catalog server) | `Content-Type: application/json` and a bounded join document | Joined JSON result with `Accept-Query` |
| `POST /{table}/records` (catalog server) | JSON object with known fields | `201 Created`, table-qualified `Location` |
| `PUT`/`PATCH`/`DELETE /{table}/records/{id}` (catalog server) | Same body and precondition rules as single-table routes | Independent named-table mutation |
| `POST /transaction` (catalog server) | JSON object containing named-table mutation operations | `200` with the new catalog `ETag` after catalog-journal commit, or `412` without mutation for a matching `If-None-Match` |
| `POST /records` | JSON object with known fields | `201 Created` and `Location` |
| `POST /transaction` | JSON object containing a non-empty `operations` array | `200` after one-table atomic snapshot commit |
| `PUT /records/{id}` | JSON object replacing fields | Resulting record |
| `PATCH /records/{id}` | Plain field object or supported update document | Resulting record |
| `DELETE /records/{id}` | No JSON body | `204 No Content` |

`PUT` and `PATCH` require the JSON media type for their request bodies.

Unknown fields, malformed JSON, unsupported update operators, and invalid field values are rejected before persistence.

`DELETE` is a logical DBF deletion.

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
The catalog-wide `POST /transaction` accepts `If-None-Match`; a matching condition is evaluated
under the catalog write lock and returns `412 Precondition Failed` with the current `ETag` without
mutating any DBF or sidecar.
An unmatched condition permits the transaction, and a successful commit returns the new catalog
representation `ETag`.

## 3. PATCH

[RFC 5789](https://www.rfc-editor.org/rfc/rfc5789.html) defines `PATCH` as applying a patch document to a resource.

The patch document media type and its processing rules are part of the API contract.

`PATCH` is not safe or idempotent by default.

An API can make a particular patch idempotent through its document semantics or through a conditional request.

For collision-sensitive patches, the RFC recommends conditional requests such as `If-Match` with a strong entity tag.

txBASE currently accepts `application/json` plain field patches and the typed `$set`, `$unset`, and `$inc` subset described in [the mutation model](mutation-model.md).

txBASE implements strong table and catalog representation tags, optional `If-Match` protection for
the state-changing routes described above, and `If-None-Match` validation for reads, single-table
mutations, and catalog-wide transactions.

It does not yet implement JSON Patch or JSON Merge Patch media types.

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

## 5. Persistence and retries

HTTP idempotence does not make the DBF write path crash-safe.

The current mutation path records a durable `TXOP` intent before its state payload, syncs the WAL, and then replaces the DBF or memo sidecar.

Startup recovery replays a supported intent when no state payload exists.

`POST /transaction` applies all operations to a private table copy and persists one snapshot/WAL
commit. If validation or any operation fails, the copy is discarded and the current DBF is not
changed. The commit ID is durable for that DBF and resumes after restart or WAL recovery. The
current boundary is one DBF table; it does not provide cross-table atomicity, catalog-wide
transaction IDs, or MVCC visibility.

The catalog server's `POST /transaction` accepts `/table/records` and
`/table/records/{id}` mutation paths. It prepares every affected table under one catalog lock,
commits the DBF and changed sidecars through a directory journal, and rolls back an incomplete
prepare on the next catalog read. It provides cross-table atomic commit and crash recovery, and
returns a durable catalog-journal transaction ID in JSON and `X-Txbase-Transaction-Id`. The ID is
an ordering/identification boundary; it does not provide MVCC visibility.

The table lock serializes save paths, and a stale independently loaded table is rejected.

Automatic network retry, request deduplication, and multi-writer merge are not implemented.

Clients must not infer exactly-once effects from a successful TCP exchange alone.

## 6. Future HTTP work

The following require explicit contracts before implementation:

- `Content-Location` and cache-key rules for QUERY bodies.
- HTTP streaming and backpressure.
- CORS and authentication policy.
- A standard patch media type in addition to the local update document.

## Primary references

- [RFC 9110: HTTP Semantics](https://www.rfc-editor.org/rfc/rfc9110.html)
- [RFC 5789: PATCH Method](https://www.rfc-editor.org/rfc/rfc5789.html)
- [RFC 10008: The HTTP QUERY Method](https://www.rfc-editor.org/rfc/rfc10008.html)
