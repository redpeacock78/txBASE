# Query planning and external vocabulary

This document separates external query terminology from the local planner contract.

MongoDB is a reference for vocabulary and tradeoffs.

txBASE does not claim MongoDB query or planner compatibility.

## 1. External query vocabulary

The [MongoDB query predicate reference](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/) groups predicates into comparison, logical, array, element, evaluation, bitwise, geospatial, and miscellaneous families.

The current txBASE subset includes comparison, membership, logical, bounded array, and bounded field-expression predicates.

MongoDB's [find command](https://www.mongodb.com/docs/manual/reference/command/find/) separates a filter from projection, sort, skip, limit, hint, and related cursor controls.

That separation lets txBASE evolve query validation, result shaping, and planner access paths independently.

MongoDB's find command returns an initial batch and a cursor identifier.

Firestore's [query cursor guidance](https://firebase.google.com/docs/firestore/query-data/query-cursors) uses the last document in one batch as the start point for the next batch.

txBASE adopts that boundary idea for physical and sorted pages.

It does not claim server-side cursor lifetime or snapshot isolation.

MongoDB's [comparison predicate reference](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/comparison/) documents operators such as `$eq`, `$gt`, `$gte`, `$lt`, `$lte`, `$ne`, `$in`, and `$nin`.

MongoDB's [BSON comparison order](https://www.mongodb.com/docs/manual/reference/bson-type-comparison-order/) and [`$gt` type-bracketing rules](https://www.mongodb.com/docs/manual/reference/operator/query/gt/) define separate boundaries.

MongoDB compares BSON values with BSON-specific type and array rules.

txBASE compares decoded JSON values with its own explicit scalar rules.

Shared operator names therefore do not imply shared results for mixed types, missing fields, arrays, or documents.

MongoDB's [logical predicate reference](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/logical/) documents `$and`, `$or`, `$nor`, and `$not`.

MongoDB's [`$expr` predicate](https://www.mongodb.com/docs/manual/reference/operator/query/expr/) allows expressions inside a query predicate, including comparisons between two fields from the same document.

txBASE implements a bounded expression tree.

Comparison leaves such as `{"$gt":["$LEFT","$RIGHT"]}` may be composed with `$and`, `$or`, and `$not`.

Comparison operands may also contain a unary numeric `$abs` expression or a two-operand numeric `$add`, `$subtract`, `$multiply`, `$divide`, or `$mod` expression.

Integer arithmetic preserves JSON integer output when the result fits.

Mixed or fractional arithmetic must produce a finite JSON number.

Missing or nonnumeric field values make the comparison not match.

A string operand beginning with `$` is a dotted field reference.

All other scalar operands are literals.

If either field reference is missing, the expression does not match.

The expression path uses a table scan because a field-to-field comparison is not a constant-bound index lookup.

Regular-expression and document expressions remain unsupported.

txBASE implements bounded `$all`, `$elemMatch`, and `$size` predicates through a table scan.

`$all` accepts an array operand and an empty operand matches no records; non-empty operands require every candidate in an array-valued field, `$elemMatch` binds every condition to one array element, and `$size` matches one exact non-negative length.

These predicates are not index candidates because the current index contract does not define multikey keys or array-length statistics.

MongoDB also has a broad [miscellaneous predicate family](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/misc/) including regular expressions and expression evaluation.

Those operators need explicit encoding, resource, and error contracts before they belong in a file-native DBF query engine.

## 2. Planner boundary

