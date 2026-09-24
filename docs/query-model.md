# Query model

txBASE uses a small JSON query document.

MongoDB supplies vocabulary and comparison points, not a compatibility target.

The query engine evaluates active DBF records after decoding them to JSON.

## 1. Query document

The current document shape is:

```json
{
  "filter": {
    "AGE": {"$gte": 20, "$lt": 30},
    "COUNTRY": {"$in": ["JP", "TW"]},
    "$expr": {"$gt": ["$AGE", "$MIN_AGE"]}
  },
  "sort": {"AGE": 1},
  "projection": {"NAME": 1, "AGE": 1},
  "collation": "unicode-lowercase",
  "skip": 0,
  "limit": 100,
  "page_size": 25,
  "cursor": "42"
}
```

The top-level keys are validated.

Unknown keys are rejected instead of ignored.

`filter` defaults to a match-all filter.

`sort` uses `1` for ascending order and `-1` for descending order.

The optional `collation` currently accepts only `"unicode-lowercase"`.

It lowercases Unicode string sort keys before comparison.

Omitting it keeps the existing Unicode codepoint ordering.

Collation affects sorting only.

Filters and index lookup retain their existing comparison rules.

Collation requires a non-empty top-level `sort`, is unavailable with aggregation, and deliberately uses a table scan.

Sorted cursors encode the collation and reject a token from a different sort contract.

`projection` uses inclusion or exclusion values of `1` and `0`.

`skip` and `limit` are non-negative integer controls.

The executor applies filtering, sorting, projection, skipping, and limiting in that order.

Sort ties preserve DBF record order.

The optional `aggregate` pipeline is defined in [Aggregation model](aggregation.md).

`page_size` enables the bounded cursor response shape:

```json
{
  "records": [{"NAME": "Alice"}],
  "cursor": "42"
}
```

The physical page boundary is the one-based DBF record number after the returned page.

The emitted physical cursor is an opaque versioned JSON string containing that number and a table-representation snapshot tag.

The next request sends the token with the same `page_size` and resumes after that record only when the table representation is unchanged.

When `sort` is present, the cursor contains versioned sort keys, the snapshot tag, and the last physical record number as a deterministic tie-breaker.

The next request must repeat the same sort fields and directions.

Sorted cursors are keyset boundaries, not offsets.

`skip` cannot be combined with either cursor mode, and `page_size` is capped at 1,000.

Reusing a cursor after a table change returns an invalid-query error instead of combining pages from different representations.

This is a cursor snapshot-consistency boundary; historical table snapshots are defined separately in [MVCC and historical snapshots](mvcc.md).

Path-aware single-table `QUERY /records` and `/explain` requests use a txBASE reader boundary.

Before execution, the reader lets the normal DBF recovery path finish, acquires the shared table lock, reloads the DBF, memo, and schema files while holding it, and validates an index sidecar against that table image.

Writers use the exclusive table lock, so cooperating txBASE readers do not combine a table image with an index from another committed generation.

This is a txBASE-reader guarantee.

It does not make a flat DBF and its sidecars physically atomic for legacy readers that ignore the lock.

The in-memory query APIs and `QUERY /records/stream` keep their supplied-table and snapshot-stream contracts.

The old page is not retained for later readers.

Legacy numeric physical cursors and version-1 sorted cursors remain accepted without a snapshot tag for compatibility.

Physical cursor pages scan active records in physical order and stop after one extra matching record proves that another page exists.

They bypass index candidate ordering, and `explain_query_at` reports a table scan for this mode.

This keeps the page result bounded.

Sorted cursor pages use the existing sort comparison and materialize matching record references before selecting the page.

They provide a resumable result boundary but do not claim streaming or backpressure behavior.

The library exposes `query::stream_query` as a borrowed, pull-based iterator over active records.

It applies `filter`, `projection`, `skip`, and `limit` as records are consumed, so it does not materialize the matching record set.

It rejects `sort`, `aggregate`, `page_size`, and `cursor` because those controls require a blocking or resumable result boundary.

The iterator does not provide a long-lived snapshot or an asynchronous backpressure protocol.

`query::stream_query_snapshot` is the stable-snapshot variant.

It clones the loaded table before iteration, so later mutations of the source table do not change its records.

`query::stream_query_bounded` runs the owned snapshot iterator on a producer thread and delivers items through a bounded standard-library channel.

The producer blocks while the channel is full, and dropping the consumer cancels production.

It remains an in-process pull consumer.

`query::stream_query_threaded` exposes the same owned snapshot through the runtime-neutral `AsyncQueryStream` contract on native targets.

It uses a non-blocking `try_recv` in `poll_next`, wakes the registered task after each produced item or end of stream, applies the channel capacity as producer backpressure, and joins the producer when the async stream is dropped.

It is a native host adapter; it does not provide worker/WASI timers, transport, or asynchronous-storage behavior.

