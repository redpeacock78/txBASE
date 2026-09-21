# Join model

txBASE keeps relational joins as a bounded catalog operation over the same JSON field and projection rules used by ordinary queries.

The join boundary is separate from single-table aggregation and from cross-table mutation transactions.

## 1. Bounded local join

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

The current join types are `inner`, `left`, `right`, `full`, `semi`, `anti`, and `cross`.

The result is a flat JSON object whose keys use the `table.field` form.

An unmatched left row is retained without right-table fields.

An unmatched right row is retained without left-table fields for a `right` join.

A `full` join retains unmatched rows from both tables.

It emits matched and unmatched left rows in left-table order, then emits unmatched right rows in right-table order.

`semi` emits one left row when at least one right row matches.

`anti` emits one left row when no right row matches.

Neither type emits right-table fields.

`cross` accepts an empty `on` object and emits every left and right pair.

Its candidate pair count is capped at 100,000 before filtering.

Missing and explicit `null` join keys do not match.

The equality-join planner selects `NestedLoop` when the left and right active-row counts have at most 64 candidate pairs.

For larger inputs, it compares bounded costs for the available strategies.

`Hash` costs one pass over both inputs.

`IndexNestedLoop` costs the outer-row count multiplied by the inner-side logarithmic probe estimate plus its average equality fanout.

`Merge` costs one pass over both inputs when compatible ordered indexes are fresh on both sides.

The strategy with the lower estimate wins, with `Merge` preferred over `IndexNestedLoop`, and `IndexNestedLoop` preferred over `Hash` for an exact tie.

For a direct single-key equality join or a chained single-key equality stage, the indexed table is the inner side of the probe.

For a chained `right` stage, it probes the indexed right table and restores right-major physical order before emitting the intermediate result.

Direct and chained stages may also use a fresh compound index when the equality fields exactly match the index field order.

For a large direct equality join with compatible fresh ordered indexes on both inputs, the planner selects `Merge` when its bounded cost is lowest and restores left-major or right-major output order after matching key groups.

When an index is unavailable, stale, malformed, or more expensive than the hash estimate, larger inputs select `Hash` and build one in-memory equality map for the right table, or the left table for a `right` join.

A `full` join uses the compatible ordered-index merge path when both sides are fresh and that bounded merge cost wins; otherwise it uses the bounded hash fallback.

It does not use an index-nested-loop strategy.

The selector preserves left-major output order for non-right joins and right-major output order for right joins.

It rejects a result larger than 100,000 rows.

For multiple joins, the required `join` object is the first stage and an optional `joins` array adds stages from left to right.

Each additional stage may reference any table already present in the intermediate row and adds one new table.

Table names cannot repeat, and the total stage count is capped at eight.

Every intermediate result is capped at 100,000 rows.

This is a bounded cardinality cost model with a compatible-index merge path.

It does not estimate index I/O, cache state, duplicate-key fanout, or output materialization, so it is not a full cost-based planner, full index-aware join planner, or streaming executor.

The single-table HTTP server does not expose joins.

The catalog server exposes the same read-only boundary at `QUERY /join`.

Cross-table writes and transactions remain outside this surface.

The join accepts the existing filter and projection rules, but not sort, pagination, aggregation, self-join aliases, or cross-table transactions.

MongoDB's [`$lookup` stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/lookup/) is reference vocabulary.

MongoDB describes `$lookup` as a left outer join that adds matching foreign documents as an array, while txBASE emits flat relational rows to match the attached join plan.

The MongoDB documentation also calls out the performance cost of an unindexed foreign-side join.

That is why this first slice has a hard result bound and makes no planner-level performance claim.

## Related documents

- [Query model](query-model.md)
- [Multi-table catalog](catalog.md)
- [Query planning and external vocabulary](query-planning.md)
- [Quality contract matrix](quality-matrix.md)

## Primary references

- [MongoDB `$lookup` join stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/lookup/)
