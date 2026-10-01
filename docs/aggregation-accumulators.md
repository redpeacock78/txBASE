# Aggregation accumulators

`$group`, `$bucket`, and `$bucketAuto` use the same bounded accumulator set. See [Aggregation model](aggregation.md) for pipeline placement, stage order, and bucket behavior.

## Numeric accumulators

- `$count: {}` counts each record in the group.
- `$sum` accepts a field reference, numeric literal, `$abs`, `$ceil`, `$floor`, or binary `$add`, `$subtract`, `$multiply`, `$divide`, and `$mod` expressions. Missing, `null`, and nonnumeric results contribute zero. Integral inputs retain an integer JSON result; any fractional input produces a finite floating-point result.
- `$avg` accepts the same expressions. Missing, `null`, and nonnumeric results are ignored; a group with no numeric inputs returns `null`.
- `$stdDevPop` and `$stdDevSamp` accept the same expressions and use constant memory per group. They ignore missing, `null`, and nonnumeric results. `$stdDevPop` returns zero for one numeric input; `$stdDevSamp` returns `null` until two inputs exist.

Non-finite intermediate or result values are rejected. The numeric expression evaluator is shared with `$expr`.

`$ceil` and `$floor` return the mathematical ceiling and floor of their numeric result.
Their integral results use a JSON integer when they fit `i64` or `u64`.

## Extrema and N-value selection

`$min` and `$max` take a field reference. They ignore missing and `null` inputs and return `null` when a group has no other value.

`$minN` and `$maxN` take an `input` expression and an `n` expression:

```json
{
  "$minN": {
    "input": ["$SCORE", "$PLAYER"],
    "n": 2
  }
}
```

`input` is evaluated for each record. Missing and `null` results are ignored, and duplicate values remain in the output. Array expressions can combine field references and other supported scalar expressions.
If every result is missing or `null`, the accumulator returns an empty array.

`$firstN` and `$lastN` use the same `input` and `n` shape:

```json
{
  "first": {"$firstN": {"input": "$SCORE", "n": 2}},
  "last": {"$lastN": {"input": "$SCORE", "n": 2}}
}
```

Both evaluate `input` for each record and retain values in group input order. Missing inputs become `null`, explicit `null` values remain, and duplicate values are retained. `$firstN` keeps the first `n` values; `$lastN` keeps the last `n` values and returns that suffix in input order. If a group has fewer than `n` records, the result contains all of its input values.

Without a preceding input `$sort`, group input order follows physical record order. An input `$sort` establishes the order used by these accumulators.

txBASE supports `$firstN` and `$lastN` only as `$group`, `$bucket`, and `$bucketAuto` accumulators. MongoDB's array-operator and window-operator forms are outside this contract.

`n` must evaluate to a positive integer no greater than 10,000. It may be constant or depend only on the output group's `_id`; txBASE evaluates it once when it creates the group. This bound is specific to txBASE.

txBASE returns `$minN` values in ascending order and `$maxN` values in descending order; equal values retain input order. MongoDB does not promise a particular output order for these accumulators, so txBASE's deterministic ordering is an explicit difference.

The comparison follows MongoDB's BSON type order for JSON values: `null`, numbers, strings, objects, arrays, then booleans. Strings use binary ordering; arrays compare lexicographically; objects compare member pairs by value type, field name, and value in txBASE's JSON map iteration order. BSON-only values and BSON object insertion order are outside the JSON data model.

## Order and collection accumulators

- `$first` and `$last` use group input order, which follows physical-record order unless an input `$sort` establishes another order. They return the selected field value, including explicit `null`; a missing field becomes `null`.
- `$push` returns all field values in group input order. `$addToSet` returns each structurally equal JSON value once, in first-seen order. Both append missing fields as `null`.
- `$mergeObjects` accepts a supported scalar expression that resolves to a document. It ignores missing and `null` results, merges fields in group input order, and lets later documents overwrite earlier values for the same field. A non-null non-document result is an error. If every result is missing or `null`, it returns `{}`. Merging is shallow.

txBASE supports `$mergeObjects` only as an accumulator in `$group`, `$bucket`, and `$bucketAuto`; the expression form is outside this contract.

The combined retained-value limit for `$push`, `$addToSet`, `$minN`, `$maxN`, `$firstN`, and `$lastN` is 10,000 values per `$group`, `$bucket`, or `$bucketAuto` stage execution, shared across that stage's groups and accumulators.
`$mergeObjects` counts each field added to a group's merged document against the same limit; overwriting a field already present in that group does not count again.
Each `n` has an individual maximum of 10,000, although a stage can reach the shared limit first.

## Primary references

- [MongoDB `$group` stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/)
- [MongoDB `$sum` accumulator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/sum/)
- [MongoDB `$avg` accumulator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/avg/)
- [MongoDB `$stdDevPop` accumulator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/stddevpop/)
- [MongoDB `$stdDevSamp` accumulator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/stddevsamp/)
- [MongoDB `$ceil` expression operator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/ceil/)
- [MongoDB `$floor` expression operator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/floor/)
- [MongoDB `$minN` accumulator](https://www.mongodb.com/docs/v8.0/reference/operator/aggregation/minn/)
- [MongoDB `$maxN` accumulator](https://www.mongodb.com/docs/v8.0/reference/operator/aggregation/maxn/)
- [MongoDB `$firstN` accumulator](https://www.mongodb.com/docs/v8.0/reference/operator/aggregation/firstn/)
- [MongoDB `$lastN` accumulator](https://www.mongodb.com/docs/v8.0/reference/operator/aggregation/lastn/)
- [MongoDB 8.0 `$mergeObjects` accumulator](https://www.mongodb.com/docs/v8.0/reference/operator/aggregation/mergeObjects/)
- [MongoDB BSON comparison order](https://www.mongodb.com/docs/v8.0/reference/bson-type-comparison-order/)
