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
    "COUNTRY": {"$in": ["JP", "TW"]}
  },
  "sort": {"AGE": 1},
  "projection": {"NAME": 1, "AGE": 1},
  "skip": 0,
  "limit": 100
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

## 2. Current predicate vocabulary

| Family | Operators | Current rule |
| --- | --- | --- |
| Comparison | `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte` | Compare decoded JSON values using the engine's explicit type rules |
| Membership | `$in`, `$nin` | Match a value against a list of candidate values |
| Logical | `$and`, `$or`, `$not` | Compose or invert predicate documents |

An empty `$and` matches every record.

An empty `$or` matches no record.

Missing fields match `$ne` and `$nin` according to the current executor contract.

When a field contains an array, a predicate can match when an array element satisfies the predicate.

These choices are tested in `src/query/tests.rs` and `src/query/malformed_tests.rs`.

They are txBASE behavior and should not be described as MongoDB compatibility.

## 3. Paths, arrays, and projection

Dotted paths traverse nested JSON objects and arrays.

A numeric path segment selects an explicit array index.

A nonnumeric segment visits matching fields across array elements.

An exact field-name match takes precedence over dotted traversal.

Projection validates its shape before applying selected fields.

The current implementation does not promise every MongoDB projection rule, positional projection, `$elemMatch` projection, or aggregation expression.

## 4. What MongoDB specifies

The [MongoDB query predicate reference](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/) groups predicates into comparison, logical, array, element, evaluation, bitwise, geospatial, and miscellaneous families.

The current txBASE subset intentionally stops at comparison, membership, and logical predicates.

MongoDB's [find command](https://www.mongodb.com/docs/manual/reference/command/find/) separates a filter from projection, sort, skip, limit, hint, and related cursor controls.

That separation is useful for txBASE because query validation, result shaping, and planner access paths can evolve independently.

MongoDB's [comparison predicate reference](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/comparison/) documents operators such as `$eq`, `$gt`, `$gte`, `$lt`, `$lte`, `$ne`, `$in`, and `$nin`.

MongoDB's [BSON comparison order](https://www.mongodb.com/docs/manual/reference/bson-type-comparison-order/) and the [`$gt` type-bracketing rules](https://www.mongodb.com/docs/manual/reference/operator/query/gt/) are important boundaries.

MongoDB compares BSON values with BSON-specific type and array rules, while txBASE compares decoded JSON values with its own explicit scalar rules.

The shared operator names therefore do not imply shared results for mixed types, missing fields, arrays, or documents.

MongoDB's [logical predicate reference](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/logical/) documents `$and`, `$or`, `$nor`, and `$not`.

MongoDB's [array predicate reference](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/arrays/) covers operators such as `$all`, `$elemMatch`, and `$size` that txBASE does not currently implement.

MongoDB also has a broad [miscellaneous predicate family](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/misc/) including regular expressions and expression evaluation.

Those operators need explicit encoding, resource, and error contracts before they belong in a file-native DBF query engine.

## 5. Indexes and selectivity

