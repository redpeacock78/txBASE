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

The current sidecar is versioned independently from DBF:

```json
{
  "format": "txbase-schema",
  "version": 1,
  "fields": {
    "ID": {"primary": true},
    "NAME": {"unique": true, "not_null": true}
  }
}
```

The optional root `encoding` property is an explicit override for the four declared multibyte codecs
plus EUC-JP and GB18030.

Accepted labels are `windows-31j` or `cp932`, `gbk` or `cp936`, `euc-kr` or `cp949`, `big5` or
`cp950`, `euc-jp`, and `gb18030`.

txBASE normalizes the label and reports the effective name in `schema` output as `encoding_override`.

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

The sidecar's `encoding` property is not a field constraint.

It selects the codec used for character reads and writes before the record enters the JSON layer.

The same six codecs can be selected temporarily by the public
`DbfTable::from_path_with_encoding` API or the CLI `--encoding` option.

It does not add strict Shift_JIS, collation, or an automatic conversion policy.

Constraint checks run before the in-memory record is changed.

Existing active records are checked when the sidecar is loaded, so an invalid pre-existing DBF is not silently accepted as a valid constrained table.

Deleted records do not participate in uniqueness checks.

`RECALL` checks the record against currently active records before removing its deletion marker.

The current implementation compares scalar numeric values by numeric order, while strings and booleans use their JSON value semantics.

## 3. Persistence boundary

The DBF remains the source of field bytes.

The metadata sidecar is loaded with the DBF and is included in the path-loaded table's stale-source check.

If the sidecar appears, disappears, or changes after a table was loaded, a save is rejected rather than applying a mutation under a different constraint set.

The sidecar is not rewritten by a record mutation.

`backup` and `restore` copy it as `*.txschema.json`, and remove an old destination metadata sidecar when the source has none.

The DBF and metadata files are still separate files.

The current copy and WAL protocols do not claim one atomic multi-file commit for a metadata edit.

## 4. Deliberate non-goals

The sidecar does not yet implement:

- Composite primary or unique keys.
- `CHECK` expressions.
- `FOREIGN KEY` references or cross-table validation.
- `DEFAULT` expressions or generated values.
- Collations or type declarations independent of DBF field descriptors.
- A schema migration or metadata-edit command.
- Automatic selection between a DBF language driver and an override.

These need an expression grammar, cross-table visibility, migration and recovery rules before they can be added safely.

SQLite's official [`CREATE TABLE` reference](https://sqlite.org/lang_createtable.html) distinguishes `NOT NULL`, `CHECK`, `UNIQUE`, `PRIMARY KEY`, and `FOREIGN KEY` constraints and documents their write-time behavior.

Its [foreign-key reference](https://www.sqlite.org/foreignkeys.html) also makes the referenced-table existence contract explicit.

txBASE uses those distinctions as design references, but does not claim SQLite compatibility.

## Primary references

- [SQLite `CREATE TABLE`](https://sqlite.org/lang_createtable.html)
- [SQLite foreign-key support](https://www.sqlite.org/foreignkeys.html)
- [DBF compatibility boundary](dbf-compatibility.md)
- [Roadmap and explicit non-goals](roadmap.md)
