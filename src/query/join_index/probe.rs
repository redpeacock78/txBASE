use super::super::join::JoinError;
use crate::dbf::DbfRecord;
use crate::query_path::field_value;
use serde_json::{Map, Value};

enum ProbeValues {
    Missing,
    Unsupported,
    Found(Vec<Value>),
}

pub(super) fn records(
    records: &[&DbfRecord],
    probe_fields: &[&str],
    index_fields: &[&str],
    index: &crate::index::IndexFile,
) -> Result<Option<Vec<Vec<usize>>>, JoinError> {
    if probe_fields.is_empty() || probe_fields.len() != index_fields.len() {
        return Ok(None);
    }
    let mut probes = Vec::with_capacity(records.len());
    for record in records {
        let values = match values(&record.values, probe_fields) {
            ProbeValues::Missing => {
                probes.push(Vec::new());
                continue;
            }
            ProbeValues::Unsupported => return Ok(None),
            ProbeValues::Found(values) => values,
        };
        let Some((_, records)) = index
            .lookup_eq_for_fields(index_fields, &values)
            .map_err(|error| JoinError::Invalid(format!("join index lookup failed: {error}")))?
        else {
            return Ok(None);
        };
        probes.push(records);
    }
    Ok(Some(probes))
}

pub(super) fn rows(
    rows: &[Map<String, Value>],
    probe_fields: &[&str],
    index_fields: &[&str],
    index: &crate::index::IndexFile,
) -> Result<Option<Vec<Vec<usize>>>, JoinError> {
    if probe_fields.is_empty() || probe_fields.len() != index_fields.len() {
        return Ok(None);
    }
    let mut probes = Vec::with_capacity(rows.len());
    for row in rows {
        let values = match values(row, probe_fields) {
            ProbeValues::Missing => {
                probes.push(Vec::new());
                continue;
            }
            ProbeValues::Unsupported => return Ok(None),
            ProbeValues::Found(values) => values,
        };
        let Some((_, records)) = index
            .lookup_eq_for_fields(index_fields, &values)
            .map_err(|error| JoinError::Invalid(format!("join index lookup failed: {error}")))?
        else {
            return Ok(None);
        };
        probes.push(records);
    }
    Ok(Some(probes))
}

fn values(row: &Map<String, Value>, fields: &[&str]) -> ProbeValues {
    let mut values = Vec::with_capacity(fields.len());
    for field in fields {
        let Some(value) = field_value(row, field) else {
            return ProbeValues::Missing;
        };
        if value.is_null() {
            return ProbeValues::Missing;
        }
        if !is_indexable(&value) {
            return ProbeValues::Unsupported;
        }
        values.push(value);
    }
    ProbeValues::Found(values)
}

fn is_indexable(value: &Value) -> bool {
    matches!(value, Value::Bool(_) | Value::Number(_) | Value::String(_))
}
