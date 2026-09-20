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

The pipeline may end with one bounded `$sort` stage over group output:

```json
{
  "aggregate": [
    {"$group": {"_id": "$COUNTRY", "count": {"$count": {}}}},
    {"$sort": {"count": -1, "_id": 1}}
  ]
}
```

A count-only pipeline may end after optional `$match` stages with one terminal `$count` stage:

```json
{
  "aggregate": [
    {"$match": {"ACTIVE": true}},
    {"$count": "total"}
  ]
}
```

A distinct pipeline may end after optional `$match` stages with one terminal `$distinct` stage:

```json
{
  "aggregate": [
    {"$match": {"ACTIVE": true}},
    {"$distinct": "$COUNTRY"}
  ]
}
```

An optional final `$limit` may follow `$sort`, or `$group` when sorting is omitted:

```json
{
  "aggregate": [
    {"$group": {"_id": "$COUNTRY", "count": {"$count": {}}}},
    {"$sort": {"count": -1}},
    {"$limit": 10}
  ]
}
```

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

This is a snapshot-consistency boundary, not historical MVCC.

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

Runtime-specific async traits remain a separate future boundary.

## 2. Bounded aggregation

The query document can contain one terminal `$count` or `$distinct` stage, or one blocking `$group` stage after `filter` and zero or more preceding `$match` stages:

```json
{
  "filter": {"ACTIVE": true},
  "aggregate": [
    {
      "$group": {
        "_id": "$COUNTRY",
        "count": {"$count": {}},
        "total_age": {"$sum": "$AGE"}
      }
    }
  ]
}
```

The current aggregation boundary accepts zero or more `$match` stages followed by one terminal `$count`, one terminal `$distinct`, or one `$group` stage.

Group output may have one optional `$project`, at most one final `$sort`, and at most one final `$limit` stage.

`_id` is either `null` or one dotted field reference.

Supported accumulators are `$count: {}`, `$sum: "$FIELD"`, `$min: "$FIELD"`, and `$max: "$FIELD"`, plus `$avg: "$FIELD"` for finite JSON numbers.

The filter runs before grouping.

The result is a JSON array of documents containing `_id` and the named accumulator fields.

`$project` reuses the query projection rules for group-output fields.

It must appear after `$group` and before `$sort` or `$limit`.

Inclusion and exclusion cannot be mixed.

Projection runs before the following `$sort`, so sorting a projected-away field uses the existing missing-value ordering.

A missing group field becomes `null`.

Missing and explicit `null` values therefore share a group.

Missing, `null`, and nonnumeric `$sum` inputs contribute zero.

Fractional numbers are rejected because this slice preserves integer sums exactly.

An integer sum that cannot be represented as a JSON signed or unsigned integer is rejected.

Missing, `null`, and nonnumeric `$avg` inputs are ignored.

An all-missing or all-nonnumeric group returns `null`, and a non-finite accumulated result is rejected.

Missing and `null` `$min` and `$max` inputs are ignored.

An all-missing or all-null group returns `null` for that accumulator.

Non-null `$min` and `$max` values must be comparable under the existing JSON ordering rules.

Incomparable values are rejected.

The executor rejects more than 10,000 groups and rejects aggregation combined with top-level sort, projection, skip, limit, or cursor pagination.

Without `$sort`, group output order is not part of the contract, although the current implementation emits deterministic key order.

`$sort` uses the existing JSON sort ordering and stable ties.

`$limit` accepts a non-negative integer and truncates the materialized group result after sorting.

`$match` stages use the same predicate rules as top-level `filter` and must precede `$group`, `$count`, or `$distinct`.

`$count` emits one document containing the named non-negative integer field, including zero when no records match.

`$distinct` takes one field reference and emits unique field values as a JSON array.

Missing fields and explicit `null` share one `null` value.

Both stages are terminal and cannot be combined with group-output stages.

Distinct output is capped at 10,000 values.

Stages after `$limit`, additional grouping, count, or distinct stages, and expression operands remain unsupported.

MongoDB documents `$group` as a blocking stage and specifies accumulator behavior such as `$count` and `$sum` in its [aggregation-stage reference](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/).

Its separate [`$count` stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/count/) is represented by this bounded txBASE stage without claiming full MongoDB pipeline compatibility.

## 3. Bounded local join

The library exposes one bounded local join through `txbase::query::join`.

It accepts the relation shape from the roadmap and supports one or more equality conditions between two catalog tables:

