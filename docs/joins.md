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

The public join parser rejects documents larger than `MAX_JSON_INPUT_BYTES`, currently 1 MiB,
before deserializing them.

The HTTP catalog server rejects a larger request at its body boundary with `413 Payload Too Large`.

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

For larger equality stages, it compares bounded costs for the available strategies.

`Hash` costs one pass over both inputs, the logical DBF pages needed for those inputs, and the bounded work of building an equality map for the inner side.

`IndexNestedLoop` costs the outer-row count multiplied by the inner-side logarithmic probe estimate plus its average equality fanout, then adds the outer logical DBF pages, a one-time logical page estimate for the index sidecar, and a conservative logical DBF record-page estimate for each probe.

`Merge` costs one pass over both inputs, the logical DBF pages needed for those inputs, and the logical page estimates for the two ordered index sidecars when compatible ordered indexes are fresh on both sides.

Each direct or chained candidate also includes deterministic materialization work based on the estimated pre-filter join-candidate rows and the materialized row width.

Equality-key multiplicities give an exact pre-filter candidate-row count for each loaded stage, including unmatched rows for outer joins and one row per matching left record for `semi` or `anti`; the final filter can reduce the emitted count afterward.

The strategy with the lower estimate wins, with `Merge` preferred over `IndexNestedLoop`, and `IndexNestedLoop` preferred over `Hash` for an exact tie.

For a direct single-key equality join or a chained single-key equality stage, the indexed table is the inner side of the probe.

For a chained `right` stage, it probes the indexed right table and restores right-major physical order before emitting the intermediate result.

Direct and chained stages may also use a fresh compound index when the equality fields exactly match the index field order.

For a large direct equality join with compatible fresh ordered indexes on both inputs, the planner selects `Merge` when its bounded cost is lowest and restores left-major or right-major output order after matching key groups.

For a chained non-`full` equality stage with a fresh exact ordered index on the newly joined table, the planner also compares `Merge`.
It sorts the materialized intermediate rows by the stage keys, includes bounded sort work in the merge estimate, consumes the loaded new-table rows in index order, and restores the documented output order.

When an index is unavailable, stale, malformed, or more expensive than the hash estimate, larger inputs select `Hash` and build one in-memory equality map for the right table, or the left table for a `right` join.

A direct `full` join uses the compatible ordered-index merge path when both sides are fresh and that bounded merge cost wins; otherwise it uses the bounded hash fallback.

Chained `full` stages use the bounded hash fallback and do not use the direct ordered-index merge path.

It does not use an index-nested-loop strategy.

The selector preserves left-major output order for non-right joins and right-major output order for right joins.

It rejects a result larger than 100,000 rows.

For multiple joins, the required `join` object is the first stage and an optional `joins` array adds stages from left to right.

Each additional stage may reference any table already present in the intermediate row and adds one new table.

Table names cannot repeat, and the total stage count is capped at eight.

Every intermediate result is capped at 100,000 rows.

This is a bounded row-work, logical-page, pre-filter-cardinality, and output-materialization cost model with a compatible-index merge path.

Chained stages propagate their estimated cardinality, materialized row width, and logical page inputs into the hash or index-probe choice for the next stage.

The model does not measure filesystem latency, cache state, or page reuse.

Filesystem- and cache-aware merge planning remains future work.

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
