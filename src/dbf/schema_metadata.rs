use super::codec::canonical_encoding_name;
use super::{DbfError, DbfRecord, FieldDescriptor};
use crate::json_order::compare_scalar_values;
use crate::query::{matches_filter, validate_filter};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const SCHEMA_FORMAT: &str = "txbase-schema";
const SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SchemaMetadata {
    format: String,
    version: u8,
    #[serde(default)]
    encoding: Option<String>,
    #[serde(default)]
    fields: BTreeMap<String, FieldMetadata>,
    #[serde(default)]
    checks: Vec<Map<String, Value>>,
    #[serde(default)]
    constraints: ConstraintMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldMetadata {
    #[serde(default)]
    primary: bool,
    #[serde(default)]
    unique: bool,
    #[serde(default)]
    not_null: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConstraintMetadata {
    #[serde(default)]
    primary: Vec<String>,
    #[serde(default)]
    unique: Vec<Vec<String>>,
}

impl SchemaMetadata {
    pub(super) fn from_bytes(bytes: &[u8]) -> Result<Self, DbfError> {
        let mut metadata = serde_json::from_slice::<Self>(bytes)
            .map_err(|error| DbfError::Invalid(format!("invalid schema metadata JSON: {error}")))?;
        if metadata.format != SCHEMA_FORMAT {
            return Err(DbfError::Invalid(format!(
                "unsupported schema metadata format: {}",
                metadata.format
            )));
        }
        if metadata.version != SCHEMA_VERSION {
            return Err(DbfError::Invalid(format!(
                "unsupported schema metadata version: {}",
                metadata.version
            )));
        }
        if let Some(encoding) = metadata.encoding.as_deref() {
            let canonical = canonical_encoding_name(encoding).ok_or_else(|| {
                DbfError::Invalid(format!("unsupported schema encoding override: {encoding}"))
            })?;
            metadata.encoding = Some(canonical.to_owned());
        }
        for (index, check) in metadata.checks.iter().enumerate() {
            validate_filter(check, &format!("schema.checks[{index}]")).map_err(|error| {
                DbfError::Invalid(format!("invalid schema check {index}: {error}"))
            })?;
        }
        Ok(metadata)
    }

    pub(super) fn validate_fields(&self, fields: &[FieldDescriptor]) -> Result<(), DbfError> {
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

    pub(super) fn apply_defaults(&self, values: &mut Map<String, Value>) {
        for (name, metadata) in &self.fields {
            if let Some(default) = &metadata.default {
                values
                    .entry(name.clone())
                    .or_insert_with(|| default.clone());
            }
        }
    }

    pub(super) fn validate_records(&self, records: &[DbfRecord]) -> Result<(), DbfError> {
        for (index, record) in records.iter().enumerate() {
            if !record.deleted {
                self.validate_values(&record.values, records, Some(index))?;
            }
        }
        Ok(())
    }

    pub(super) fn validate_candidate(
        &self,
        values: &Map<String, Value>,
        records: &[DbfRecord],
        excluded_index: Option<usize>,
    ) -> Result<(), DbfError> {
        self.validate_values(values, records, excluded_index)
    }

    pub(super) fn json(&self) -> Value {
        json!({
            "format": self.format,
            "version": self.version,
            "encoding": self.encoding,
            "fields": self.fields,
            "checks": self.checks,
            "constraints": self.constraints,
        })
    }

    pub(super) fn encoding(&self) -> Option<&str> {
        self.encoding.as_deref()
    }
}

impl SchemaMetadata {
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

pub(super) fn schema_metadata_path(path: &Path) -> PathBuf {
    path.with_extension("txschema.json")
}

pub(super) fn schema_metadata_bytes(path: &Path) -> Result<Option<Vec<u8>>, DbfError> {
    let path = schema_metadata_path(path);
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn read_schema_metadata(
    path: &Path,
) -> Result<Option<(SchemaMetadata, Vec<u8>)>, DbfError> {
    let Some(bytes) = schema_metadata_bytes(path)? else {
        return Ok(None);
    };
    let metadata = SchemaMetadata::from_bytes(&bytes)?;
    Ok(Some((metadata, bytes)))
}
