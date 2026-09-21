use super::{CompositeForeignKeyMetadata, SchemaMetadata};
use crate::dbf::types::ForeignKey;
use crate::dbf::{DbfError, DbfRecord, FieldDescriptor};
use crate::json_order::compare_scalar_values;
use crate::query::matches_filter;
use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::collections::BTreeSet;

impl SchemaMetadata {
    pub(in crate::dbf) fn validate_fields(
        &self,
        fields: &[FieldDescriptor],
    ) -> Result<(), DbfError> {
        let mut field_primary = false;
        for (name, metadata) in &self.fields {
            if !fields
                .iter()
                .any(|field| !field.is_system() && field.name == *name)
            {
                return Err(DbfError::Invalid(format!(
                    "schema metadata refers to unknown field {name}"
                )));
            }
            if let Some(default) = &metadata.default {
                if default.is_array() || default.is_object() {
                    return Err(DbfError::Invalid(format!(
                        "schema default for field {name} must be a scalar"
                    )));
                }
            }
            if metadata.primary && field_primary {
                return Err(DbfError::Invalid(
                    "composite primary keys are not supported by this schema version".into(),
                ));
            }
            field_primary |= metadata.primary;
        }
        self.foreign_keys()?;
        for (index, key) in self.constraints.foreign_keys.iter().enumerate() {
            validate_composite_foreign_key(
                key,
                fields,
                &format!("constraints.foreign_keys[{index}]"),
            )?;
        }
        if !self.constraints.primary.is_empty() {
            if field_primary {
                return Err(DbfError::Invalid(
                    "schema primary constraints cannot mix field and composite definitions".into(),
                ));
            }
            validate_key_fields(&self.constraints.primary, fields, "constraints.primary", 2)?;
        }
        for (index, key) in self.constraints.unique.iter().enumerate() {
            validate_key_fields(key, fields, &format!("constraints.unique[{index}]"), 2)?;
        }
        Ok(())
    }

    pub(in crate::dbf) fn foreign_keys(&self) -> Result<Vec<ForeignKey>, DbfError> {
        let mut foreign_keys = self
            .fields
            .iter()
            .filter_map(|(local_field, metadata)| {
                metadata
                    .references
                    .as_deref()
                    .map(|reference| (local_field, reference))
            })
            .map(|(local_field, reference)| {
                let mut parts = reference.split('.');
                let table = parts.next().unwrap_or_default();
                let field = parts.next().unwrap_or_default();
                if table.is_empty() || field.is_empty() || parts.next().is_some() {
                    return Err(DbfError::Invalid(format!(
                        "schema reference for field {local_field} must be TABLE.FIELD"
                    )));
                }
                Ok(ForeignKey {
                    local_fields: vec![local_field.clone()],
                    parent_table: table.to_owned(),
                    parent_fields: vec![field.to_owned()],
                })
            })
            .collect::<Result<Vec<_>, DbfError>>()?;
        for key in &self.constraints.foreign_keys {
            if key.references.table.is_empty() {
                return Err(DbfError::Invalid(
                    "composite foreign-key reference table must not be empty".into(),
                ));
            }
            if key.references.fields.is_empty() {
                return Err(DbfError::Invalid(
                    "composite foreign-key reference fields must not be empty".into(),
                ));
            }
            foreign_keys.push(ForeignKey {
                local_fields: key.fields.clone(),
                parent_table: key.references.table.clone(),
                parent_fields: key.references.fields.clone(),
            });
        }
        Ok(foreign_keys)
    }

    pub(in crate::dbf) fn apply_defaults(&self, values: &mut Map<String, Value>) {
        for (name, metadata) in &self.fields {
            if let Some(default) = &metadata.default {
                values
                    .entry(name.clone())
                    .or_insert_with(|| default.clone());
            }
        }
    }

    pub(in crate::dbf) fn validate_records(&self, records: &[DbfRecord]) -> Result<(), DbfError> {
        for (index, record) in records.iter().enumerate() {
            if !record.deleted {
                self.validate_values(&record.values, records, Some(index))?;
            }
        }
        Ok(())
    }

    pub(in crate::dbf) fn validate_candidate(
        &self,
        values: &Map<String, Value>,
        records: &[DbfRecord],
        excluded_index: Option<usize>,
    ) -> Result<(), DbfError> {
        self.validate_values(values, records, excluded_index)
    }

