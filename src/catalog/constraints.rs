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
        for foreign_key in foreign_keys {
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
