# Query model

txBASE uses a small JSON query document.

MongoDB is a source of vocabulary and comparison points, not a compatibility target.

The query engine operates on active DBF records after they have been decoded to JSON.

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
  "skip": 0,
  "limit": 100,
  "page_size": 25,
  "cursor": "42"
}
```

The pipeline may also end with one bounded `$sort` stage over the group output:

```json
{
  "aggregate": [
    {"$group": {"_id": "$COUNTRY", "count": {"$count": {}}}},
    {"$sort": {"count": -1, "_id": 1}}
  ]
}
```

An optional final `$limit` may follow `$sort` (or `$group` when sorting is omitted):

```json
{
  "aggregate": [
    {"$group": {"_id": "$COUNTRY", "count": {"$count": {}}}},
    {"$sort": {"count": -1}},
    {"$limit": 10}
  ]
}
```

The top-level keys are validated.

Unknown keys are rejected rather than ignored.

`filter` defaults to a match-all filter.

`sort` uses `1` for ascending and `-1` for descending order.

`projection` uses inclusion or exclusion values of `1` and `0`.

`skip` and `limit` are non-negative integer controls.

The executor applies filtering, sorting, projection, skipping, and limiting in that order.

Sort ties preserve DBF record order.

`page_size` enables the bounded cursor response shape:

```json
{
  "records": [{"NAME": "Alice"}],
  "cursor": "42"
}
```

The cursor is the one-based physical DBF record number after the returned page.
The next request sends that token with the same `page_size` and resumes after that record.
When `sort` is present, the cursor is a JSON string containing versioned sort keys and the
last physical record number used as a deterministic tie-breaker.
The next request must repeat the same sort fields and directions.
Sorted cursors are keyset boundaries, not offsets; `skip` cannot be combined with either cursor
mode, and `page_size` is capped at 1,000.
Neither cursor token is a snapshot identifier; callers must repeat the same query semantics and
keep the underlying table snapshot stable while paging.

Physical cursor pages scan active records in physical order and stop after one extra matching
record proves that another page exists.
They deliberately bypass index candidate ordering, and `explain_query_at` reports a table scan
for this mode.
This keeps the page result bounded.

Sorted cursor pages use the existing sort comparison and materialize the matching record
references before selecting the page.
They provide a resumable result boundary but do not claim streaming or backpressure behavior.

The library also exposes `query::stream_query` for a borrowed, pull-based iterator over active
records.
It applies `filter`, `projection`, `skip`, and `limit` as records are consumed, so it does not
materialize the matching record set.
It rejects `sort`, `aggregate`, `page_size`, and `cursor` because those controls require a
blocking or resumable result boundary.
The iterator does not provide a long-lived snapshot or an asynchronous backpressure protocol.
`query::stream_query_snapshot` is the stable-snapshot variant: it clones the loaded table before
iteration, so later mutations of the source table do not change its records.
It remains pull-based and does not provide an asynchronous backpressure protocol.

## 2. Bounded aggregation

The query document can contain one blocking `$group` stage after `filter` and zero or more
preceding `$match` stages:

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

The current aggregation boundary accepts one `$group` stage, zero or more `$match` stages before it,
at most one final `$sort` stage, and at most one final `$limit` stage.
`_id` is either `null` or one dotted field reference.
The supported accumulators are `$count: {}`, `$sum: "$FIELD"`, `$min: "$FIELD"`, and
`$max: "$FIELD"`, plus `$avg: "$FIELD"` for finite JSON numbers.
The filter runs before grouping, and the result is a JSON array of documents containing `_id`
and the named accumulator fields.

A missing group field becomes `null`, so missing and explicit `null` values share a group.
Missing, `null`, and nonnumeric `$sum` inputs contribute zero.
Fractional numbers are rejected because this slice preserves integer sums exactly.
An integer sum that cannot be represented as a JSON signed or unsigned integer is rejected.
Missing, `null`, and nonnumeric `$avg` inputs are ignored; an all-missing or all-nonnumeric group
returns `null`, and a non-finite accumulated result is rejected.
Missing and `null` `$min` / `$max` inputs are ignored; an all-missing or all-null group returns
`null` for that accumulator.
Non-null `$min` / `$max` values must be comparable under the existing JSON ordering rules.
Incomparable values are rejected.
The executor rejects more than 10,000 groups and rejects combining aggregation with top-level sort,
projection, skip, limit, or cursor pagination.
Without `$sort`, group output order is not part of the contract, although the current implementation
emits a deterministic key order. `$sort` uses the existing JSON sort ordering and stable ties.
`$limit` accepts a non-negative integer and truncates the materialized group result after sorting.
`$match` stages use the same predicate rules as the top-level `filter` and must precede `$group`.
Stages after `$limit`, additional grouping stages, and expression operands remain unsupported.

MongoDB documents `$group` as a blocking stage and specifies accumulator behavior such as
`$count` and `$sum` in its [aggregation-stage reference](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/).
Its separate [`$count` stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/count/)
is not accepted by this first txBASE slice.

## 3. Bounded local join

The library exposes one bounded local join through `txbase::query::join`.
It accepts the relation shape from the roadmap and supports one or more equality conditions
between two catalog tables:

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

`join.parse` validates this JSON and `join::execute` loads the named tables from a `Catalog`.
The current join types are `inner`, `left`, `semi`, `anti`, and `cross`.
The result is a flat JSON object whose keys are qualified as `table.field`.
An unmatched left row is retained without right-table fields.
`semi` emits one left row when at least one right row matches, while `anti` emits one left row
when no right row matches; neither type emits right-table fields.
`cross` accepts an empty `on` object and emits every left/right pair.
Its candidate pair count is capped at 100,000 before filtering.
Missing and explicit `null` join keys do not match.

The implementation builds one in-memory equality map for the right table and rejects a result
larger than 100,000 rows.
This is a bounded nested execution boundary, not a cost-based planner or a streaming executor.
The HTTP server does not expose cross-table joins yet.
The join accepts the existing filter and projection rules, but not sort, pagination, aggregation,
multiple joins, self-join aliases, or cross-table transactions.

MongoDB's [`$lookup` stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/lookup/)
is the reference vocabulary.
MongoDB describes `$lookup` as a left outer join that adds matching foreign documents as an array,
while txBASE currently emits flat relational rows to match the attached join plan.
The MongoDB documentation also calls out the performance cost of an unindexed foreign-side join,
which is why this first slice has a hard result bound and no claim of planner-level performance.

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

They are txBASE behavior and should not be described as MongoDB compatibility.

## 4. Paths, arrays, and projection

Dotted paths traverse nested JSON objects and arrays.

A numeric path segment selects an explicit array index.

A nonnumeric segment visits matching fields across array elements.

An exact field-name match takes precedence over dotted traversal.

Projection validates its shape before applying selected fields.

The current implementation does not promise every MongoDB projection rule, positional projection, `$elemMatch` projection, or aggregation expression.

## 5. What MongoDB specifies

The [MongoDB query predicate reference](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/) groups predicates into comparison, logical, array, element, evaluation, bitwise, geospatial, and miscellaneous families.

The current txBASE subset intentionally stops at comparison, membership, logical, and bounded field-expression predicates.

MongoDB's [find command](https://www.mongodb.com/docs/manual/reference/command/find/) separates a filter from projection, sort, skip, limit, hint, and related cursor controls.

That separation is useful for txBASE because query validation, result shaping, and planner access paths can evolve independently.

MongoDB's find command returns an initial batch and a cursor identifier, while Firestore's
[query cursor guidance](https://firebase.google.com/docs/firestore/query-data/query-cursors)
uses the last document in one batch as the start point for the next batch.
txBASE adopts that boundary idea for physical and sorted pages;
it does not claim server-side cursor lifetime or snapshot isolation.

MongoDB's [comparison predicate reference](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/comparison/) documents operators such as `$eq`, `$gt`, `$gte`, `$lt`, `$lte`, `$ne`, `$in`, and `$nin`.

MongoDB's [BSON comparison order](https://www.mongodb.com/docs/manual/reference/bson-type-comparison-order/) and the [`$gt` type-bracketing rules](https://www.mongodb.com/docs/manual/reference/operator/query/gt/) are important boundaries.

MongoDB compares BSON values with BSON-specific type and array rules, while txBASE compares decoded JSON values with its own explicit scalar rules.

The shared operator names therefore do not imply shared results for mixed types, missing fields, arrays, or documents.

MongoDB's [logical predicate reference](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/logical/) documents `$and`, `$or`, `$nor`, and `$not`.

MongoDB's [`$expr` predicate](https://www.mongodb.com/docs/manual/reference/operator/query/expr/) allows expressions inside a query predicate, including comparisons between two fields from the same document.

txBASE implements only the bounded form `{"$expr":{"$gt":["$LEFT","$RIGHT"]}}` with one comparison operator and two scalar operands.

A string operand beginning with `$` is a dotted field reference; all other scalar operands are literals.

If either field reference is missing, the expression does not match.

The expression path uses the table scan because a field-to-field comparison is not a constant-bound index lookup.

MongoDB's [array predicate reference](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/arrays/) covers operators such as `$all`, `$elemMatch`, and `$size` that txBASE does not currently implement.

MongoDB also has a broad [miscellaneous predicate family](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/misc/) including regular expressions and expression evaluation.

Those operators need explicit encoding, resource, and error contracts before they belong in a file-native DBF query engine.

## 6. Indexes and selectivity

The [MongoDB query optimization guide](https://www.mongodb.com/docs/manual/core/query-optimization/) explains why predicate selectivity and index key order affect the amount of data examined.

It also warns that low-selectivity operators such as `$ne` and `$nin` often do not benefit from an index in the same way as selective equality predicates.

txBASE keeps the record scan as the query executor reference path.

The path-aware query entry point now attempts external scalar-key equality, range, or ordered traversal before applying the same filter, sort, projection, skip, and limit pipeline.

The planner reports `TableScan`, `EqualityIndex`, `RangeIndex`, `OrderedIndex`, `OrderedIndexPrefix`, or `CompoundOrderedIndex` through `explain_query_at`.

Missing, stale, malformed, and semantically unsupported sidecars fall back to `TableScan` because the sidecar is an optional acceleration structure.

Connecting an index to query execution is therefore not just a parser change.

It needs a key encoding, null and missing-field rules, duplicate ordering, update maintenance, recovery records, stale-index detection, and a planner policy.

The current planner considers direct top-level equality, single-bound-per-side range predicates, single-field ordered traversal, and compound sort requests whose fields match an index suffix after an exact equality prefix.

Multiple valid single-field equality indexes may be intersected by record number before the normal filter pipeline.

The planner uses the sidecar's active-record count and each single-field index's distinct-key count to estimate equality cardinality before loading candidate lists.

It processes the exact candidate lists in estimated-selectivity order, which limits repeated membership checks when predicates have different expected cardinalities.

The equality estimate assumes a uniform distribution.

Single-field range indexes use a persisted equi-depth histogram and sum the record counts of overlapping buckets.

These are local selectivity estimates, not a full cost model or MongoDB planner compatibility.

For a multi-key sort, a single-field index provides the first-key order and the remaining keys are sorted in memory within each equal first-key group.

An ascending or mixed-direction compound index provides the complete order when the requested sort fields match its indexed fields after any exact equality prefix.

The planner accepts the index order or its complete reverse, so compatible mixed-direction requests can avoid an in-memory sort as well.

The planner compares exact candidate counts for compatible compound definitions, then prefers a shorter definition and a stable name tie-breaker.

This local candidate-count choice is not a full cost model because it does not estimate index I/O, memory, cache state, collation, or range selectivity for compound keys.

MongoDB's current guidance recommends a compound index for queries that repeatedly search multiple fields.

The txBASE intersection is a local candidate-reduction feature and does not claim MongoDB planner compatibility or replace a future compound-index contract.

MongoDB's [compound-index sort-order guidance](https://www.mongodb.com/docs/manual/core/indexes/index-types/index-compound/sort-order/) and [equality-sort-range guideline](https://www.mongodb.com/docs/manual/tutorial/equality-sort-range-guideline/) show why a future compound-index planner must define index field order instead of treating every index as an interchangeable lookup table.

txBASE currently has an active-record count, a uniform distinct-key estimate for equality, a single-field range histogram, and per-field direction metadata for compound definitions, but no full cost model.

The equality intersection is a bounded candidate prefilter, not a covered query or a claim of end-to-end speedup.

The roadmap keeps index design separate from the query syntax so a query document does not imply an implementation strategy.

## 7. Mutation operators

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

MongoDB has a much larger [update operator reference](https://www.mongodb.com/docs/manual/reference/mql/update/) including array and arithmetic operators.

txBASE rejects unsupported operators instead of silently treating them as field names.

## 8. Not a MongoDB wire protocol

txBASE does not implement BSON, the MongoDB wire protocol, JavaScript expressions, MongoDB collation, aggregation pipelines, MongoDB indexes, or the complete update operator set.

The JSON syntax is a deliberately small local API.

The [MongoDB atomicity and transactions guide](https://www.mongodb.com/docs/manual/core/write-operations-atomicity/) is useful for separating single-record mutation from future multi-record transaction guarantees.

No multi-record atomicity should be inferred from `$inc` or from the current HTTP `PATCH` route.

## 9. Next query work

The roadmap may later cover the following in separate contracts:

1. Full expression evaluation and cost-based index choice with explicit missing, null, collation, and compound-range rules.
2. A catalog for multiple tables and schema metadata.
3. Additional aggregation stages and joins with bounded memory behavior.
4. Backpressure and stable snapshot rules for long-lived streams.
5. Differential tests against a small reference evaluator.

Until those contracts exist, the record scan is the simpler and more honest execution model.

## Primary references

- [MongoDB documents](https://www.mongodb.com/docs/manual/core/document/)
- [MongoDB query predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/)
- [MongoDB comparison predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/comparison/)
- [MongoDB logical predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/logical/)
- [MongoDB `$expr` predicate](https://www.mongodb.com/docs/manual/reference/operator/query/expr/)
- [MongoDB array predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/arrays/)
- [MongoDB find command](https://www.mongodb.com/docs/manual/reference/command/find/)
- [Firestore query cursors](https://firebase.google.com/docs/firestore/query-data/query-cursors)
- [MongoDB query optimization](https://www.mongodb.com/docs/manual/core/query-optimization/)
- [MongoDB BSON comparison order](https://www.mongodb.com/docs/manual/reference/bson-type-comparison-order/)
- [MongoDB `$gt` type bracketing](https://www.mongodb.com/docs/manual/reference/operator/query/gt/)
- [MongoDB compound-index sort order](https://www.mongodb.com/docs/manual/core/indexes/index-types/index-compound/sort-order/)
- [MongoDB equality-sort-range guideline](https://www.mongodb.com/docs/manual/tutorial/equality-sort-range-guideline/)
- [SQLite query planning](https://www.sqlite.org/queryplanner.html)
- [MongoDB update operators](https://www.mongodb.com/docs/manual/reference/mql/update/)
- [MongoDB atomicity and transactions](https://www.mongodb.com/docs/manual/core/write-operations-atomicity/)
