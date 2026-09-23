# Aggregation model

txBASE keeps aggregation as a bounded pipeline over the same JSON query document used for filtering.

The aggregation boundary is separate from ordinary cursor pagination and from the relational join boundary.

## 1. Bounded aggregation

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

The input portion accepts zero or more `$match` and `$unwind` stages, at most one input `$set` or `$addFields` stage in total, and at most one input `$project`, `$sort`, `$skip`, and `$limit` stage each before one terminal `$count`, one terminal `$distinct`, or one `$group` stage.

Input stages execute in the order listed in the pipeline.

`$unwind` accepts a top-level field reference such as `{"$unwind": "$FIELD"}` or a document with `path`, `includeArrayIndex`, and `preserveNullAndEmptyArrays` options.

The document form requires `path` and accepts an optional top-level `includeArrayIndex` field name and an optional boolean `preserveNullAndEmptyArrays` flag.

`includeArrayIndex` writes a zero-based integer for each array element and `null` for a preserved missing, null, or empty-array input.

Multiple `$unwind` stages apply sequentially in their listed order.

An input `$match` after `$unwind` filters the expanded records.

An input `$sort` uses the existing JSON sort order and stable physical-record ties before the terminal stage.

Input `$skip` and `$limit` discard or truncate records before the terminal stage.

`$set` and `$addFields` are aliases for one bounded input stage that preserves existing fields and computes named top-level fields before later stages.

Each computed field accepts a scalar literal, a field reference including a dotted path, `$literal`, `$ifNull` with exactly two operands, or the bounded numeric `$abs`, `$add`, `$subtract`, `$multiply`, `$divide`, and `$mod` expressions.

All expressions in one stage read the record as it entered that stage, so one computed field cannot depend on another field computed in the same stage.

Missing field references and missing or nonnumeric numeric results become `null`; `$ifNull` treats missing and explicit `null` as null and evaluates its fallback in that case.

An existing top-level field is overwritten, while dotted output field names and array literals outside `$literal` remain unsupported.

An input `$project` reuses the query projection rules and may appear once in the input phase, in the listed order with the other input stages.

The input form accepts only `0` and `1` inclusion or exclusion values; computed projection expressions, an empty specification, and mixed inclusion and exclusion are unsupported.

The projection is materialized before the next stage, so subsequent `$match`, `$group`, `$count`, or `$distinct` stages see only the projected fields.

For an array field, `$unwind` emits one copy of the input record for each element in input array order and replaces the field with that element.

By default, missing fields, explicit `null`, and empty arrays emit no records.

With `preserveNullAndEmptyArrays: true`, each of those inputs emits one record; an empty-array field is removed from that record, an explicit `null` remains `null`, and a missing field remains absent.

A non-null, non-array field rejects the aggregate instead of being coerced to a one-element array.

The total number of records emitted by all `$unwind` stages, including preserved records, is capped at 10,000 before `$count`, `$distinct`, or `$group` runs.

Dotted field paths, an `includeArrayIndex` name equal to `path`, and other extended `$unwind` forms remain unsupported.

Group output may have zero or more `$match` stages, followed by one optional `$project`, at most one final `$sort`, at most one `$skip`, and at most one final `$limit` stage.

`_id` is either `null` or one dotted field reference.

Supported accumulators are `$count: {}`, numeric `$sum`, `$min: "$FIELD"`, `$max: "$FIELD"`, `$first: "$FIELD"`, `$last: "$FIELD"`, `$push: "$FIELD"`, and `$addToSet: "$FIELD"`, plus numeric `$avg`, `$stdDevPop`, and `$stdDevSamp` for finite JSON numbers.

Numeric `$sum` and `$avg` operands accept a field reference, numeric literal, unary `$abs`, or binary `$add`, `$subtract`, `$multiply`, `$divide`, or `$mod` expression.

The bounded numeric expression evaluator is shared with `$expr`; missing or nonnumeric resolved values are ignored by `$sum` and `$avg`.

The filter runs before grouping.

The result is a JSON array of documents containing `_id` and the named accumulator fields.

`$project` reuses the query projection rules for group-output fields.

It must appear after `$group` and before `$sort`, `$skip`, or `$limit`.

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

The executor rejects more than 10,000 groups and rejects aggregation combined with top-level sort, projection, skip, limit, or cursor pagination.

Without `$sort`, group output order is not part of the contract, although the current implementation emits deterministic key order.

`$sort` uses the existing JSON sort ordering and stable ties.

`$limit` accepts a non-negative integer and truncates the materialized group result after sorting.

`$skip` accepts a non-negative integer and discards that many materialized group results after sorting and before `$limit`.

`$skip` must appear after `$group` and any optional `$project` or `$sort`, and before `$limit`.

`$match` stages use the same predicate rules as top-level `filter`.

Input `$match` stages must precede `$group`, `$count`, or `$distinct`.

Group-output `$match` stages must follow `$group` and precede `$project`, `$sort`, `$skip`, or `$limit`.

`$count` emits one document containing the named non-negative integer field, including zero when no records match.

`$distinct` takes one field reference and emits unique field values as a JSON array.

Missing fields and explicit `null` share one `null` value.

Both stages are terminal and cannot be combined with group-output stages.

Distinct output is capped at 10,000 values.

Additional grouping, count, or distinct stages remain unsupported.

After `$group`, stages after the group-output `$limit` remain unsupported.

`$expr`, `$sum`, and `$avg` operands are supported only in the bounded numeric `$abs`, `$add`, `$subtract`, `$multiply`, `$divide`, and `$mod` forms described in the query model.

Broader expression evaluation remains unsupported.

MongoDB documents `$group` as a blocking stage and specifies accumulator behavior such as `$count` and `$sum` in its [aggregation-stage reference](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/).

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
- [MongoDB `$project` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/project/)
- [MongoDB `$set` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/set/)
- [MongoDB `$addFields` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/addfields/)
- [MongoDB `$ifNull` expression operator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/ifnull/)
- [MongoDB `$literal` expression operator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/literal/)
- [MongoDB `$unwind` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/unwind/)
