use super::{Catalog, CatalogError};
use crate::dbf::DbfTable;
use crate::json_order::compare_scalar_values;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn validate_replacements(
    catalog: &Catalog,
    replacements: &BTreeMap<String, DbfTable>,
) -> Result<(), CatalogError> {
    let mut tables = BTreeMap::new();
    // ponytail: reload the bounded catalog under its existing lock; add relationship indexes if it grows.
    for entry in catalog.tables() {
        let table = match replacements.get(entry.name()) {
            Some(table) => table.clone(),
            None => catalog.open_table_unlocked(entry.name())?,
        };
        tables.insert(entry.name().to_owned(), table);
    }
    validate_loaded_tables(&tables)
}

pub(super) fn validate_loaded_tables(
    tables: &BTreeMap<String, DbfTable>,
) -> Result<(), CatalogError> {
    validate_loaded_tables_with_deferred(tables, true, None)
}

pub(super) fn validate_statement_constraints(
    tables: &BTreeMap<String, DbfTable>,
    changed_tables: &BTreeSet<String>,
) -> Result<(), CatalogError> {
    validate_loaded_tables_with_deferred(tables, false, Some(changed_tables))
}

fn validate_loaded_tables_with_deferred(
    tables: &BTreeMap<String, DbfTable>,
    include_deferred: bool,
    changed_tables: Option<&BTreeSet<String>>,
) -> Result<(), CatalogError> {
    for (child_name, child) in tables {
        let foreign_keys = child.foreign_keys().map_err(|source| CatalogError::Table {
            name: child_name.clone(),
            source,
        })?;
        for foreign_key in foreign_keys {
            let affected = changed_tables.is_none_or(|changed| {
                changed.contains(child_name) || changed.contains(&foreign_key.parent_table)
            });
            if !affected {
                continue;
            }
            let parent = tables.get(&foreign_key.parent_table).ok_or_else(|| {
                CatalogError::Invalid(format!(
                    "table {child_name} references missing table {}",
                    foreign_key.parent_table
                ))
            })?;
            for parent_field in &foreign_key.parent_fields {
                if !parent
                    .fields
                    .iter()
                    .any(|field| !field.is_system() && field.name == *parent_field)
                {
                    return Err(CatalogError::Invalid(format!(
                        "table {child_name} references unknown field {}.{}",
                        foreign_key.parent_table, parent_field
                    )));
                }
            }
            if foreign_key.deferred && !include_deferred {
                continue;
            }
            for record in child.active_records() {
                let values = foreign_key
                    .local_fields
                    .iter()
                    .map(|field| record.values.get(field).unwrap_or(&Value::Null))
                    .collect::<Vec<_>>();
                if values.iter().any(|value| value.is_null()) {
                    continue;
                }
                let found = parent.active_records().any(|candidate| {
                    foreign_key.parent_fields.iter().zip(values.iter()).all(
                        |(parent_field, value)| {
                            let parent_value =
                                candidate.values.get(parent_field).unwrap_or(&Value::Null);
                            values_equal(value, parent_value)
                        },
                    )
                });
                if !found {
                    let local_fields = format_fields(&foreign_key.local_fields);
                    let parent_fields = format_fields(&foreign_key.parent_fields);
                    return Err(CatalogError::Invalid(format!(
                        "table {child_name} foreign key {local_fields} has no matching {}.{parent_fields}",
                        foreign_key.parent_table
                    )));
                }
            }
        }
    }
    Ok(())
}

fn format_fields(fields: &[String]) -> String {
    if fields.len() == 1 {
        fields[0].clone()
    } else {
        format!("({})", fields.join(", "))
    }
}

fn values_equal(left: &Value, right: &Value) -> bool {
    compare_scalar_values(left, right).is_some_and(|ordering| ordering.is_eq()) || left == right
}