```json
{
  "from": "users",
  "join": {
    "type": "left",
    "table": "posts",
    "on": {
      "users.ID": {
        "$eq": {
          "$field": "posts.USER_ID"
        }
      }
    }
  },
  "filter": {
    "users.ACTIVE": true
  },
  "projection": {
    "users.NAME": 1,
    "posts.TITLE": 1
  }
}
```

`join.parse` validates this JSON, and `join::execute` loads named tables from a `Catalog`.

The current join types are `inner`, `left`, `right`, `semi`, `anti`, and `cross`.

The result is a flat JSON object whose keys use the `table.field` form.

An unmatched left row is retained without right-table fields.

An unmatched right row is retained without left-table fields for a `right` join.

`semi` emits one left row when at least one right row matches.

`anti` emits one left row when no right row matches.

Neither type emits right-table fields.

`cross` accepts an empty `on` object and emits every left and right pair.

Its candidate pair count is capped at 100,000 before filtering.

Missing and explicit `null` join keys do not match.

The equality-join planner selects `NestedLoop` when the left and right active-row counts have at most 64 candidate pairs.

For a direct single-key equality join or a chained single-key equality stage with a fresh single-field index on the probed table, it selects `IndexNestedLoop` for larger inputs and looks up each outer key in that index.

For a chained `right` stage, it probes the indexed right table and restores right-major physical order before emitting the intermediate result.

Direct and chained stages may also use a fresh compound index when the equality fields exactly match the index field order.

When that index is unavailable, stale, malformed, or not applicable, larger inputs select `Hash` and build one in-memory equality map for the right table, or the left table for a `right` join.

The selector preserves left-major output order for non-right joins and right-major output order for right joins.

It rejects a result larger than 100,000 rows.

For multiple joins, the required `join` object is the first stage and an optional `joins` array adds stages from left to right.

Each additional stage may reference any table already present in the intermediate row and adds one new table.

Table names cannot repeat, and the total stage count is capped at eight.

Every intermediate result is capped at 100,000 rows.

This is a bounded cardinality and index-availability strategy selector, not a full cost-based planner, full index-aware join planner, or streaming executor.

The single-table HTTP server does not expose joins.

The catalog server exposes the same read-only boundary at `QUERY /join`.

Cross-table writes and transactions remain outside this surface.

The join accepts the existing filter and projection rules, but not sort, pagination, aggregation, self-join aliases, or cross-table transactions.

MongoDB's [`$lookup` stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/lookup/) is reference vocabulary.

MongoDB describes `$lookup` as a left outer join that adds matching foreign documents as an array, while txBASE emits flat relational rows to match the attached join plan.

The MongoDB documentation also calls out the performance cost of an unindexed foreign-side join.

That is why this first slice has a hard result bound and makes no planner-level performance claim.

## 4. Current predicate vocabulary

| Family | Operators | Current rule |
| --- | --- | --- |
| Comparison | `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte` | Compare decoded JSON values using the engine's explicit type rules |
| Membership | `$in`, `$nin` | Match a value against a list of candidate values |
| Logical | `$and`, `$or`, `$not` | Compose or invert predicate documents |
| Expression | `$expr` | Compare two scalar literals or field references from the same record |

An empty `$and` matches every record.

An empty `$or` matches no record.

Missing fields match `$ne` and `$nin` according to the current executor contract.

When a field contains an array, a predicate can match when an array element satisfies the predicate.

These choices are tested in `src/query/tests.rs` and `src/query/malformed_tests.rs`.

They are txBASE behavior and must not be described as MongoDB compatibility.

## 5. Paths, arrays, and projection

Dotted paths traverse nested JSON objects and arrays.

A numeric path segment selects an explicit array index.

A nonnumeric segment visits matching fields across array elements.

An exact field-name match takes precedence over dotted traversal.

Projection validates its shape before applying selected fields.

The current implementation does not promise every MongoDB projection rule, positional projection, `$elemMatch` projection, or aggregation expression.

## Related documents

- [Query planning and external vocabulary](query-planning.md)
- [Mutation model](mutation-model.md)
- [Secondary-index sidecar](indexes.md)
- [HTTP method semantics](http-semantics.md)
- [Quality contract matrix](quality-matrix.md)

## Primary references

- [MongoDB documents](https://www.mongodb.com/docs/manual/core/document/)
- [MongoDB query predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/)
- [MongoDB find command](https://www.mongodb.com/docs/manual/reference/command/find/)
- [Firestore query cursors](https://firebase.google.com/docs/firestore/query-data/query-cursors)
- [MongoDB `$group` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/)
- [MongoDB `$lookup` join stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/lookup/)
