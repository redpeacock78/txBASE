# Schema metadata and local constraints

DBF field descriptors describe the legacy file layout.

They do not provide a safe place for every modern relational constraint.
txBASE therefore keeps optional relational metadata in a sibling file instead of changing DBF bytes or pretending that a DBF field flag has a stronger meaning than it does.

## 1. Sidecar identity

For `users.dbf`, the metadata sidecar is:

```text
users.txschema.json
```

The sidecar is optional.

Without it, a table keeps the existing DBF-only behavior.

With it, `txbase schema`, `txbase verify`, path-loaded mutations, and backup or restore expose and preserve the metadata.

Use the schema command to validate and install a metadata candidate without changing DBF bytes:

```bash
txbase schema apply users.dbf users.txschema.candidate.json
```

The candidate is checked against the current DBF field descriptors and active records before the existing sidecar is atomically replaced.

The current sidecar is versioned independently from DBF:

```json
{
  "format": "txbase-schema",
  "version": 1,
  "fields": {
    "ID": {"primary": true},
    "NAME": {"not_null": true, "default": "Unknown"}
  },
  "checks": [
    {"AGE": {"$gte": 0}}
  ],
  "constraints": {
    "unique": [["NAME", "AGE"]],
    "foreign_keys": [
      {
        "fields": ["TENANT_ID", "USER_ID"],
        "references": {
          "table": "users",
          "fields": ["TENANT_ID", "ID"]
        },
        "deferred": true,
        "on_delete": "no_action",
        "on_update": "no_action"
      }
    ]
  ]
}
```

Schema version 1 remains supported.
Version 2 adds named, deferrable constraints without changing the DBF bytes.
The `deferred` foreign-key property shown above is also accepted in version 1.
Version 1 cannot use the version 2 naming or `deferrable` properties.

The optional root `encoding` property is an explicit override for the four declared multibyte codecs
plus strict Shift_JIS, EUC-JP, GB18030, and ISO-2022-JP.

Accepted labels are `windows-31j` or `cp932`, `shift_jis`, `shift-jis`, or `sjis`, `gbk` or `cp936`,
`euc-kr` or `cp949`, `big5` or `cp950`, `euc-jp`, `gb18030`, and `iso-2022-jp` or `iso2022-jp`.

txBASE normalizes the label and reports the effective name in `schema` output as `encoding_override`.
The same effective name is available in `encoding_metadata.effective`.
That object also reports the DBF declaration and whether interpretation came from the language
driver, an explicit override, or the existing fallback.

The path-oriented commands also accept `--encoding NAME` for a per-invocation override.
That value takes precedence over the sidecar value, is used for reads and writes during that
invocation, and is not persisted to the DBF header or metadata sidecar.

Unknown root or field properties are rejected.

An unknown field name, unsupported format, or unsupported version is rejected when the table is loaded.

## 2. Current constraint slice

Versions 1 and 2 accept the following field properties:

| Property | Behavior |
| --- | --- |
| `primary` | Implies `unique` and `not_null`; only one single-field primary key is accepted |
| `unique` | Rejects a non-null value already used by another active record |
| `not_null` | Rejects JSON `null` on insert, replace, patch, or recall |
| `default` | Supplies a scalar value when the field is omitted from an insert |
| `references` | Declares a catalog-scoped `TABLE.FIELD` foreign-key target whose parent field must be declared primary or unique |
| `constraint_name` | Version 2 only; names this scalar foreign key for runtime mode changes |
| `deferrable` | Version 2 only; allows a scalar foreign key to switch between immediate and deferred mode |
| `deferred` | Sets a foreign key's initial mode; `true` implies `deferrable`, and defaults to `false` |
| `on_delete` | Selects `restrict` (the default), `no_action`, `cascade`, or `set_null` for a scalar reference |
| `on_update` | Selects `restrict` (the default), `no_action`, `cascade`, or `set_null` for a scalar reference |

The optional root `checks` array rejects a candidate record unless every table-level query
predicate matches.

The optional root `constraints` object accepts bounded composite keys:

