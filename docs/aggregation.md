# Aggregation model

txBASE keeps aggregation as a bounded pipeline over the same JSON query document used for filtering.

The aggregation boundary is separate from ordinary cursor pagination and from the relational join boundary.

## 1. Bounded aggregation

The query document can contain one terminal `$count` or `$distinct` stage, one blocking `$group` stage, one blocking `$bucket` stage, one blocking `$bucketAuto` stage, or one `$sortByCount` stage after `filter` and zero or more preceding `$match` stages:

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

The input portion accepts zero or more `$match` and `$unwind` stages, at most one input `$set` or `$addFields` stage in total, and at most one input `$project`, `$sort`, `$skip`, and `$limit` stage each before one terminal `$count`, one terminal `$distinct`, one `$group` stage, one `$bucket` stage, one `$bucketAuto` stage, or one `$sortByCount` stage.

Input stages execute in the order listed in the pipeline.

`$unwind` accepts a top-level field reference such as `{"$unwind": "$FIELD"}` or a document with `path`, `includeArrayIndex`, and `preserveNullAndEmptyArrays` options.

The document form requires `path` and accepts an optional top-level `includeArrayIndex` field name and an optional boolean `preserveNullAndEmptyArrays` flag.

`includeArrayIndex` writes a zero-based integer for each array element and `null` for a preserved missing, null, or empty-array input.

Multiple `$unwind` stages apply sequentially in their listed order.

An input `$match` after `$unwind` filters the expanded records.

An input `$sort` uses the existing JSON sort order and stable physical-record ties before the terminal stage.

Input `$skip` and `$limit` discard or truncate records before the terminal stage.

`$set` and `$addFields` are aliases for one bounded input stage that preserves existing fields and computes named top-level fields before later stages.

Each computed field accepts a scalar literal, a field reference including a dotted path, `$literal`, `$ifNull` with exactly two operands, `$concat` with at least two string expressions, `$toLower`, `$toUpper`, or the bounded numeric `$abs`, `$add`, `$subtract`, `$multiply`, `$divide`, and `$mod` expressions.

All expressions in one stage read the record as it entered that stage, so one computed field cannot depend on another field computed in the same stage.

Missing field references, missing or nonnumeric numeric results, and missing, null, or non-string string results become `null`; `$ifNull` treats missing and explicit `null` as null and evaluates its fallback in that case.

`$concat` preserves operand order and returns `null` when any operand is missing, `null`, or not a string. `$toLower` and `$toUpper` use locale-independent Unicode case conversion.

The shared scalar-expression evaluator is used by `$set`/`$addFields` and `$expr`. Computed string results are capped at 1 MiB.

An existing top-level field is overwritten, while dotted output field names and array literals outside `$literal` remain unsupported.

An input `$project` reuses the query projection rules and may appear once in the input phase, in the listed order with the other input stages.

The input form accepts only `0` and `1` inclusion or exclusion values; computed projection expressions, an empty specification, and mixed inclusion and exclusion are unsupported.

The projection is materialized before the next stage, so subsequent `$match`, `$group`, `$bucket`, `$bucketAuto`, `$count`, or `$distinct` stages see only the projected fields.

For an array field, `$unwind` emits one copy of the input record for each element in input array order and replaces the field with that element.

By default, missing fields, explicit `null`, and empty arrays emit no records.

With `preserveNullAndEmptyArrays: true`, each of those inputs emits one record; an empty-array field is removed from that record, an explicit `null` remains `null`, and a missing field remains absent.

A non-null, non-array field rejects the aggregate instead of being coerced to a one-element array.

The total number of records emitted by all `$unwind` stages, including preserved records, is capped at 10,000 before `$count`, `$distinct`, `$group`, `$bucket`, or `$bucketAuto` runs.

Dotted field paths, an `includeArrayIndex` name equal to `path`, and other extended `$unwind` forms remain unsupported.

`$bucket` groups records into numeric ranges using `groupBy`, `boundaries`, and an optional `default` value:

```json
{
  "$bucket": {
    "groupBy": "$AGE",
    "boundaries": [0, 20, 40],
    "default": "other",
    "output": {"count": {"$count": {}}}
  }
}
```

`groupBy` accepts the shared bounded scalar-expression subset, and its result must be a finite JSON number for range assignment.

`boundaries` must contain at least two finite JSON numbers in strictly ascending order.

Each range includes its lower boundary and excludes its upper boundary.

A missing, null, nonnumeric, or out-of-range `groupBy` result uses `default` when it is present; otherwise the aggregate is rejected.

Each non-empty range produces one document whose `_id` is the range lower boundary, and a populated default bucket uses the default value as `_id`.

Empty buckets are omitted, and the default bucket is emitted after the range buckets.

`$bucketAuto` derives numeric ranges from the input values and attempts to distribute distinct values evenly across a requested number of buckets:

