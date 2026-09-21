use super::CatalogError;
use crate::dbf::{DbfRecord, DbfTable, ForeignKey, ForeignKeyAction};
use crate::json_order::compare_scalar_values;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn apply_actions(
    before: &BTreeMap<String, DbfTable>,
    tables: &mut BTreeMap<String, DbfTable>,
) -> Result<BTreeSet<String>, CatalogError> {
    let relationship_count = tables
        .values()
        .map(|table| table.foreign_keys())
        .collect::<Result<Vec<_>, _>>()
        .map(|keys| keys.into_iter().map(|keys| keys.len()).sum::<usize>())
        .map_err(|source| CatalogError::Invalid(format!("foreign-key metadata: {source}")))?;
    let total_rows = tables
        .values()
        .map(|table| table.records().len())
        .sum::<usize>();
    let max_passes = total_rows
        .saturating_add(1)
        .saturating_mul(relationship_count.max(1))
        .max(1);
    let mut changed_tables = BTreeSet::new();

    // ponytail: bounded full-table cascade scan; add relationship indexes if catalog scale requires it.
    for _ in 0..max_passes {
        let mut pass_changed = false;
        let child_names = tables.keys().cloned().collect::<Vec<_>>();
        for child_name in child_names {
            let foreign_keys = tables
                .get(&child_name)
                .expect("child table exists")
                .foreign_keys()
                .map_err(|source| CatalogError::Table {
                    name: child_name.clone(),
                    source,
                })?;
            for foreign_key in foreign_keys {
                let Some(before_parent) = before.get(&foreign_key.parent_table) else {
                    continue;
                };
                let Some(current_parent) = tables.get(&foreign_key.parent_table) else {
                    continue;
                };
                let changes =
                    parent_changes(before_parent, current_parent, &foreign_key.parent_fields);
                for change in changes {
                    let action = if change.new.is_some() {
                        &foreign_key.on_update
                    } else {
                        &foreign_key.on_delete
                    };
                    let Some(child) = tables.get_mut(&child_name) else {
                        continue;
                    };
                    if apply_action(child, &child_name, &foreign_key, &change, action)? {
                        pass_changed = true;
                        changed_tables.insert(child_name.clone());
                    }
                }
            }
        }
        if !pass_changed {
            return Ok(changed_tables);
        }
    }

    Err(CatalogError::Invalid(
        "foreign-key cascade did not converge".into(),
    ))
}

#[derive(Debug, Clone, PartialEq)]
struct ParentChange {
    old: Vec<Value>,
    new: Option<Vec<Value>>,
}

fn parent_changes(
    before: &DbfTable,
    current: &DbfTable,
    parent_fields: &[String],
) -> Vec<ParentChange> {
    before
        .records()
        .iter()
        .enumerate()
        .filter_map(|(index, old)| {
            if old.deleted {
                return None;
            }
            let old_values = tuple_values(old, parent_fields)?;
            let new = current.records().get(index);
            if new.is_none_or(|record| record.deleted) {
                return Some(ParentChange {
                    old: old_values,
                    new: None,
                });
            }
            let new_values = tuple_values(new.expect("checked above"), parent_fields)?;
            (!tuples_equal(&old_values, &new_values)).then_some(ParentChange {
                old: old_values,
                new: Some(new_values),
            })
        })
        .collect()
}

fn apply_action(
    child: &mut DbfTable,
    child_name: &str,
    foreign_key: &ForeignKey,
    change: &ParentChange,
    action: &ForeignKeyAction,
) -> Result<bool, CatalogError> {
    if matches!(action, ForeignKeyAction::Restrict) {
        return Ok(false);
    }
    let matching_records = child
        .active_records()
        .filter(|record| {
            tuple_values(record, &foreign_key.local_fields)
                .is_some_and(|values| tuples_equal(&values, &change.old))
        })
        .map(|record| record.number)
        .collect::<Vec<_>>();
    if matching_records.is_empty() {
        return Ok(false);
    }

    for record_number in &matching_records {
        match action {
            ForeignKeyAction::Cascade => {
                if let Some(new) = &change.new {
                    child
                        .patch_record(*record_number, fields_patch(&foreign_key.local_fields, new))
                        .map_err(|source| CatalogError::Table {
                            name: child_name.to_owned(),
                            source,
                        })?;
                } else {
                    child
                        .delete_record(*record_number)
                        .map_err(|source| CatalogError::Table {
                            name: child_name.to_owned(),
                            source,
                        })?;
                }
            }
            ForeignKeyAction::SetNull => {
                child
                    .patch_record(
                        *record_number,
                        fields_patch(
                            &foreign_key.local_fields,
                            &vec![Value::Null; foreign_key.local_fields.len()],
                        ),
                    )
                    .map_err(|source| CatalogError::Table {
                        name: child_name.to_owned(),
                        source,
                    })?;
            }
            ForeignKeyAction::Restrict => unreachable!("restrict returned above"),
        }
    }
    Ok(true)
}

fn tuple_values(record: &DbfRecord, fields: &[String]) -> Option<Vec<Value>> {
    fields
        .iter()
        .map(|field| record.values.get(field).cloned())
        .collect()
}

fn fields_patch(fields: &[String], values: &[Value]) -> Map<String, Value> {
    fields.iter().cloned().zip(values.iter().cloned()).collect()
}

fn tuples_equal(left: &[Value], right: &[Value]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| values_equal(left, right))
}

fn values_equal(left: &Value, right: &Value) -> bool {
    compare_scalar_values(left, right).is_some_and(|ordering| ordering.is_eq()) || left == right
}
