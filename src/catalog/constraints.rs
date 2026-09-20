use super::{Catalog, CatalogError};
use crate::dbf::DbfTable;
use crate::json_order::compare_scalar_values;
use serde_json::Value;
use std::collections::BTreeMap;

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
    validate_foreign_keys(&tables)
}

fn validate_foreign_keys(tables: &BTreeMap<String, DbfTable>) -> Result<(), CatalogError> {
    for (child_name, child) in tables {
        let foreign_keys = child.foreign_keys().map_err(|source| CatalogError::Table {
            name: child_name.clone(),
            source,
        })?;
        for (local_field, parent_name, parent_field) in foreign_keys {
            let parent = tables.get(&parent_name).ok_or_else(|| {
                CatalogError::Invalid(format!(
                    "table {child_name} references missing table {parent_name}"
                ))
            })?;
            if !parent
                .fields
                .iter()
                .any(|field| !field.is_system() && field.name == parent_field)
            {
                return Err(CatalogError::Invalid(format!(
                    "table {child_name} references unknown field {parent_name}.{parent_field}"
                )));
            }
            for record in child.active_records() {
                let value = record.values.get(&local_field).unwrap_or(&Value::Null);
                if value.is_null() {
                    continue;
                }
                let found = parent.active_records().any(|candidate| {
                    let parent_value = candidate.values.get(&parent_field).unwrap_or(&Value::Null);
                    compare_scalar_values(value, parent_value)
                        .is_some_and(|ordering| ordering.is_eq())
                        || value == parent_value
                });
                if !found {
                    return Err(CatalogError::Invalid(format!(
                        "table {child_name} foreign key {local_field} has no matching {parent_name}.{parent_field}"
                    )));
                }
            }
        }
    }
    Ok(())
}