The [MongoDB query optimization guide](https://www.mongodb.com/docs/manual/core/query-optimization/) explains why predicate selectivity and index key order affect the amount of data examined.

It also warns that low-selectivity operators such as `$ne` and `$nin` often do not benefit from an index in the same way as selective equality predicates.

txBASE keeps the record scan as the query executor reference path.

The path-aware query entry point now attempts external scalar-key equality, range, or ordered traversal before applying the same filter, sort, projection, skip, and limit pipeline.

The planner reports `TableScan`, `EqualityIndex`, `RangeIndex`, `OrderedIndex`, `OrderedIndexPrefix`, or `CompoundOrderedIndex` through `explain_query_at`.

Missing, stale, malformed, and semantically unsupported sidecars fall back to `TableScan` because the sidecar is an optional acceleration structure.

Connecting an index to query execution is therefore not just a parser change.

It needs a key encoding, null and missing-field rules, duplicate ordering, update maintenance, recovery records, stale-index detection, and a planner policy.

The current planner considers direct top-level equality, single-bound-per-side range predicates, single-field ordered traversal, and compound sort requests whose fields are an index prefix.

Multiple valid single-field equality indexes may be intersected by record number before the normal filter pipeline.

The planner processes those candidate lists from the smallest exact list to the largest, which limits repeated membership checks when predicates have different cardinalities.

This is a local candidate-ordering heuristic, not MongoDB-style statistics, histograms, or a cost-based planner.

For a multi-key sort, a single-field index provides the first-key order and the remaining keys are sorted in memory within each equal first-key group.

An ascending compound index provides the complete order when the requested sort fields match its prefix.

The planner accepts the index order or its complete reverse, so all-ascending and all-descending requests can avoid an in-memory sort.

Mixed sort directions fall back to `TableScan` unless the single-field ordered-prefix path can preserve the first key and sort the ties.

MongoDB's current guidance recommends a compound index for queries that repeatedly search multiple fields.

The txBASE intersection is a local candidate-reduction feature and does not claim MongoDB planner compatibility or replace a future compound-index contract.

MongoDB's [compound-index sort-order guidance](https://www.mongodb.com/docs/manual/core/indexes/index-types/index-compound/sort-order/) and [equality-sort-range guideline](https://www.mongodb.com/docs/manual/tutorial/equality-sort-range-guideline/) show why a future compound-index planner must define index field order instead of treating every index as an interchangeable lookup table.

txBASE currently has no collection statistics or selectivity estimate, and compound definitions do not carry per-field direction metadata.

The equality intersection is a bounded candidate prefilter, not a covered query or a claim of end-to-end speedup.

The roadmap keeps index design separate from the query syntax so a query document does not imply an implementation strategy.

## 6. Mutation operators

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

## 7. Not a MongoDB wire protocol

txBASE does not implement BSON, the MongoDB wire protocol, JavaScript expressions, MongoDB collation, aggregation pipelines, MongoDB indexes, or the complete update operator set.

The JSON syntax is a deliberately small local API.

The [MongoDB atomicity and transactions guide](https://www.mongodb.com/docs/manual/core/write-operations-atomicity/) is useful for separating single-record mutation from future multi-record transaction guarantees.

No multi-record atomicity should be inferred from `$inc` or from the current HTTP `PATCH` route.

## 8. Next query work

The roadmap may later cover the following in separate contracts:

1. Collection-statistics-based index choice and mixed-direction compound planning with explicit missing, null, and collation rules.
2. A catalog for multiple tables and schema metadata.
3. Joins and aggregation with bounded memory behavior.
4. Cursors or streaming responses with stable snapshot rules.
5. Differential tests against a small reference evaluator.

Until those contracts exist, the record scan is the simpler and more honest execution model.

## Primary references

- [MongoDB documents](https://www.mongodb.com/docs/manual/core/document/)
- [MongoDB query predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/)
- [MongoDB comparison predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/comparison/)
- [MongoDB logical predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/logical/)
- [MongoDB array predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/arrays/)
- [MongoDB find command](https://www.mongodb.com/docs/manual/reference/command/find/)
- [MongoDB query optimization](https://www.mongodb.com/docs/manual/core/query-optimization/)
- [MongoDB BSON comparison order](https://www.mongodb.com/docs/manual/reference/bson-type-comparison-order/)
- [MongoDB `$gt` type bracketing](https://www.mongodb.com/docs/manual/reference/operator/query/gt/)
- [MongoDB compound-index sort order](https://www.mongodb.com/docs/manual/core/indexes/index-types/index-compound/sort-order/)
- [MongoDB equality-sort-range guideline](https://www.mongodb.com/docs/manual/tutorial/equality-sort-range-guideline/)
- [SQLite query planning](https://www.sqlite.org/queryplanner.html)
- [MongoDB update operators](https://www.mongodb.com/docs/manual/reference/mql/update/)
- [MongoDB atomicity and transactions](https://www.mongodb.com/docs/manual/core/write-operations-atomicity/)