```json
{
  "$bucketAuto": {
    "groupBy": "$AGE",
    "buckets": 2
  }
}
```

`groupBy` accepts the shared bounded scalar-expression subset, and its result must be a finite JSON number. `buckets` must be a positive integer no greater than 10,000.

The expression is evaluated once per input record, after input stages and before range derivation.

The implementation sorts finite numeric values, partitions their distinct values into at most the requested number of non-empty buckets, and preserves the original input order for bucket accumulators.

Missing, null, and nonnumeric `groupBy` results reject the aggregate because `$bucketAuto` has no default bucket in this bounded contract.

Each output `_id` is an object with `min` and `max` numeric bounds; the upper bound is exclusive except for the final bucket, whose upper bound is inclusive.

Fewer buckets are emitted when the input contains fewer distinct numeric values than requested.

When `output` is omitted, `$bucketAuto` emits a `count` accumulator.

When `output` is present, it uses the same bounded accumulator forms as `$group` and `$bucket`.

The stage materializes at most 10,000 numeric input values, does not spill to disk, and rejects the MongoDB `granularity` option until a txBASE-owned boundary-series contract exists.

`$sortByCount` groups records by one bounded scalar expression and emits the grouping value as `_id` with a `count`, ordered by `count` descending:

```json
{
  "$sortByCount": "$COUNTRY"
}
```

The expression can be a field reference, scalar literal, `$literal`, `$ifNull`, `$concat`, `$toLower`, `$toUpper`, or the bounded numeric expression subset used by `$expr` and `$set`.

It is evaluated for each input record.
Missing or null expression results form the `null` group.

For example, this groups case variants together:

```json
{
  "$sortByCount": {"$toUpper": "$COUNTRY"}
}
```

Unsupported expression operators remain rejected.

`$literal` follows the shared expression semantics and can carry a JSON value without interpreting it as an operator.

Missing and explicit `null` field values share one group.

Empty groups are not emitted, the stage is capped at 10,000 groups, and it does not spill to disk.

The built-in descending count order is applied before later group-output stages.
A later `$sort` may replace that order.

When `output` is omitted, `$bucket` emits a `count` accumulator.

When `output` is present, it uses the same bounded accumulator forms as `$group` except that `_id` is assigned by the bucket stage.

The stage supports at most 10,000 ranges, does not spill to disk, and shares the 10,000-value materialization bound with `$push` and `$addToSet`.

Group, bucket, bucket-auto, or `$sortByCount` output may have zero or more `$match` stages, followed by one optional `$project`, at most one final `$sort`, at most one `$skip`, and at most one final `$limit` stage.

`_id` is either `null` or one expression from the shared bounded scalar-expression subset.

The expression is evaluated once per input record; a missing or null result is grouped as `null`.

Supported accumulators are `$count: {}`, numeric `$sum`, `$min: "$FIELD"`, `$max: "$FIELD"`, `$first: "$FIELD"`, `$last: "$FIELD"`, `$push: "$FIELD"`, and `$addToSet: "$FIELD"`, plus numeric `$avg`, `$stdDevPop`, and `$stdDevSamp` for finite JSON numbers.

Numeric `$sum` and `$avg` operands accept a field reference, numeric literal, unary `$abs`, or binary `$add`, `$subtract`, `$multiply`, `$divide`, or `$mod` expression.

The bounded numeric expression evaluator is shared with `$expr`; missing or nonnumeric resolved values are ignored by `$sum` and `$avg`.

The filter runs before grouping or bucketing.

The result is a JSON array of documents containing `_id` and the named accumulator fields for `$group`, `$bucket`, or `$bucketAuto`, or `_id` and `count` for `$sortByCount`.

`$project` reuses the query projection rules for group-output fields.

It must appear after `$group`, `$bucket`, `$bucketAuto`, or `$sortByCount` and before `$sort`, `$skip`, or `$limit`.

Inclusion and exclusion cannot be mixed.

Projection runs before the following `$sort`, so sorting a projected-away field uses the existing missing-value ordering.

A missing group field becomes `null`.

Missing and explicit `null` values therefore share a group.

Missing, `null`, and nonnumeric `$sum` inputs contribute zero.

A numeric `$sum` literal contributes once for each input record.

All-integral `$sum` inputs preserve an integer JSON result.

If any fractional input occurs, `$sum` returns a finite JSON floating-point result.

An accumulated `$sum` that is not finite or cannot be represented as JSON is rejected.

Missing, `null`, and nonnumeric `$avg` inputs are ignored.

An all-missing or all-nonnumeric group returns `null`, and a non-finite accumulated result is rejected.

`$stdDevPop` returns the population standard deviation, and `$stdDevSamp` returns the sample standard deviation.

Both standard-deviation accumulators accept the same bounded numeric expressions as `$sum` and `$avg`.

Missing, `null`, and nonnumeric standard-deviation inputs are ignored.