The [MongoDB query optimization guide](https://www.mongodb.com/docs/manual/core/query-optimization/) explains why predicate selectivity and index key order affect the amount of data examined.

It also warns that low-selectivity operators such as `$ne` and `$nin` often do not benefit from an index like selective equality predicates do.

txBASE keeps the record scan as the query executor reference path.

The path-aware query entry point attempts external scalar-key or compound-key equality, range, or ordered traversal before applying the same filter, sort, projection, skip, and limit pipeline.

The planner reports `TableScan`, `EqualityIndex`, `CompoundEqualityIndex`, `CompoundEqualityPrefixIndex`, `IndexIntersection`, `RangeIndex`, `OrderedIndex`, `OrderedIndexPrefix`, or `CompoundOrderedIndex` through `explain_query_at`.

The single-table HTTP server exposes the same explanation as `QUERY /explain`.

Its response is `{"plan": {"kind": "table_scan"}}` or a tagged index-plan object with the selected name, fields, and directions.
When a valid index sidecar is available, the response also includes a `cost` object.

`explain_query_details_at` and the `cost` object expose deterministic row-equivalent work for the selected access path.

The planner uses bounded candidate-count, index-traversal, and remaining-sort work as the primary ordering, then uses logical-page work as a deterministic tie-break; the additional fields make record reads and filter evaluations observable without changing the established candidate-selection rule for index intersections.

`candidate_rows` is the exact candidate count after the selected access path.

`index_traversal` is the bounded logarithmic search work for the selected sidecar or sidecars.

`index_page_reads` estimates the 4 KiB logical pages needed to load the JSON sidecar once; it is zero for a table scan.

`record_reads` estimates the candidate records read from the DBF; an equality intersection includes the estimated lists read before their record-number intersection.

`record_page_reads` estimates the logical DBF pages touched by those records.

`filter_evaluations` is the number of candidate records passed through the normal filter pipeline.

`sort_work` is the estimated in-memory comparison work left after any ordered index prefix.

`total` is the saturating sum of all of those terms.

The values are deterministic row-equivalent estimates, not wall-clock measurements.

When the sidecar is missing, stale, malformed, or semantically unsupported, the planner falls back to `TableScan` and omits the cost because no sidecar statistics are available.

Missing, stale, malformed, and semantically unsupported sidecars fall back to `TableScan` because the sidecar is optional acceleration state.

Connecting an index to query execution therefore needs more than a parser change.

It needs key encoding, null and missing-field rules, duplicate ordering, update maintenance, recovery records, stale-index detection, and a planner policy.

The current planner considers direct top-level equality, exact compound equality, compound equality-prefix candidates, single-bound-per-side range predicates, single-field ordered traversal, compound equality-prefix range predicates, and single-key or compound sort requests whose fields match an index suffix after an exact equality prefix.

For multiple usable single-field equality indexes, the planner compares each single-index candidate with one record-number intersection candidate and chooses the lowest bounded cost.

The planner uses the sidecar's active-record count and each single-field index's distinct-key count to estimate equality cardinality before loading candidate lists.

It processes exact candidate lists in estimated-selectivity order.

This limits repeated membership checks when predicates have different expected cardinalities.

The equality estimate assumes a uniform distribution.

Single-field range indexes use a persisted equi-depth histogram and sum record counts of overlapping buckets.

A compound index can supply range candidates when every preceding indexed field has an exact equality predicate and the next indexed field has the range predicate.

The compound candidate path filters the indexed range component and returns physical record order before the normal query filter pipeline runs.

These statistics feed the deterministic estimates exposed by `QUERY /explain`.

The planner compares the active-record count for a table scan with the exact candidate count for an index path, adds a bounded logarithmic traversal term derived from the selected sidecar's entry count, and adds estimated in-memory sort work when the path does not provide the complete requested order.

The explanation additionally reports candidate record reads and filter evaluations, and an equality intersection includes estimated lists read before its record-number intersection.

Access-path candidate construction remains in `src/query/planner.rs`, while the bounded cost calculation lives in `src/query/planner_cost.rs`.

This is local planning logic, not MongoDB planner compatibility.

For a multi-key sort, a single-field index provides the first-key order.

A single-key sort can also use a compound index suffix when every preceding index field has an exact equality predicate.

The executor sorts remaining keys in memory within each equal first-key group.

An ascending or mixed-direction compound index provides complete order when requested sort fields match indexed fields after an exact equality prefix.

The planner accepts the index order or its complete reverse.

Compatible mixed-direction requests can therefore avoid an in-memory sort as well.

When equality, range, and ordered access paths coexist, the planner compares bounded work estimates for the table scan and each valid index path.

The table-scan estimate uses the active-record count.

An index-path estimate uses its exact candidate count, adds one bounded logarithmic traversal term per selected sidecar, and adds sort work when the path leaves part of the requested order to the executor.

An index intersection sums one traversal term for each selected sidecar.

An ordered path that supplies the complete requested order has no sort term.

Compound definitions use their shortest definition and stable name tie-breakers when candidate counts are equal.

An exact cost tie preserves the table scan and established access paths before a compound equality-prefix prefilter.

The planner uses a deterministic 4 KiB logical-page model for the persisted JSON index and DBF bytes.
Candidate record pages use a bounded worst-case estimate because the planner does not materialize page maps.
The model does not claim to measure filesystem latency, memory, cache state, collation, or compound-range selectivity.

MongoDB's current guidance recommends a compound index for queries that repeatedly search multiple fields.

The txBASE intersection is a local candidate-reduction feature.

It does not claim MongoDB planner compatibility or replace a future compound-index contract.

MongoDB's [compound-index sort-order guidance](https://www.mongodb.com/docs/manual/core/indexes/index-types/index-compound/sort-order/) and [equality-sort-range guideline](https://www.mongodb.com/docs/manual/tutorial/equality-sort-range-guideline/) show why a full compound-index planner must define index field order.

txBASE currently has an active-record count, a uniform distinct-key estimate for equality, a single-field range histogram, compound equality-prefix and compound equality-prefix range candidates, per-field direction metadata for compound definitions, and an explainable cost estimate for scan, index traversal, logical index pages, candidate record reads, logical record pages, filter evaluations, and remaining sort work.

The cost model is physical-layout aware at the logical 4 KiB page level, while actual filesystem and cache behavior remains outside the contract.

The equality intersection is a bounded candidate prefilter, not a covered query or a claim of end-to-end speedup.

Direct equality joins use a separate deterministic cost model that keeps the 64-candidate-pair nested-loop boundary, estimates pre-filter join cardinality from equality-key multiplicities, counts logical DBF pages from the loaded byte lengths, and includes output materialization work from the estimated candidate rows and projection width.

The direct and chained join models are not part of the single-table `QUERY /explain` cost object.

Chained join stages propagate estimated cardinality, materialized row width, and logical page inputs through each intermediate stage for hash and index-probe selection.

An ordered-merge path remains limited to direct joins, and filesystem- and cache-aware merge behavior remains outside the contract.

The roadmap keeps index design separate from query syntax so a query document does not imply an implementation strategy.

## 3. Future query work

The following require separate public contracts:

1. Full expression evaluation and more precise cost-based index choice with explicit missing, null, collation, and compound-range selectivity rules.
2. Additional aggregation stages and accumulators beyond the current bounded aggregation contract, including its bounded numeric `$sum` and `$avg` expressions, with bounded memory behavior.
3. Filesystem- and cache-aware merge planning, streaming join execution, and broader join semantics.
4. Host-specific scheduling, backpressure, timeout, cancellation, and transport implementations for `AsyncQueryStream`.
5. Differential tests against a small reference evaluator.

Until those contracts exist, the record scan remains the simpler reference execution model.

## Primary references

- [MongoDB query predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/)
- [MongoDB comparison predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/comparison/)
- [MongoDB logical predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/logical/)
- [MongoDB `$expr` predicate](https://www.mongodb.com/docs/manual/reference/operator/query/expr/)
- [MongoDB array predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/arrays/)
- [MongoDB `$all` query predicate](https://www.mongodb.com/docs/manual/reference/operator/query/all/)
- [MongoDB `$elemMatch` query predicate](https://www.mongodb.com/docs/manual/reference/operator/query/elemmatch/)
- [MongoDB `$size` query predicate](https://www.mongodb.com/docs/manual/reference/operator/query/size/)
- [MongoDB find command](https://www.mongodb.com/docs/manual/reference/command/find/)
- [Firestore query cursors](https://firebase.google.com/docs/firestore/query-data/query-cursors)
- [MongoDB query optimization](https://www.mongodb.com/docs/manual/core/query-optimization/)
- [MongoDB BSON comparison order](https://www.mongodb.com/docs/manual/reference/bson-type-comparison-order/)
- [MongoDB `$gt` type bracketing](https://www.mongodb.com/docs/manual/reference/operator/query/gt/)
- [MongoDB compound-index sort order](https://www.mongodb.com/docs/manual/core/indexes/index-types/index-compound/sort-order/)
- [MongoDB equality-sort-range guideline](https://www.mongodb.com/docs/manual/tutorial/equality-sort-range-guideline/)
- [SQLite query planning](https://www.sqlite.org/queryplanner.html)