| Property | Behavior |
| --- | --- |
| `primary` | Requires two or more field names; all values must be non-null and the tuple must be unique |
| `unique` | Accepts arrays of two or more field names; a non-null tuple may not repeat among active records |
| `foreign_keys` | Accepts composite child and parent field lists with equal length; version 2 also accepts `name` and `deferrable` |
| `deferrable` | Version 2 only; declares named local `unique`, `primary_key`, or `check` constraints |

Version 2 local constraints use this shape:

```json
{
  "format": "txbase-schema",
  "version": 2,
  "fields": {
    "MANAGER_ID": {
      "references": "users.ID",
      "constraint_name": "users_manager_fk",
      "deferrable": true
    }
  },
  "constraints": {
    "deferrable": [
      {"name": "users_name_unique", "kind": "unique", "fields": ["NAME"]},
      {
        "name": "users_age_nonnegative",
        "kind": "check",
        "predicate": {"AGE": {"$gte": 0}},
        "deferred": true
      }
    ]
  }
}
```

Each local entry requires a unique `name` and a `kind`.
`unique` and `primary_key` require `fields`; `check` requires a `predicate`.
The optional `deferred` flag defaults to `false`, so these constraints begin each transaction in immediate mode unless set to `true`.
Constraint names must be unique within a table, including names generated for unnamed foreign keys.
Unnamed foreign keys receive `fk_<index>` in sorted field-name order, followed by `constraints.foreign_keys` order; use explicit names when an identifier must remain stable as the schema changes.

Every scalar reference must target a field marked `primary` or `unique` in the parent table's schema sidecar.

A composite reference must target the same field set as one `constraints.primary` or `constraints.unique` entry in that sidecar.

The declaration order may differ from the reference order, but child and parent fields still pair positionally when their values are compared.

Current data that happens to be unique is insufficient, and the `.txidx` sidecar does not declare a unique key.

The sidecar's `encoding` property is not a field constraint.

It selects the codec used for character reads and writes before the record enters the JSON layer.

The same eight codecs can be selected temporarily by the public
`DbfTable::from_path_with_encoding` API or the CLI `--encoding` option.

It does not add collation or an automatic conversion policy.

Non-deferrable field constraints and the root `checks` array run before the in-memory record is changed.
Named deferrable constraints run after each operation in immediate mode and are checked against the complete transaction image at commit when deferred.
All constraints are checked before a commit is published.

Deferrable `primary_key` uniqueness can be postponed, but its non-null requirement remains immediate.
The named `check` kind is a txBASE extension; PostgreSQL does not defer `CHECK` or `NOT NULL` constraints.

Scalar `default` values are applied only to fields omitted from an insert. An explicit JSON `null`
is not replaced, and replace, patch, and recall use the candidate values they already produce.
Defaults are applied before `not_null`, uniqueness, and `checks` validation.

Composite unique constraints use the existing scalar comparison rules. A tuple containing a null
value does not participate in uniqueness, matching the existing single-field unique behavior.

`references` is resolved by the directory catalog after each named-table mutation statement and at catalog transaction commit.
Before comparing values, the catalog rejects a target that is not declared primary or unique in the parent schema sidecar.
Non-null child values must match an active record in the referenced table; null values are allowed.

`on_delete` and `on_update` apply to scalar references, default to `restrict`, and accept
`no_action`, `cascade`, or `set_null`. `restrict` rejects a parent logical delete or key update
immediately, even when the foreign-key check is deferred. `no_action` does not modify child rows;
a deferred foreign key may remain temporarily unmatched until commit. `cascade` applies the
corresponding delete or key update to matching children immediately. `set_null` writes JSON `null`
to every local key field and therefore requires those fields not to be `not_null` or part of a
primary key.

`constraints.foreign_keys` declares a composite reference with a `fields` child list and a
`references` object containing `table` and `fields` parent values. Version 2 also accepts an optional
`name` and `deferrable`; both schema versions accept `deferred`, `on_delete`, and `on_update`.

The two field lists must contain at least two distinct names and have the same length.

If any child value is null, the composite reference is not checked; otherwise every child value
must match the corresponding value in one active parent record.

Foreign-key checks are immediate by default. A deferrable scalar or composite foreign key can start
in either mode and can change mode by name in a Rust transaction. Deferred checks run against the
complete catalog image before the journal commit. This permits a child to be inserted before its
parent, or a `no_action` parent change to be repaired by a later operation in the same transaction.
A failed final check publishes no table or sidecar changes. A standalone mutation has one operation
and therefore still has to satisfy every foreign key before it commits.

