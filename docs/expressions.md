# Expression model

This document defines the shared scalar and boolean expression subset.
It records txBASE behavior, not MongoDB compatibility.

## 1. Evaluation surfaces

`$expr` accepts a boolean expression over the current record.

The shared scalar evaluator is used by aggregation `$set` and `$addFields`, group keys, `$sortByCount`, `$bucket` and `$bucketAuto` group expressions, and scalar-valued accumulator inputs.
Numeric accumulators use the bounded numeric-expression grammar described below.

The `n` value for `$minN`, `$maxN`, `$firstN`, and `$lastN` may depend only on the group `_id` and literals.
Scoped variables are checked recursively; `$$ROOT` and `$$CURRENT` do not pass this group-key restriction.

The evaluator accepts JSON scalar values, field paths such as `"$PRICE"`, arrays of expressions, and one-key operator objects.
Use `$literal` to preserve a value that would otherwise be interpreted as an expression.
Computed multi-field objects are not supported.

## 2. Boolean expressions

The boolean grammar accepts `$and`, `$or`, and `$not`, plus two-operand `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, and `$lte` comparisons.
Each operator object must contain exactly one operator.

`$and` and `$or` short-circuit from left to right.
Comparisons with a missing operand return `false`; ordered comparisons with values that cannot be compared also return `false`.

`$cond.if` and `$filter.cond` use this boolean grammar.
They do not accept arbitrary truthy scalar expressions.

## 3. Scalar expressions

The shared scalar evaluator supports:

- `$literal` and array constructors; a missing array element becomes `null`.
- `$ifNull` with exactly two operands.
- `$cond` as either a three-element array or an object with exactly `if`, `then`, and `else`.
- `$concat` with at least two string expressions, and unary `$toLower` and `$toUpper`.
- Unicode code-point operators `$strLenCP`, `$substrCP`, and `$indexOfCP`.
- String splitting with `$split`, and literal replacement with `$replaceOne` and `$replaceAll`.
- Unary numeric `$abs`, `$ceil`, and `$floor`.
- Binary numeric `$add`, `$subtract`, `$multiply`, `$divide`, and `$mod`.
- `$let`, `$map`, `$filter`, and `$reduce`, defined below.

### String operator behavior

`$strLenCP` counts Unicode code points and errors when its input is missing, `null`, or not a string.

`$substrCP` accepts `[input, start, count]` and uses zero-based code-point offsets.
It returns an empty string for a missing or `null` input; otherwise, `start` and `count` must resolve to non-negative integers representable as `u64`.
Integer-valued JSON numbers such as `2.0` are accepted.

`$split` requires string input and delimiter expressions.
txBASE rejects an empty delimiter; leading, repeated, and trailing delimiters produce empty array elements.
If the delimiter does not occur, the result contains the input as its only element.
Regular-expression delimiters are unsupported, and the materialized result uses the 1 MiB array limit below.

`$indexOfCP` accepts `[input, search, start?, end?]` and returns a zero-based code-point index.
The search range includes `start` and excludes `end`.
`start` and `end` must be non-negative integers representable as `u64`; a missing or `null` input returns `null`, while a missing or non-string search value is an error.
It returns `-1` under any of these conditions:

- No match exists.
- `start` exceeds the input length.
- `start` is greater than `end`.

An `end` beyond the input is clamped to its length.

`$replaceOne` and `$replaceAll` require an object with exactly `input`, `find`, and `replacement` string expressions.
If any operand is missing or `null`, the result is `null`; other non-string values are errors.
Matching is literal and case-sensitive, does not normalize Unicode, and replaces the first or all non-overlapping matches, respectively.
txBASE rejects an empty `find` value, and regular-expression patterns are unsupported.

Numeric operators accept numeric literals, field or variable references, and nested numeric expressions.
Integer results remain JSON integers when representable; other results must be finite JSON numbers.
Division or modulo by zero, integer overflow, and non-finite results are errors.

Missing or incompatible operands do not match in `$expr`.
In `$set` and `$addFields`, a missing or incompatible scalar result becomes `null`.
The aggregation stage still evaluates each computed field against the record as it entered that stage; sibling fields computed in the same stage are not visible.

## 4. Variables and lexical scope

Write variable references with two dollar signs, for example `"$$item.price"`.
A dotted suffix resolves a path inside the referenced value.

User variable names start with an ASCII lowercase letter or a non-ASCII character.
Remaining characters may be ASCII letters, digits, underscores, or non-ASCII characters.
Names are case-sensitive.

The supported system variables are `$$ROOT`, `$$CURRENT`, and `$$IDX`.
`$$ROOT` and `$$CURRENT` refer to the record at the current evaluator boundary.
`$$IDX` is bound to the zero-based index while evaluating an array element when `arrayIndexAs` is omitted.
Other system variables are rejected.

`$let` requires exactly `vars` and `in`.
Each binding is evaluated in the enclosing scope, so bindings in the same `vars` object cannot refer to one another.
The bindings are visible only in `in`; nested scopes may shadow them.

```json
{
  "$let": {
    "vars": {"taxed": {"$multiply": ["$PRICE", 1.1]}},
    "in": {"$add": ["$$taxed", "$SHIPPING"]}
  }
}
```

## 5. Array expressions

`$map` applies `in` to each item and returns the results in input order.
`$filter` returns matching input items in input order.
`$reduce` folds from left to right and returns one value.

```json
{
  "$map": {
    "input": "$ITEMS",
    "as": "item",
    "arrayIndexAs": "position",
    "in": ["$$item.price", "$$position"]
  }
}
```

`as` defaults to `this` for all three operators.
`arrayIndexAs` names the zero-based item index; when it is omitted, use `$$IDX`.
`$reduce` also accepts `valueAs`, which defaults to `value`.
Item, accumulator, and custom index names must be distinct within one operator.

`$filter.limit` is optional.
It accepts an expression that resolves to a positive JSON integer or `null`; `null` means no limit.
A missing result or any other value is an error.
The operator returns at most that many matches.

For all three operators, a missing or `null` input returns `null`, while a non-array, non-null input is an error.
An empty `$reduce` input returns `initialValue`.
An unbound variable or a malformed option is an error.

## 6. Evaluation limits and references

One scalar evaluation may visit at most 100,000 array items across nested array expressions.
Each materialized array is limited to 1 MiB of encoded JSON.
Computed strings are also limited to 1 MiB.
Exceeding a limit returns a query error.

Unsupported expression operators and unsupported system variables are rejected.
The JSON model, missing-value rules, and type ordering differ from MongoDB; the references below inform selected operator shapes and edge cases only.

## Related documents

- [Query model](query-model.md)
- [Aggregation model](aggregation.md)
- [Aggregation accumulators](aggregation-accumulators.md)
- [Quality contract matrix](quality-matrix.md)

## Primary references

- [MongoDB `$expr` query operator](https://www.mongodb.com/docs/manual/reference/operator/query/expr/)
- [MongoDB aggregation expressions](https://www.mongodb.com/docs/manual/reference/aggregation/)
- [MongoDB aggregation expression operators](https://www.mongodb.com/docs/manual/reference/operator/aggregation/)
- [MongoDB `$let` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/let/)
- [MongoDB aggregation variables](https://www.mongodb.com/docs/manual/reference/aggregation-variables/)
- [MongoDB `$map` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/map/)
- [MongoDB `$filter` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/filter/)
- [MongoDB `$reduce` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/reduce/)
- [MongoDB `$cond` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/cond/)
- [MongoDB `$ifNull` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/ifnull/)
- [MongoDB `$literal` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/literal/)
- [MongoDB `$concat` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/concat/)
- [MongoDB `$toLower` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/tolower/)
- [MongoDB `$toUpper` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/toupper/)
- [MongoDB `$strLenCP` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/strlencp/)
- [MongoDB `$substrCP` expression](https://www.mongodb.com/docs/v8.3/reference/operator/aggregation/substrcp/)
- [MongoDB `$split` expression](https://www.mongodb.com/docs/v8.2/reference/operator/aggregation/split/)
- [MongoDB `$indexOfCP` expression](https://www.mongodb.com/docs/v8.3/reference/operator/aggregation/indexofcp/)
- [MongoDB `$replaceOne` expression](https://www.mongodb.com/docs/v7.0/reference/operator/aggregation/replaceone/)
- [MongoDB `$replaceAll` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/replaceall/)
- [MongoDB `$abs` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/abs/)
- [MongoDB `$ceil` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/ceil/)
- [MongoDB `$floor` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/floor/)
- [MongoDB `$add` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/add/)
- [MongoDB `$subtract` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/subtract/)
- [MongoDB `$multiply` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/multiply/)
- [MongoDB `$divide` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/divide/)
- [MongoDB `$mod` expression](https://www.mongodb.com/docs/manual/reference/operator/aggregation/mod/)