    fn validate_values(
        &self,
        values: &Map<String, Value>,
        records: &[DbfRecord],
        excluded_index: Option<usize>,
    ) -> Result<(), DbfError> {
        for (index, check) in self.checks.iter().enumerate() {
            let matches = matches_filter(values, check).map_err(|error| {
                DbfError::Invalid(format!(
                    "schema check {index} could not be evaluated: {error}"
                ))
            })?;
            if !matches {
                return Err(DbfError::Invalid(format!(
                    "constraint violation: check {index} failed"
                )));
            }
        }
        for (name, metadata) in &self.fields {
            let value = values.get(name).unwrap_or(&Value::Null);
            if (metadata.not_null || metadata.primary) && value.is_null() {
                return Err(DbfError::Invalid(format!(
                    "constraint violation: field {name} must not be null"
                )));
            }
            if !(metadata.unique || metadata.primary) || value.is_null() {
                continue;
            }
            for (index, record) in records.iter().enumerate() {
                if record.deleted || excluded_index == Some(index) {
                    continue;
                }
                let other = record.values.get(name).unwrap_or(&Value::Null);
                if !other.is_null() && values_equal(value, other) {
                    return Err(DbfError::Invalid(format!(
                        "constraint violation: duplicate value for field {name}"
                    )));
                }
            }
        }
        for name in &self.constraints.primary {
            let value = values.get(name).unwrap_or(&Value::Null);
            if value.is_null() {
                return Err(DbfError::Invalid(format!(
                    "constraint violation: field {name} must not be null"
                )));
            }
        }
        if !self.constraints.primary.is_empty() {
            validate_composite_unique(&self.constraints.primary, values, records, excluded_index)?;
        }
        for key in &self.constraints.unique {
            validate_composite_unique(key, values, records, excluded_index)?;
        }
        Ok(())
    }
}

fn validate_key_fields(
    key: &[String],
    fields: &[FieldDescriptor],
    path: &str,
    minimum: usize,
) -> Result<(), DbfError> {
    if key.len() < minimum {
        return Err(DbfError::Invalid(format!(
            "{path} must contain at least {minimum} fields"
        )));
    }
    let mut seen = BTreeSet::new();
    for name in key {
        if !seen.insert(name) {
            return Err(DbfError::Invalid(format!(
                "{path} contains duplicate field {name}"
            )));
        }
        if !fields
            .iter()
            .any(|field| !field.is_system() && field.name == *name)
        {
            return Err(DbfError::Invalid(format!(
                "schema metadata refers to unknown field {name}"
            )));
        }
    }
    Ok(())
}

fn validate_composite_foreign_key(
    key: &CompositeForeignKeyMetadata,
    fields: &[FieldDescriptor],
    path: &str,
) -> Result<(), DbfError> {
    validate_key_fields(&key.fields, fields, &format!("{path}.fields"), 2)?;
    validate_name_list(
        &key.references.fields,
        &format!("{path}.references.fields"),
        2,
    )?;
    if key.references.table.is_empty() {
        return Err(DbfError::Invalid(format!(
            "{path}.references.table must not be empty"
        )));
    }
    if key.fields.len() != key.references.fields.len() {
        return Err(DbfError::Invalid(format!(
            "{path}.fields and {path}.references.fields must have the same length"
        )));
    }
    Ok(())
}

fn validate_name_list(names: &[String], path: &str, minimum: usize) -> Result<(), DbfError> {
    if names.len() < minimum {
        return Err(DbfError::Invalid(format!(
            "{path} must contain at least {minimum} fields"
        )));
    }
    let mut seen = BTreeSet::new();
    for name in names {
        if name.is_empty() {
            return Err(DbfError::Invalid(format!(
                "{path} contains an empty field name"
            )));
        }
        if !seen.insert(name) {
            return Err(DbfError::Invalid(format!(
                "{path} contains duplicate field {name}"
            )));
        }
    }
    Ok(())
}

fn validate_composite_unique(
    fields: &[String],
    values: &Map<String, Value>,
    records: &[DbfRecord],
    excluded_index: Option<usize>,
) -> Result<(), DbfError> {
    if fields
        .iter()
        .any(|field| values.get(field).unwrap_or(&Value::Null).is_null())
    {
        return Ok(());
    }
    // ponytail: scan active records for bounded local constraints; add a tuple index if this grows.
    for (index, record) in records.iter().enumerate() {
        if record.deleted || excluded_index == Some(index) {
            continue;
        }
        let matches = fields.iter().all(|field| {
            let value = values.get(field).unwrap_or(&Value::Null);
            let other = record.values.get(field).unwrap_or(&Value::Null);
            !other.is_null() && values_equal(value, other)
        });
        if matches {
            return Err(DbfError::Invalid(format!(
                "constraint violation: duplicate composite value for fields {}",
                fields.join(", ")
            )));
        }
    }
    Ok(())
}

fn values_equal(left: &Value, right: &Value) -> bool {
    match compare_scalar_values(left, right) {
        Some(ordering) => ordering == Ordering::Equal,
        None => left == right,
    }
}
