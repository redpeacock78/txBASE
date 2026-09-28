use super::{Catalog, CatalogError};
use crate::ConstraintMode;
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
    validate_loaded_tables_with_deferred(tables, None, None)
}

pub(super) fn initial_deferred_constraints(
    tables: &BTreeMap<String, DbfTable>,
) -> Result<BTreeMap<String, BTreeSet<String>>, CatalogError> {
    let mut deferred = BTreeMap::new();
    for (table_name, table) in tables {
        let modes = table
            .deferrable_constraint_modes()
            .map_err(|source| CatalogError::Table {
                name: table_name.clone(),
                source,
            })?;
        let names = modes
            .into_iter()
            .filter_map(|(name, mode)| (mode == ConstraintMode::Deferred).then_some(name))
            .collect::<BTreeSet<_>>();
        deferred.insert(table_name.clone(), names);
    }
    Ok(deferred)
}

pub(super) fn validate_statement_constraints(
    tables: &BTreeMap<String, DbfTable>,
    changed_tables: &BTreeSet<String>,
    deferred_constraints: &BTreeMap<String, BTreeSet<String>>,
) -> Result<(), CatalogError> {
    validate_loaded_tables_with_deferred(tables, Some(deferred_constraints), Some(changed_tables))
}

pub(super) fn validate_transaction_constraints(
    tables: &BTreeMap<String, DbfTable>,
    deferred_constraints: &BTreeMap<String, BTreeSet<String>>,
) -> Result<(), CatalogError> {
    validate_loaded_tables_with_deferred(tables, Some(deferred_constraints), None)
}

fn validate_loaded_tables_with_deferred(
    tables: &BTreeMap<String, DbfTable>,
    deferred_constraints: Option<&BTreeMap<String, BTreeSet<String>>>,
    changed_tables: Option<&BTreeSet<String>>,
) -> Result<(), CatalogError> {
    for (child_name, child) in tables {
        if changed_tables.is_none_or(|changed| changed.contains(child_name)) {
            child
                .validate_schema_constraints(
                    deferred_constraints
                        .and_then(|constraints| constraints.get(child_name))
                        .unwrap_or(&BTreeSet::new()),
                )
                .map_err(|source| CatalogError::Table {
                    name: child_name.clone(),
                    source,
                })?;
        }
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
            if !parent.has_unique_key(&foreign_key.parent_fields) {
                return Err(CatalogError::Invalid(format!(
                    "table {child_name} foreign key {} references parent key {} on table {}, which is not declared primary or unique",
                    format_fields(&foreign_key.local_fields),
                    format_fields(&foreign_key.parent_fields),
                    foreign_key.parent_table
                )));
            }
            let is_deferred = foreign_key.deferrable
                && deferred_constraints
                    .and_then(|constraints| constraints.get(child_name))
                    .is_some_and(|names| names.contains(&foreign_key.name));
            if is_deferred {
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