The single-table server exposes `QUERY /records/stream`, and the catalog server exposes
`QUERY /{table}/records/stream`.

Both routes accept the same JSON query document but only permit `filter`, `projection`, `skip`,
and `limit` controls.

Each successful response uses `application/x-ndjson` and emits one compact JSON record per line.

The response omits `Content-Length`, so HTTP/1.1 clients receive chunked transfer data as the
bounded producer makes records available.

The server clones the table before starting the producer and uses the same positive-capacity
channel as `query::stream_query_bounded`.

Dropping the client connection stops the producer when the response reader is dropped.

Malformed request input is rejected with the normal `400`, `415`, or `422` response before the
stream starts.

There is no resume token, ETag, or byte-range contract for a stream.

If evaluation fails after headers are sent, the response terminates and the client must retry the
whole query.

`query::AsyncQueryStream` defines an executor-neutral `poll_next` boundary for long-lived query streams.

`QueryStream` and `QuerySnapshotStream` implement it with immediate in-memory polls.

`BoundedQueryStream` remains a blocking iterator because polling a standard-library `recv` call would block the host task.

The native threaded adapter supplies scheduling, bounded backpressure, waker notification, and drop cancellation.

Worker/WASI scheduling, timeout, cancellation, transport, and asynchronous-storage behavior remain host-specific.

The complete boundary is documented in [asynchronous query streaming](async-streaming.md).

## 2. Current predicate vocabulary

| Family | Operators | Current rule |
| --- | --- | --- |
| Comparison | `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte` | Compare decoded JSON values using the engine's explicit type rules |
| Membership | `$in`, `$nin` | Match a value against a list of candidate values |
| Array | `$all`, `$elemMatch`, `$size` | Match array contents, one array element's conditions, or exact array length |
| Logical | `$and`, `$or`, `$not` | Compose or invert predicate documents |
| Expression | `$expr` | Compare scalar literals, field references, or bounded numeric `$abs`/`$add`/`$subtract`/`$multiply`/`$divide`/`$mod` expressions from the same record |

An empty `$and` matches every record.

An empty `$or` matches no record.

Missing fields match `$ne` and `$nin` according to the current executor contract.

When a field contains an array, a predicate can match when an array element satisfies the predicate.

`$all` accepts an array operand; an empty operand matches no records, while a non-empty operand requires an array-valued field whose elements match every candidate value.

`$all` may contain bounded `$elemMatch` candidates, and each candidate must match one array element.

`$elemMatch` requires an array-valued field and matches when one element satisfies all scalar operators or all field conditions in the same element.

`$size` requires a non-negative integer and matches only an array with exactly that many elements.

These array predicates use a bounded table scan and are not index candidates.

These choices are tested in `src/query/tests.rs` and `src/query/malformed_tests.rs`.

`$abs` accepts exactly one numeric literal, field reference, or nested numeric expression.

`$add`, `$subtract`, `$multiply`, `$divide`, and `$mod` accept exactly two numeric literals, field references, or nested numeric expressions.

Integer results remain JSON integers when they fit.

Mixed or fractional results must be finite JSON numbers.

Exact integer division remains an integer; non-exact division produces a finite JSON number.

Division by zero is rejected.

Modulo by zero is rejected.

Missing or nonnumeric field operands make the comparison not match.

Integer overflow and non-finite results are rejected.

They are txBASE behavior and must not be described as MongoDB compatibility.

## 3. Paths, arrays, and projection

Dotted paths traverse nested JSON objects and arrays.

A numeric path segment selects an explicit array index.

A nonnumeric segment visits matching fields across array elements.

An exact field-name match takes precedence over dotted traversal.

Projection validates its shape before applying selected fields.

The current implementation does not promise every MongoDB projection rule, positional projection, `$elemMatch` projection, or aggregation expression.

## Related documents

- [Query planning and external vocabulary](query-planning.md)
- [Aggregation model](aggregation.md)
- [Join model](joins.md)
- [Mutation model](mutation-model.md)
- [Secondary-index sidecar](indexes.md)
- [HTTP method semantics](http-semantics.md)
- [Quality contract matrix](quality-matrix.md)

## Primary references

- [MongoDB documents](https://www.mongodb.com/docs/manual/core/document/)
- [MongoDB query predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/)
- [MongoDB array predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/arrays/)
- [MongoDB `$all` query predicate](https://www.mongodb.com/docs/manual/reference/operator/query/all/)
- [MongoDB `$elemMatch` query predicate](https://www.mongodb.com/docs/manual/reference/operator/query/elemmatch/)
- [MongoDB `$size` query predicate](https://www.mongodb.com/docs/manual/reference/operator/query/size/)
- [MongoDB find command](https://www.mongodb.com/docs/manual/reference/command/find/)
- [Firestore query cursors](https://firebase.google.com/docs/firestore/query-data/query-cursors)