An all-missing or all-nonnumeric group returns `null` for either accumulator.

`$stdDevPop` returns `0` for one numeric input, while `$stdDevSamp` returns `null` until two numeric inputs exist.

Both accumulators use constant memory per group, return finite JSON floating-point values, and reject a non-finite intermediate or result.

Missing and `null` `$min` and `$max` inputs are ignored.

An all-missing or all-null group returns `null` for that accumulator.

Non-null `$min` and `$max` values must be comparable under the existing JSON ordering rules.

Incomparable values are rejected.

`$first` and `$last` use input physical record order within each group.

They return the first or last field value, including explicit `null`; a missing field is returned as `null`.

`$push` returns every field value in input physical record order; `$addToSet` returns each JSON value once in its first-seen order.

Missing fields are appended as `null` by both accumulators.

The combined materialized value count for all `$push` and `$addToSet` accumulators is capped at 10,000.

The executor rejects more than 10,000 groups, `$sortByCount` groups, bucket ranges, or `$bucketAuto` input values and rejects aggregation combined with top-level sort, projection, skip, limit, or cursor pagination.

Without `$sort`, group, bucket, or bucket-auto output order is not part of the contract, although the current implementation emits deterministic key or boundary order.

`$sortByCount` output is ordered by descending `count` before any later group-output stage.

`$sort` uses the existing JSON sort ordering and stable ties.

`$limit` accepts a non-negative integer and truncates the materialized group result after sorting.

`$skip` accepts a non-negative integer and discards that many materialized group results after sorting and before `$limit`.

`$skip` must appear after `$group`, `$bucket`, `$bucketAuto`, or `$sortByCount` and any optional `$project` or `$sort`, and before `$limit`.

`$match` stages use the same predicate rules as top-level `filter`.

Input `$match` stages must precede `$group`, `$bucket`, `$bucketAuto`, `$sortByCount`, `$count`, or `$distinct`.

Group-output `$match` stages must follow `$group`, `$bucket`, `$bucketAuto`, or `$sortByCount` and precede `$project`, `$sort`, `$skip`, or `$limit`.

`$count` emits one document containing the named non-negative integer field, including zero when no records match.

`$distinct` takes one field reference and emits unique field values as a JSON array.

Missing fields and explicit `null` share one `null` value.

Both stages are terminal and cannot be combined with group-output stages.

Distinct output is capped at 10,000 values.

Additional grouping, bucket, bucket-auto, sort-by-count, count, or distinct stages remain unsupported.

After `$group`, `$bucket`, `$bucketAuto`, or `$sortByCount`, stages after the group-output `$limit` remain unsupported.

`$sum`, `$avg`, `$stdDevPop`, and `$stdDevSamp` operands remain numeric-only. `$expr` also accepts the shared scalar `$literal`, `$ifNull`, `$concat`, `$toLower`, and `$toUpper` forms described in the query model.

Broader expression evaluation remains unsupported.

MongoDB documents `$group` as a blocking stage and specifies accumulator behavior such as `$count` and `$sum` in its [aggregation-stage reference](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/).

MongoDB documents `$sortByCount` as a grouping stage that is equivalent to `$group` followed by a descending `$sort` on `count`; txBASE keeps that behavior while limiting the group expression to the shared bounded scalar-expression subset.

MongoDB documents `$bucketAuto` as a stage that derives boundaries to distribute input documents across a requested number of buckets; txBASE implements the shared scalar-expression subset whose result is numeric, with explicit value and granularity limits.

Its separate [`$count` stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/count/) is represented by this bounded txBASE stage without claiming full MongoDB pipeline compatibility.

## Related documents

- [Query model](query-model.md)
- [Query planning and external vocabulary](query-planning.md)
- [Quality contract matrix](quality-matrix.md)

## Primary references

- [MongoDB `$group` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/)
- [MongoDB `$sum` accumulator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/sum/)
- [MongoDB `$avg` accumulator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/avg/)
- [MongoDB `$stdDevPop` accumulator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/stddevpop/)
- [MongoDB `$stdDevSamp` accumulator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/stddevsamp/)
- [MongoDB `$count` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/count/)
- [MongoDB `$bucket` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/bucket/)
- [MongoDB `$bucketAuto` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/bucketAuto/)
- [MongoDB `$sortByCount` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/sortByCount/)
- [MongoDB `$project` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/project/)
- [MongoDB `$set` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/set/)
- [MongoDB `$addFields` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/addfields/)
- [MongoDB `$ifNull` expression operator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/ifnull/)
- [MongoDB `$literal` expression operator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/literal/)
- [MongoDB `$concat` expression operator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/concat/)
- [MongoDB `$toLower` expression operator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/tolower/)
- [MongoDB `$toUpper` expression operator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/toupper/)
- [MongoDB `$unwind` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/unwind/)
