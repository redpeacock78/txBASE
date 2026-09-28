use super::{CompositeForeignKeyMetadata, DeferrableConstraintKind, SchemaMetadata};
use crate::ConstraintMode;
use crate::dbf::types::{ForeignKey, ForeignKeyAction};
use crate::dbf::{DbfError, DbfRecord, DbfTable, FieldDescriptor};
use crate::json_order::compare_scalar_values;
use crate::query::matches_filter;
use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

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
            if metadata.references.is_none()
                && (metadata.constraint_name.is_some()
                    || metadata.deferrable
                    || metadata.deferred
                    || metadata.on_delete.is_some()
                    || metadata.on_update.is_some())
            {
                return Err(DbfError::Invalid(format!(
                    "schema field {name} foreign-key options require references"
                )));
            }
            if let Some(action) = &metadata.on_delete {
                self.validate_action_fields(
                    action,
                    std::slice::from_ref(name),
                    fields,
                    &format!("fields.{name}.on_delete"),
                )?;
            }
            if let Some(action) = &metadata.on_update {
                self.validate_action_fields(
                    action,
                    std::slice::from_ref(name),
                    fields,
                    &format!("fields.{name}.on_update"),
                )?;
            }
        }
        self.foreign_keys()?;
        for (index, key) in self.constraints.foreign_keys.iter().enumerate() {
            validate_composite_foreign_key(
                key,
                fields,
                &format!("constraints.foreign_keys[{index}]"),
            )?;
            let path = format!("constraints.foreign_keys[{index}]");
            if let Some(action) = &key.on_delete {
                self.validate_action_fields(
                    action,
                    &key.fields,
                    fields,
                    &format!("{path}.on_delete"),
                )?;
            }
            if let Some(action) = &key.on_update {
                self.validate_action_fields(
                    action,
                    &key.fields,
                    fields,
                    &format!("{path}.on_update"),
                )?;
            }
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
        let mut constraint_names = BTreeSet::new();
        let mut primary_key_count =
            usize::from(field_primary || !self.constraints.primary.is_empty());
        for (index, constraint) in self.constraints.deferrable.iter().enumerate() {
            let path = format!("constraints.deferrable[{index}]");
            validate_constraint_name(&constraint.name, &path)?;
            if !constraint_names.insert(constraint.name.clone()) {
                return Err(DbfError::Invalid(format!(
                    "duplicate schema constraint name: {}",
                    constraint.name
                )));
            }
            match constraint.kind {
                DeferrableConstraintKind::Unique => {
                    validate_key_fields(&constraint.fields, fields, &format!("{path}.fields"), 1)?;
                    if constraint.predicate.is_some() {
                        return Err(DbfError::Invalid(format!(
                            "{path}.predicate is only valid for check constraints"
                        )));
                    }
                }
                DeferrableConstraintKind::PrimaryKey => {
                    validate_key_fields(&constraint.fields, fields, &format!("{path}.fields"), 1)?;
                    if constraint.predicate.is_some() {
                        return Err(DbfError::Invalid(format!(
                            "{path}.predicate is only valid for check constraints"
                        )));
                    }
                    primary_key_count += 1;
                }
                DeferrableConstraintKind::Check => {
                    if !constraint.fields.is_empty() {
                        return Err(DbfError::Invalid(format!(
                            "{path}.fields is only valid for key constraints"
                        )));
                    }
                    let predicate = constraint.predicate.as_ref().ok_or_else(|| {
                        DbfError::Invalid(format!("{path}.predicate is required"))
                    })?;
                    crate::query::validate_filter(predicate, &format!("{path}.predicate"))
                        .map_err(|error| {
                            DbfError::Invalid(format!("invalid {path}.predicate: {error}"))
                        })?;
                }
            }
        }
        if primary_key_count > 1 {
            return Err(DbfError::Invalid(
                "schema metadata may declare only one primary key".into(),
            ));
        }
        for foreign_key in self.foreign_keys()? {
            validate_constraint_name(&foreign_key.name, "foreign-key constraint")?;
            if !constraint_names.insert(foreign_key.name.clone()) {
                return Err(DbfError::Invalid(format!(
                    "duplicate schema constraint name: {}",
                    foreign_key.name
                )));
            }
        }
        Ok(())
    }

    pub(in crate::dbf) fn foreign_keys(&self) -> Result<Vec<ForeignKey>, DbfError> {
        let mut next_generated_name = 0;
        let mut foreign_keys = self
            .fields
            .iter()
            .filter_map(|(local_field, metadata)| {
                metadata
                    .references
                    .as_deref()
                    .map(|reference| (local_field, reference, metadata))
            })
            .map(|(local_field, reference, metadata)| {
                let generated_name = format!("fk_{next_generated_name}");
                next_generated_name += 1;
                let mut parts = reference.split('.');
                let table = parts.next().unwrap_or_default();
                let field = parts.next().unwrap_or_default();
                if table.is_empty() || field.is_empty() || parts.next().is_some() {
                    return Err(DbfError::Invalid(format!(
                        "schema reference for field {local_field} must be TABLE.FIELD"
                    )));
                }
                Ok(ForeignKey {
                    name: metadata.constraint_name.clone().unwrap_or(generated_name),
                    local_fields: vec![local_field.clone()],
                    parent_table: table.to_owned(),
                    parent_fields: vec![field.to_owned()],
                    on_delete: metadata.on_delete.unwrap_or_default(),
                    on_update: metadata.on_update.unwrap_or_default(),
                    deferrable: metadata.deferrable || metadata.deferred,
                    deferred: metadata.deferred,
                })
            })
            .collect::<Result<Vec<_>, DbfError>>()?;
        for key in &self.constraints.foreign_keys {
            let generated_name = format!("fk_{next_generated_name}");
            next_generated_name += 1;
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
                name: key.name.clone().unwrap_or(generated_name),
                local_fields: key.fields.clone(),
                parent_table: key.references.table.clone(),
                parent_fields: key.references.fields.clone(),
                on_delete: key.on_delete.unwrap_or_default(),
                on_update: key.on_update.unwrap_or_default(),
                deferrable: key.deferrable || key.deferred,
                deferred: key.deferred,
            });
        }
        Ok(foreign_keys)
    }

    pub(in crate::dbf) fn has_unique_key(&self, fields: &[String]) -> bool {
        if fields.len() == 1
            && self
                .fields
                .get(&fields[0])
                .is_some_and(|metadata| metadata.primary || metadata.unique)
        {
            return true;
        }

        let matches_key = |key: &[String]| {
            key.len() == fields.len() && key.iter().all(|field| fields.contains(field))
        };
        matches_key(&self.constraints.primary)
            || self.constraints.unique.iter().any(|key| matches_key(key))
            || self.constraints.deferrable.iter().any(|constraint| {
                matches!(
                    constraint.kind,
                    DeferrableConstraintKind::PrimaryKey | DeferrableConstraintKind::Unique
                ) && matches_key(&constraint.fields)
            })
    }

    pub(in crate::dbf) fn deferrable_constraint_modes(
        &self,
    ) -> Result<BTreeMap<String, ConstraintMode>, DbfError> {
        let mut modes = self.local_deferrable_constraint_modes();
        for foreign_key in self.foreign_keys()? {
            if foreign_key.deferrable {
                modes.insert(
                    foreign_key.name,
                    if foreign_key.deferred {
                        ConstraintMode::Deferred
                    } else {
                        ConstraintMode::Immediate
                    },
                );
            }
        }
        Ok(modes)
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
        self.validate_records_with_deferred(records, &BTreeSet::new())
    }

    pub(in crate::dbf) fn validate_records_with_deferred(
        &self,
        records: &[DbfRecord],
        deferred_constraints: &BTreeSet<String>,
    ) -> Result<(), DbfError> {
        for (index, record) in records.iter().enumerate() {
            if !record.deleted {
                self.validate_values(&record.values, records, Some(index), deferred_constraints)?;
            }
        }
        Ok(())
    }

    pub(in crate::dbf) fn validate_candidate_with_deferred(
        &self,
        values: &Map<String, Value>,
        records: &[DbfRecord],
        excluded_index: Option<usize>,
        deferred_constraints: &BTreeSet<String>,
    ) -> Result<(), DbfError> {
        self.validate_values(values, records, excluded_index, deferred_constraints)
    }

    fn validate_values(
        &self,
        values: &Map<String, Value>,
        records: &[DbfRecord],
        excluded_index: Option<usize>,
        deferred_constraints: &BTreeSet<String>,
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
        for constraint in &self.constraints.deferrable {
            if constraint.kind == DeferrableConstraintKind::PrimaryKey
                && constraint
                    .fields
                    .iter()
                    .any(|field| values.get(field).unwrap_or(&Value::Null).is_null())
            {
                return Err(DbfError::Invalid(format!(
                    "constraint violation: primary key {} contains null",
                    constraint.name
                )));
            }
            if deferred_constraints.contains(&constraint.name) {
                continue;
            }
            match constraint.kind {
                DeferrableConstraintKind::Unique | DeferrableConstraintKind::PrimaryKey => {
                    validate_named_unique(
                        &constraint.name,
                        &constraint.fields,
                        values,
                        records,
                        excluded_index,
                    )?;
                }
                DeferrableConstraintKind::Check => {
                    let predicate = constraint
                        .predicate
                        .as_ref()
                        .expect("check predicate is validated when metadata loads");
                    let matches = matches_filter(values, predicate).map_err(|error| {
                        DbfError::Invalid(format!(
                            "schema constraint {} could not be evaluated: {error}",
                            constraint.name
                        ))
                    })?;
                    if !matches {
                        return Err(DbfError::Invalid(format!(
                            "constraint violation: check {} failed",
                            constraint.name
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}

impl DbfTable {
    pub(crate) fn local_deferrable_constraint_modes(&self) -> BTreeMap<String, ConstraintMode> {
        self.schema
            .as_ref()
            .map(SchemaMetadata::local_deferrable_constraint_modes)
            .unwrap_or_default()
    }

    pub(crate) fn deferrable_constraint_modes(
        &self,
    ) -> Result<BTreeMap<String, ConstraintMode>, DbfError> {
        self.schema
            .as_ref()
            .map(SchemaMetadata::deferrable_constraint_modes)
            .transpose()
            .map(Option::unwrap_or_default)
    }

    pub(crate) fn validate_schema_constraints(
        &self,
        deferred_constraints: &BTreeSet<String>,
    ) -> Result<(), DbfError> {
        if let Some(schema) = &self.schema {
            schema.validate_records_with_deferred(&self.records, deferred_constraints)?;
        }
        Ok(())
    }
}

fn validate_named_unique(
    name: &str,
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
    for (index, record) in records.iter().enumerate() {
        if record.deleted || excluded_index == Some(index) {
            continue;
        }
        if fields.iter().all(|field| {
            let value = values.get(field).unwrap_or(&Value::Null);
            let other = record.values.get(field).unwrap_or(&Value::Null);
            values_equal(value, other)
        }) {
            return Err(DbfError::Invalid(format!(
                "constraint violation: duplicate key for constraint {name}"
            )));
        }
    }
    Ok(())
}

fn validate_constraint_name(name: &str, path: &str) -> Result<(), DbfError> {
    if name.trim().is_empty() || name.len() > 128 {
        return Err(DbfError::Invalid(format!(
            "{path} name must contain 1 to 128 non-whitespace bytes"
        )));
    }
    Ok(())
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
    if let Some(action) = &key.on_delete {
        // The shape and local-field checks above make this safe to inspect here.
        validate_set_null_fields(action, &key.fields, path, fields)?;
    }
    if let Some(action) = &key.on_update {
        validate_set_null_fields(action, &key.fields, path, fields)?;
    }
    Ok(())
}

impl SchemaMetadata {
    fn validate_action_fields(
        &self,
        action: &ForeignKeyAction,
        local_fields: &[String],
        fields: &[FieldDescriptor],
        path: &str,
    ) -> Result<(), DbfError> {
        validate_set_null_fields(action, local_fields, path, fields)?;
        if matches!(action, ForeignKeyAction::SetNull)
            && local_fields.iter().any(|field| {
                self.fields
                    .get(field)
                    .is_some_and(|metadata| metadata.not_null || metadata.primary)
                    || self.constraints.primary.iter().any(|name| name == field)
            })
        {
            return Err(DbfError::Invalid(format!(
                "{path} set_null requires nullable child fields"
            )));
        }
        Ok(())
    }
}

fn validate_set_null_fields(
    action: &ForeignKeyAction,
    local_fields: &[String],
    path: &str,
    fields: &[FieldDescriptor],
) -> Result<(), DbfError> {
    if matches!(action, ForeignKeyAction::SetNull)
        && local_fields.iter().any(|name| {
            !fields
                .iter()
                .any(|field| !field.is_system() && field.name == *name)
        })
    {
        return Err(DbfError::Invalid(format!(
            "{path} set_null references an unknown child field"
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
