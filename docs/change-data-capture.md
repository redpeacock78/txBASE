# Change data capture

txBASE records committed single-table row changes in a durable CDC sidecar.

The sidecar is an observation boundary for local consumers.

It is not a replication protocol, a delivery queue, or a claim of PostgreSQL logical-decoding compatibility.

## 1. Sidecar and event format

For `users.dbf`, the CDC sidecar is `users.txbase.cdc`.

The sidecar uses the existing `TXWL` record container, with one `TXCD` payload per committed DBF transaction.

`TXCD` version 1 contains a JSON event with this shape:

```json
{
  "transaction_id": 2,
  "reset": false,
  "changes": [
    {
      "record_number": 1,
      "before": {
        "deleted": false,
        "values": {"ID": 1, "NAME": "Alice"}
      },
      "after": {
        "deleted": false,
        "values": {"ID": 1, "NAME": "Bob"}
      }
    }
  ]
}
```

`transaction_id` is the positive DBF commit ID persisted by the same transaction boundary as the DBF WAL.

`changes` is ordered by one-based physical DBF record number.

An insert has no `before` state.

A logical delete keeps the `after` state and sets its `deleted` flag to `true`.

An empty `changes` array is valid when a committed operation changes no row values.

## 2. Commit and recovery

The CDC payload is written to the DBF WAL before target replacement.

The DBF, memo, index, transaction-state, and MVCC targets are committed before the CDC event is published to the CDC sidecar.

If publication fails after the table commit, the WAL remains and the next normal DBF read retries the same event.

Publishing an event with an existing transaction ID is idempotent only when the event bytes describe the same change.

A different event for an existing transaction ID, an out-of-order transaction ID, or a malformed `TXCD` payload is rejected.

The CDC reader accepts the same torn final `TXWL` boundary as the WAL reader and removes only the incomplete tail when it opens the sidecar.

The recovery path validates the CDC transaction ID against `TXTI` and publishes the event only after the DBF recovery has completed.

## 3. Layout changes

`reset` is `true` when a physical layout change can invalidate record-number identity, such as `PACK`.

Such an event contains the before and after states for the affected record-number range.

Consumers must treat a reset event as a replacement boundary instead of applying each record number as a stable logical key.

The current event does not include schema-sidecar edits.

Catalog transactions do not yet publish one multi-table CDC envelope.

Those cases require a catalog-level event contract before they can be presented as one atomic change stream.

## 4. Reading events

The Rust API returns committed events after an optional exclusive transaction ID:

```rust
use txbase::dbf::DbfTable;

let events = DbfTable::cdc_events("users.dbf", Some(10))?;
```

The CLI exposes the same read boundary:

```bash
txbase cdc users.dbf
txbase cdc users.dbf --after 10
```

An absent sidecar returns an empty JSON array.

The `--after` cursor is a read position, not an acknowledgement or a persisted consumer slot.

CDC retention is explicit file maintenance and is not performed automatically.

`backup` and `restore` validate and copy the CDC sidecar together with the DBF and other supported sidecars.

## 5. Scope and future work

The event records state differences for one DBF commit and preserve commit order.

They do not provide a network transport, consumer leases, acknowledgements, backpressure, schema evolution, or replay into another table.

They do not include catalog transaction IDs as a cross-table envelope.

An adapter for replication or an external consumer must define those behaviors separately.

SQLite's [session extension](https://www.sqlite.org/sessionintro.html) is a reference for changeset shape and conflict-aware change transport.

PostgreSQL's [logical decoding](https://www.postgresql.org/docs/current/logicaldecoding.html) is a reference for deriving committed changes from WAL and exposing them to external consumers.

Those systems provide broader contracts than this local sidecar and do not make txBASE compatible with their formats or protocols.

## Primary references and scope

- [SQLite session extension](https://www.sqlite.org/sessionintro.html)
- [PostgreSQL logical decoding](https://www.postgresql.org/docs/current/logicaldecoding.html)
- [DBF compatibility boundary](dbf-compatibility.md)
- [Snapshot transactions](transactions.md)
- [MVCC and historical snapshots](mvcc.md)

The external documents provide terminology and design lessons.

The `TXCD` payload, physical-record reset rule, sidecar path, recovery ordering, and CLI cursor are txBASE contracts.
