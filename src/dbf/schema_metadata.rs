use super::DbfError;
use super::codec::canonical_encoding_name;
use crate::query::validate_filter;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const SCHEMA_FORMAT: &str = "txbase-schema";
const SCHEMA_VERSION: u8 = 1;

#[path = "schema_metadata/constraints.rs"]
mod constraints;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    references: Option<String>,
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

    pub(super) fn xbf_field_constraints(
        &self,
    ) -> Result<BTreeMap<String, (bool, bool, bool)>, DbfError> {
        if !self.checks.is_empty()
            || !self.constraints.primary.is_empty()
            || !self.constraints.unique.is_empty()
            || self
                .fields
                .values()
                .any(|field| field.default.is_some() || field.references.is_some())
        {
            return Err(DbfError::Invalid(
                "schema metadata contains constraints not representable in XBF v1".into(),
            ));
        }
        Ok(self
            .fields
            .iter()
            .map(|(name, field)| (name.clone(), (field.primary, field.unique, field.not_null)))
            .collect())
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
