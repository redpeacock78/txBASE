# Mutation model

This document defines the local update document and its boundary.

The JSON update format is an application contract inside txBASE.

It does not define HTTP `PATCH` by itself and does not claim MongoDB compatibility.

## 1. Typed update operators

`PATCH` accepts either a plain field object or a typed update document.

The current typed subset is:

```json
{
  "$set": {"NAME": "Caroline"},
  "$unset": {"TEMP": true},
  "$inc": {"COUNT": 1}
}
```

An update document cannot mix operators with plain fields.

A field cannot be modified more than once in one update document.

Unknown fields and writes to auto-increment fields are rejected.

txBASE rejects unsupported operators instead of silently treating them as field names.

MongoDB has a much larger [update operator reference](https://www.mongodb.com/docs/manual/reference/mql/update/), including array and arithmetic operators.

Those operators are not part of the current txBASE contract.

## 2. Mutation and atomicity

HTTP idempotence does not make the DBF write path crash-safe.

The current mutation path records a durable `TXOP` intent before its state payload, syncs the WAL, and then replaces the DBF or memo sidecar.

Startup recovery replays a supported intent when no state payload exists.

`POST /transaction` applies all operations to a private table copy and persists one snapshot/WAL commit.

If validation or any operation fails, the copy is discarded and the current DBF is unchanged.

The commit ID is durable for that DBF and resumes after restart or WAL recovery.

This single-table boundary does not provide cross-table atomicity, catalog-wide transaction IDs, or MVCC visibility.

The catalog server's `POST /transaction` is the separate cross-table boundary.

It prepares affected tables under one catalog lock, commits DBF and changed sidecars through a directory journal, and rolls back an incomplete prepare on the next catalog read.

It returns a durable catalog-journal transaction ID in JSON and `X-Txbase-Transaction-Id`.

That ID identifies and orders a catalog commit.

It does not provide MVCC visibility.

The table lock serializes save paths, and a stale independently loaded table is rejected.

Automatic network retry, request deduplication, and multi-writer merge are not implemented.

Clients must not infer exactly-once effects from a successful TCP exchange alone.

## 3. HTTP boundary

[RFC 5789](https://www.rfc-editor.org/rfc/rfc5789.html) defines `PATCH` as applying a patch document to a resource.

The patch document media type and its processing rules are part of the API contract.

`PATCH` is not safe or idempotent by default.

An API can make a particular patch idempotent through document semantics or a conditional request.

For collision-sensitive patches, the RFC recommends conditional requests such as `If-Match` with a strong entity tag.

txBASE accepts `application/json` plain field patches and the typed subset above.

It implements a strong table representation tag and optional `If-Match` protection for state-changing routes.

GET and HEAD provide `If-None-Match` cache validation, and single-table mutation routes use the same condition with `412 Precondition Failed` when the weak comparison matches.

The catalog-wide transaction endpoint has no catalog representation ETag yet.
JSON Patch and JSON Merge Patch media types are not implemented.

The MongoDB-shaped update document is an application format inside the JSON body.

It is not the definition of HTTP `PATCH`.

## 4. Not a MongoDB wire protocol

txBASE does not implement BSON, the MongoDB wire protocol, JavaScript expressions, MongoDB collation, MongoDB aggregation pipelines, MongoDB indexes, or the complete update operator set.

The JSON syntax is a deliberately small local API.

The [MongoDB atomicity and transactions guide](https://www.mongodb.com/docs/manual/core/write-operations-atomicity/) helps separate single-record mutation from future multi-record transaction guarantees.

No multi-record atomicity should be inferred from `$inc` or from the current HTTP `PATCH` route.

## 5. Future mutation work

The following require explicit contracts before implementation:

- A standard patch media type in addition to the local update document.
- A catalog representation ETag and validator for the catalog-wide transaction endpoint.
- Broader multi-record mutation semantics and visibility rules.
- Differential tests against a small mutation reference model.

## Primary references

- [RFC 5789: PATCH Method](https://www.rfc-editor.org/rfc/rfc5789.html)
- [MongoDB update operators](https://www.mongodb.com/docs/manual/reference/mql/update/)
- [MongoDB atomicity and transactions](https://www.mongodb.com/docs/manual/core/write-operations-atomicity/)
- [HTTP method semantics](http-semantics.md)
