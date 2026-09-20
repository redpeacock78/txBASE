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

The current aggregation boundary accepts zero or more `$match` stages followed by one terminal `$count`, one terminal `$distinct`, or one `$group` stage.

Group output may have zero or more `$match` stages, followed by one optional `$project`, at most one final `$sort`, and at most one final `$limit` stage.

`_id` is either `null` or one dotted field reference.

Supported accumulators are `$count: {}`, numeric `$sum: "$FIELD"`, `$min: "$FIELD"`, `$max: "$FIELD"`, `$first: "$FIELD"`, and `$last: "$FIELD"`, plus `$avg: "$FIELD"` for finite JSON numbers.

The filter runs before grouping.

The result is a JSON array of documents containing `_id` and the named accumulator fields.

`$project` reuses the query projection rules for group-output fields.

It must appear after `$group` and before `$sort` or `$limit`.

Inclusion and exclusion cannot be mixed.

Projection runs before the following `$sort`, so sorting a projected-away field uses the existing missing-value ordering.

A missing group field becomes `null`.

Missing and explicit `null` values therefore share a group.

Missing, `null`, and nonnumeric `$sum` inputs contribute zero.

All-integral `$sum` inputs preserve an integer JSON result.

If any fractional input occurs, `$sum` returns a finite JSON floating-point result.

An accumulated `$sum` that is not finite or cannot be represented as JSON is rejected.

Missing, `null`, and nonnumeric `$avg` inputs are ignored.

An all-missing or all-nonnumeric group returns `null`, and a non-finite accumulated result is rejected.

Missing and `null` `$min` and `$max` inputs are ignored.

An all-missing or all-null group returns `null` for that accumulator.

Non-null `$min` and `$max` values must be comparable under the existing JSON ordering rules.

Incomparable values are rejected.

`$first` and `$last` use input physical record order within each group.

They return the first or last field value, including explicit `null`; a missing field is returned as `null`.

The executor rejects more than 10,000 groups and rejects aggregation combined with top-level sort, projection, skip, limit, or cursor pagination.

Without `$sort`, group output order is not part of the contract, although the current implementation emits deterministic key order.

`$sort` uses the existing JSON sort ordering and stable ties.

`$limit` accepts a non-negative integer and truncates the materialized group result after sorting.

`$match` stages use the same predicate rules as top-level `filter`.

Input `$match` stages must precede `$group`, `$count`, or `$distinct`.

Group-output `$match` stages must follow `$group` and precede `$project`, `$sort`, or `$limit`.

`$count` emits one document containing the named non-negative integer field, including zero when no records match.

`$distinct` takes one field reference and emits unique field values as a JSON array.

Missing fields and explicit `null` share one `null` value.

Both stages are terminal and cannot be combined with group-output stages.

Distinct output is capped at 10,000 values.

Stages after `$limit`, additional grouping, count, or distinct stages remain unsupported.

`$expr` operands are supported only in the bounded numeric `$abs`, `$add`, `$subtract`, `$multiply`, `$divide`, and `$mod` forms described in the query model.

Broader expression evaluation remains unsupported.

MongoDB documents `$group` as a blocking stage and specifies accumulator behavior such as `$count` and `$sum` in its [aggregation-stage reference](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/).

Its separate [`$count` stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/count/) is represented by this bounded txBASE stage without claiming full MongoDB pipeline compatibility.

## Related documents

- [Query model](query-model.md)
- [Query planning and external vocabulary](query-planning.md)
- [Quality contract matrix](quality-matrix.md)

## Primary references

- [MongoDB `$group` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/)
- [MongoDB `$count` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/count/)
