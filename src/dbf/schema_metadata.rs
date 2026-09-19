use super::{DbfError, DbfRecord, FieldDescriptor};
use crate::json_order::compare_scalar_values;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::cmp::Ordering;
use std::collections::BTreeMap;
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
    fields: BTreeMap<String, FieldMetadata>,
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
}

impl SchemaMetadata {
    pub(super) fn from_bytes(bytes: &[u8]) -> Result<Self, DbfError> {
        let metadata = serde_json::from_slice::<Self>(bytes)
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
        Ok(metadata)
    }

    pub(super) fn validate_fields(&self, fields: &[FieldDescriptor]) -> Result<(), DbfError> {
        let mut primary = false;
        for (name, metadata) in &self.fields {
            if !fields
                .iter()
                .any(|field| !field.is_system() && field.name == *name)
            {
                return Err(DbfError::Invalid(format!(
                    "schema metadata refers to unknown field {name}"
                )));
            }
            if metadata.primary && primary {
                return Err(DbfError::Invalid(
                    "composite primary keys are not supported by this schema version".into(),
                ));
            }
            primary |= metadata.primary;
        }
        Ok(())
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
            "fields": self.fields,
        })
    }
}

impl SchemaMetadata {
    fn validate_values(
        &self,
        values: &Map<String, Value>,
        records: &[DbfRecord],
        excluded_index: Option<usize>,
    ) -> Result<(), DbfError> {
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
        Ok(())
    }
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
