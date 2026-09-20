# txBASE documentation

Start with the document that matches the contract being changed.

Current behavior is labeled explicitly.

Research and future work do not imply an implemented feature.

## Storage and compatibility

- [DBF and dBASE compatibility](dbf-compatibility.md)
- [Schema metadata and local constraints](schema-metadata.md)
- [Secondary-index sidecar](indexes.md)
- [Multi-table catalog](catalog.md)
- [XBF v1 format draft](xbf.md)

## Query and protocol contracts

- [Query model](query-model.md)
- [Aggregation model](aggregation.md)
- [Join model](joins.md)
- [Query planning and external vocabulary](query-planning.md)
- [Mutation model](mutation-model.md)
- [HTTP method semantics and QUERY](http-semantics.md)

## Quality and research

- [SQLite testing and quality model](testing-quality.md)
- [Quality contract matrix](quality-matrix.md)
- [Firebase data-model and synchronization lessons](firebase-model.md)
- [Specification research index](research.md)
- [Roadmap and explicit non-goals](roadmap.md)

## File-granularity rule

Split a document when ownership, failure behavior, fixtures, or change cadence differ.

Keep related contracts together when splitting would only create navigation overhead.
