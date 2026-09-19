# Firebase data-model and synchronization lessons

Firebase is not one storage model.

Cloud Firestore and the Firebase Realtime Database make different tradeoffs, so txBASE treats them as separate references.

The useful question is not how to imitate Firebase.

The useful question is which local-state, durability, conflict, and authorization boundaries should be explicit in a file-native database.

## 1. Cloud Firestore

Firestore stores documents in collections.

Documents can contain nested objects and can have subcollections.

The [Firestore data model](https://firebase.google.com/docs/firestore/data-model) does not expose tables and rows as its primary abstraction.

That model makes document identity and collection paths part of the API.

Firestore transactions and batched writes have different purposes.

According to the [transaction and batched-write documentation](https://firebase.google.com/docs/firestore/manage-data/transactions):

- A transaction reads and writes documents atomically.
- A transaction function may run more than once when a concurrent edit causes a retry.
- Transaction reads must happen before transaction writes.
- A transaction fails when the client is offline.
- A batched write commits multiple writes atomically without requiring reads.

The retry rule is a major application contract.

A transaction callback must be safe to execute more than once.

External side effects must not be hidden inside a retryable callback.

Firestore also documents [write-time aggregation](https://firebase.google.com/docs/firestore/solutions/aggregation), where an application updates a summary document as source documents change.

That approach trades read cost for write complexity and requires a repair strategy when a summary becomes stale.

## 2. Realtime Database

The Realtime Database stores a JSON tree addressed by references.

The [save-data guide](https://firebase.google.com/docs/database/admin/save-data) distinguishes `set`, `update`, `push`, and transaction operations.

`set` replaces the value at a path.

`update` changes selected child paths without replacing unrelated siblings.

`push` creates a generated child key.

A transaction reads the current value, computes a new value, and retries when another writer changes the value first.

The [web read and write guide](https://firebase.google.com/docs/database/web/read-and-write) also makes local events and asynchronous server synchronization visible to clients.

The tree model and the document model should not be combined casually.

A DBF table has fixed physical records and field descriptors, not arbitrary nested paths.

## 3. Security and validation rules

Realtime Database security rules separate several concerns:

| Rule | Role |
| --- | --- |
| `.read` | Authorize reads |
| `.write` | Authorize writes |
| `.validate` | Validate new data after a write is otherwise authorized |
| `.indexOn` | Declare query-relevant indexes for a path |

The [security documentation](https://firebase.google.com/docs/database/security) describes server-side enforcement and the default-deny posture.

Validation rules are not a replacement for a storage format constraint.

An authorization decision answers who may perform an operation.

A schema constraint answers whether the resulting value is valid.

An index declaration answers how a query can be served efficiently.

txBASE currently implements none of Firebase's rules language or authentication model.

Those concerns should remain separate if an HTTP authorization layer is added later.

## 4. Offline and local-first behavior

Firebase client libraries can expose local state before the server has accepted a write.

That behavior needs a durability label.

The [Realtime Database offline guide](https://firebase.google.com/docs/database/android/offline-capabilities) describes queued writes and notes that Realtime Database transactions are not persisted across an app restart.

An in-memory local event, a queued client write, a synced WAL record, and a published DBF replacement are four different states.

They must not be reported to callers as the same kind of success.

The current txBASE path-loaded mutation contract is narrower:

1. A mutation is validated against the loaded DBF schema.
2. A durable intent or state payload is appended to the WAL.
3. The WAL is synced.
4. The DBF and any memo sidecar are replaced atomically as far as the current file protocol permits.
5. Startup recovery replays the supported intent or state payload after an interruption.

There is no client cache, listener stream, offline queue, or last-write-wins policy in the current server.

## 5. Identity and conflict lessons

Firebase makes a stable path or document key central to reads and writes.

txBASE currently uses the one-based physical DBF record number in its HTTP path.

That choice preserves the DBF layout but exposes physical identity to clients.

Deleted record numbers are not reused by the current mutation layer.

A future logical identifier would need a uniqueness rule, migration behavior, and an index or lookup contract.

The current table lock serializes save paths.

A separately loaded stale table is rejected rather than automatically merged.

This is a deliberate conflict policy, not Firebase synchronization.

## 6. What would need a new contract

The following capabilities are not implied by this document:

- Realtime listeners or change feeds.
- Offline client persistence.
- Authentication or Firebase security-rule evaluation.
- Last-write-wins merge semantics.
- Multi-document or multi-table transactions.
- Server-maintained aggregation documents.

Each would need failure behavior, recovery behavior, and tests for concurrent changes before implementation.

## Primary references

- [Firestore data model](https://firebase.google.com/docs/firestore/data-model)
- [Firestore transactions and batched writes](https://firebase.google.com/docs/firestore/manage-data/transactions)
- [Firestore write-time aggregation](https://firebase.google.com/docs/firestore/solutions/aggregation)
- [Realtime Database save data](https://firebase.google.com/docs/database/admin/save-data)
- [Realtime Database web read and write](https://firebase.google.com/docs/database/web/read-and-write)
- [Realtime Database security](https://firebase.google.com/docs/database/security)
- [Realtime Database offline capabilities](https://firebase.google.com/docs/database/android/offline-capabilities)
