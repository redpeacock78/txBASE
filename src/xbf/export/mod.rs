use super::{XbfError, XbfTable};
use crate::dbf::DbfTable;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

mod convert;

pub(super) use convert::value_to_dbf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct XbfExportIssue {
    pub record: Option<usize>,
    pub field: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct XbfExportReport {
    pub representable: bool,
    pub requires_schema_sidecar: bool,
    pub issues: Vec<XbfExportIssue>,
}

pub fn dbf_export_report(table: &XbfTable) -> XbfExportReport {
    let mut issues = Vec::new();
    let mut names = BTreeSet::new();
    let mut descriptors = Vec::with_capacity(table.fields.len());
    let mut shape_valid = true;
    for (index, record) in table.records.iter().enumerate() {
        if record.values.len() != table.fields.len() {
            shape_valid = false;
            issues.push(XbfExportIssue {
                record: Some(index + 1),
                field: None,
                message: "XBF record value count does not match schema".into(),
            });
        }
    }

    for (field_index, field) in table.fields.iter().enumerate() {
        if !names.insert(field.name.clone()) {
            issues.push(XbfExportIssue {
                record: None,
                field: Some(field.name.clone()),
                message: "field name is duplicated".into(),
            });
        }
        let values = if shape_valid {
            table
                .records
                .iter()
                .map(|record| &record.values[field_index])
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        match convert::descriptor(field, &values, true) {
            Ok(descriptor) => descriptors.push(descriptor),
            Err(error) => {
                issues.push(XbfExportIssue {
                    record: None,
                    field: Some(field.name.clone()),
                    message: error.to_string(),
                });
            }
        }
        for (record_index, record) in table.records.iter().enumerate() {
            let Some(value) = record.values.get(field_index) else {
                continue;
            };
            if let Err(error) = convert::value_to_dbf(field, value) {
                issues.push(XbfExportIssue {
                    record: Some(record_index + 1),
                    field: Some(field.name.clone()),
                    message: error.to_string(),
                });
            }
        }
    }

    if shape_valid && descriptors.len() == table.fields.len() {
        if let Err(error) = convert::empty_dbf(&table.fields, &descriptors) {
            issues.push(XbfExportIssue {
                record: None,
                field: None,
                message: error.to_string(),
            });
        }
        if let Err(error) = super::schema::validate_constraints(&table.fields, &table.records) {
            issues.push(XbfExportIssue {
                record: None,
                field: None,
                message: error.to_string(),
            });
        }
    }

    XbfExportReport {
        representable: issues.is_empty(),
        requires_schema_sidecar: table
            .fields
            .iter()
            .any(|field| field.primary_key || field.unique || !field.nullable),
        issues,
    }
}

pub fn to_dbf(table: &XbfTable) -> Result<DbfTable, XbfError> {
    convert::to_dbf_inner(table, false)
}

pub fn to_dbf_with_schema(table: &XbfTable) -> Result<(DbfTable, Value), XbfError> {
    super::schema::validate_constraints(&table.fields, &table.records)?;
    let dbf = convert::to_dbf_inner(table, true)?;
    Ok((dbf, convert::schema_metadata(table)))
}

pub fn save_dbf_with_schema(table: &XbfTable, path: impl AsRef<Path>) -> Result<(), XbfError> {
    let path = path.as_ref();
    let (dbf, schema) = to_dbf_with_schema(table)?;
    let mut schema_bytes = serde_json::to_vec_pretty(&schema)
        .map_err(|error| XbfError::Invalid(format!("schema metadata encoding failed: {error}")))?;
    schema_bytes.push(b'\n');
    crate::dbf::commit_schema_export(path, &dbf, &schema_bytes).map_err(convert::dbf_error)
}
