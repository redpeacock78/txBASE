use super::DbfError;
use super::codec::canonical_encoding_name;
use super::types::ForeignKeyAction;
use crate::ConstraintMode;
use crate::query::validate_filter;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const SCHEMA_FORMAT: &str = "txbase-schema";
const CURRENT_SCHEMA_VERSION: u8 = 2;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    constraint_name: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    deferrable: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    deferred: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    on_delete: Option<ForeignKeyAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    on_update: Option<ForeignKeyAction>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConstraintMetadata {
    #[serde(default)]
    primary: Vec<String>,
    #[serde(default)]
    unique: Vec<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    foreign_keys: Vec<CompositeForeignKeyMetadata>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    deferrable: Vec<DeferrableConstraintMetadata>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompositeForeignKeyMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    fields: Vec<String>,
    references: ForeignKeyTargetMetadata,
    #[serde(default, skip_serializing_if = "is_false")]
    deferrable: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    deferred: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    on_delete: Option<ForeignKeyAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    on_update: Option<ForeignKeyAction>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeferrableConstraintMetadata {
    name: String,
    kind: DeferrableConstraintKind,
    #[serde(default)]
    fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    predicate: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "is_false")]
    deferred: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DeferrableConstraintKind {
    Unique,
    PrimaryKey,
    Check,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ForeignKeyTargetMetadata {
    table: String,
    fields: Vec<String>,
}

fn is_false(value: &bool) -> bool {
    !value
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
        if !matches!(metadata.version, 1..=CURRENT_SCHEMA_VERSION) {
            return Err(DbfError::Invalid(format!(
                "unsupported schema metadata version: {}",
                metadata.version
            )));
        }
        if metadata.version < CURRENT_SCHEMA_VERSION
            && (!metadata.constraints.deferrable.is_empty()
                || metadata
                    .fields
                    .values()
                    .any(|field| field.constraint_name.is_some() || field.deferrable)
                || metadata
                    .constraints
                    .foreign_keys
                    .iter()
                    .any(|key| key.name.is_some() || key.deferrable))
        {
            return Err(DbfError::Invalid(
                "named deferrable constraints require schema metadata version 2".into(),
            ));
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
            || !self.constraints.foreign_keys.is_empty()
            || !self.constraints.deferrable.is_empty()
            || self.fields.values().any(|field| {
                field.default.is_some()
                    || field.references.is_some()
                    || field.constraint_name.is_some()
                    || field.deferrable
                    || field.deferred
                    || field.on_delete.is_some()
                    || field.on_update.is_some()
            })
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

    pub(super) fn local_deferrable_constraint_modes(&self) -> BTreeMap<String, ConstraintMode> {
        self.constraints
            .deferrable
            .iter()
            .map(|constraint| {
                (
                    constraint.name.clone(),
                    if constraint.deferred {
                        ConstraintMode::Deferred
                    } else {
                        ConstraintMode::Immediate
                    },
                )
            })
            .collect()
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
