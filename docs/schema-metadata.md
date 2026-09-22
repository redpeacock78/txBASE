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
        }
      }
    ]
  ]
}
```

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

The current version accepts the following field properties:

| Property | Behavior |
| --- | --- |
| `primary` | Implies `unique` and `not_null`; only one single-field primary key is accepted |
| `unique` | Rejects a non-null value already used by another active record |
| `not_null` | Rejects JSON `null` on insert, replace, patch, or recall |
| `default` | Supplies a scalar value when the field is omitted from an insert |
| `references` | Declares a catalog-scoped `TABLE.FIELD` foreign-key target |
| `on_delete` | Selects `restrict` (the default), `cascade`, or `set_null` for a scalar reference |
| `on_update` | Selects `restrict` (the default), `cascade`, or `set_null` for a scalar reference |

The optional root `checks` array rejects a candidate record unless every table-level query
predicate matches.

The optional root `constraints` object accepts bounded composite keys:

| Property | Behavior |
| --- | --- |
| `primary` | Requires two or more field names; all values must be non-null and the tuple must be unique |
| `unique` | Accepts arrays of two or more field names; a non-null tuple may not repeat among active records |
| `foreign_keys` | Accepts composite child and parent field lists with equal length and optional update/delete actions |

The sidecar's `encoding` property is not a field constraint.

It selects the codec used for character reads and writes before the record enters the JSON layer.

The same eight codecs can be selected temporarily by the public
`DbfTable::from_path_with_encoding` API or the CLI `--encoding` option.

It does not add collation or an automatic conversion policy.

Constraint checks run before the in-memory record is changed.

Scalar `default` values are applied only to fields omitted from an insert. An explicit JSON `null`
is not replaced, and replace, patch, and recall use the candidate values they already produce.
Defaults are applied before `not_null`, uniqueness, and `checks` validation.

Composite unique constraints use the existing scalar comparison rules. A tuple containing a null
value does not participate in uniqueness, matching the existing single-field unique behavior.

`references` is resolved by the directory catalog after a named-table mutation or catalog
transaction. Non-null child values must match an active record in the referenced table; null
values are allowed.

`on_delete` and `on_update` apply to scalar references, default to `restrict`, and accept
`cascade` or `set_null`. `restrict` rejects a parent logical delete or key update that would
leave a child dangling. `cascade` applies the corresponding delete or key update to matching
children. `set_null` writes JSON `null` to every local key field and therefore requires those
fields not to be `not_null` or part of a primary key.

`constraints.foreign_keys` declares a composite reference with a `fields` child list and a
`references` object containing `table` and `fields` parent values. It accepts the same optional
`on_delete` and `on_update` actions.

The two field lists must contain at least two distinct names and have the same length.

If any child value is null, the composite reference is not checked; otherwise every child value
must match the corresponding value in one active parent record.

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

Additional cross-table constraint semantics need broader metadata, migration, and recovery rules before they can be added safely.

SQLite's official [`CREATE TABLE` reference](https://sqlite.org/lang_createtable.html) distinguishes `NOT NULL`, `CHECK`, `UNIQUE`, `PRIMARY KEY`, and `FOREIGN KEY` constraints and documents their write-time behavior.

Its [foreign-key reference](https://www.sqlite.org/foreignkeys.html) also makes the referenced-table existence contract explicit.

txBASE uses those distinctions as design references, but does not claim SQLite compatibility.

## Primary references

- [SQLite `CREATE TABLE`](https://sqlite.org/lang_createtable.html)
- [SQLite foreign-key support](https://www.sqlite.org/foreignkeys.html)
- [DBF compatibility boundary](dbf-compatibility.md)
- [Roadmap and explicit non-goals](roadmap.md)