`DbfTransaction::set_constraints` changes local named constraints, and
`DbfTransaction::set_all_constraints` changes every deferrable local constraint. The catalog
counterparts accept a table name and can also change every deferrable constraint in the catalog.
Changing a mode to immediate validates the current transaction image before the mode changes.
The HTTP `POST /transaction` routes accept ordered `setConstraints` commands among mutations.
Named catalog selections require a table; `all: true` can target one table or the full catalog.
The request remains atomic, so a failed step publishes no changes.

`no_action` may be deferred, while `restrict` is always immediate. `cascade` and `set_null` actions
run as each operation is applied. This follows the timing distinction in PostgreSQL's foreign-key
contract, but txBASE does not claim PostgreSQL compatibility.

Catalog cascades are applied recursively inside the same catalog transaction and journal commit.
An action that violates a local schema constraint, or a cascade graph that does not converge, is
rejected without publishing any table.

Direct single-table operations cannot resolve cross-table references and therefore do not enforce
these actions.

Existing active records are checked when the sidecar is loaded, so an invalid pre-existing DBF is not silently accepted as a valid constrained table.

Deleted records do not participate in uniqueness checks.

`RECALL` checks the record against currently active records before removing its deletion marker.

Each `checks` entry is a query predicate object using the same bounded comparison, membership,
logical, and `$expr` rules as `filter`. Checks are validated when the sidecar is loaded and are
evaluated for active records and every insert, replace, patch, and recall candidate.

The current implementation compares scalar numeric values by numeric order, while strings and booleans use their JSON value semantics.

## 3. Persistence boundary

The DBF remains the source of field bytes.

The metadata sidecar is loaded with the DBF and is included in the path-loaded table's stale-source check.

If the sidecar appears, disappears, or changes after a table was loaded, a save is rejected rather than applying a mutation under a different constraint set.

The sidecar is not rewritten by a record mutation.

`txbase schema apply` acquires the same table lock used by DBF readers and writers, recovers pending local WAL or XBF export work, validates the candidate against the current table, and replaces only the metadata sidecar through a synced temporary file.

The DBF bytes, index sidecar, transaction-state sidecar, and MVCC history are unchanged by this metadata-only operation.

If candidate validation fails, the existing sidecar and DBF remain unchanged.

`backup` and `restore` copy it as `*.txschema.json`, and remove an old destination metadata sidecar when the source has none.

The DBF and metadata files are still separate files.

The schema edit is atomic for the metadata sidecar, but it is not a DBF layout migration and does not provide a multi-file transaction for unrelated files.

## 4. Deliberate non-goals

The sidecar does not yet implement:

- Collations or type declarations independent of DBF field descriptors.
- DBF layout migrations or schema versions beyond the current sidecar format.
- Automatic selection between a DBF language driver and an override.

References across catalog roots and DBF layout migrations remain future work.

SQLite's official [`CREATE TABLE` reference](https://sqlite.org/lang_createtable.html) distinguishes `NOT NULL`, `CHECK`, `UNIQUE`, `PRIMARY KEY`, and `FOREIGN KEY` constraints and documents their write-time behavior.

Its [foreign-key reference](https://www.sqlite.org/foreignkeys.html) requires the parent fields to be a primary key or collectively unique.
txBASE applies that target-key invariant through schema metadata and does not treat standalone index sidecars as unique-key declarations.

txBASE uses those distinctions as design references, but does not claim SQLite compatibility.

## Primary references

- [SQLite `CREATE TABLE`](https://sqlite.org/lang_createtable.html)
- [SQLite foreign-key support](https://www.sqlite.org/foreignkeys.html)
- [PostgreSQL `SET CONSTRAINTS`](https://www.postgresql.org/docs/18/sql-set-constraints.html)
- [PostgreSQL `CREATE TABLE` constraint deferrability](https://www.postgresql.org/docs/18/sql-createtable.html)
- [DBF compatibility boundary](dbf-compatibility.md)
- [Roadmap and explicit non-goals](roadmap.md)
