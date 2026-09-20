use super::join::{JoinError, JoinRequest, JoinType, emit};
use crate::catalog::Catalog;
use crate::dbf::DbfRecord;
use crate::index::IndexFile;
use crate::query_path::field_value;
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) fn load(catalog: &Catalog, table_name: &str, field: &str) -> Option<IndexFile> {
    let path = catalog.table_path(table_name)?;
    let index = IndexFile::load(path).ok()?;
    index.has_single_field(field).then_some(index)
}

pub(super) fn execute_join(
    left_records: &[&DbfRecord],
    right_records: &[&DbfRecord],
    request: &JoinRequest,
    index: &IndexFile,
    local_field: &str,
    foreign_field: &str,
) -> Result<Option<Vec<Value>>, JoinError> {
    let Some(probes) = probe_records(left_records, local_field, foreign_field, index)? else {
        return Ok(None);
    };
    let right_by_number = records_by_number(right_records);
    let mut output = Vec::new();
    for (&left_record, matches) in left_records.iter().zip(&probes) {
        match &request.join.kind {
            JoinType::Inner => {
                emit_matches(&mut output, request, left_record, matches, &right_by_number)?
            }
            JoinType::Left => {
                if matches.is_empty() {
                    emit(&mut output, request, Some(left_record), None)?;
                } else {
                    emit_matches(&mut output, request, left_record, matches, &right_by_number)?;
                }
            }
            JoinType::Semi if !matches.is_empty() => {
                emit(&mut output, request, Some(left_record), None)?;
            }
            JoinType::Anti if matches.is_empty() => {
                emit(&mut output, request, Some(left_record), None)?;
            }
            JoinType::Semi | JoinType::Anti => {}
            JoinType::Right | JoinType::Cross => return Ok(None),
        }
    }
    Ok(Some(output))
}

pub(super) fn execute_right_join(
    left_records: &[&DbfRecord],
    right_records: &[&DbfRecord],
    request: &JoinRequest,
    index: &IndexFile,
    local_field: &str,
    foreign_field: &str,
) -> Result<Option<Vec<Value>>, JoinError> {
    let Some(probes) = probe_records(right_records, foreign_field, local_field, index)? else {
        return Ok(None);
    };
    let left_by_number = records_by_number(left_records);
    let mut output = Vec::new();
    for (&right_record, matches) in right_records.iter().zip(&probes) {
        if matches.is_empty() {
            emit(&mut output, request, None, Some(right_record))?;
        } else {
            emit_matches(&mut output, request, right_record, matches, &left_by_number)?;
        }
    }
    Ok(Some(output))
}

fn probe_records(
    records: &[&DbfRecord],
    probe_field: &str,
    index_field: &str,
    index: &IndexFile,
) -> Result<Option<Vec<Vec<usize>>>, JoinError> {
    let mut probes = Vec::with_capacity(records.len());
    for record in records {
        let Some(value) = field_value(&record.values, probe_field) else {
            probes.push(Vec::new());
            continue;
        };
        if value.is_null() {
            probes.push(Vec::new());
            continue;
        }
        if !is_indexable(&value) {
            return Ok(None);
        }
        let Some((_, records)) = index
            .lookup_eq_for_field(index_field, &value)
            .map_err(|error| JoinError::Invalid(format!("join index lookup failed: {error}")))?
        else {
            return Ok(None);
        };
        probes.push(records);
    }
    Ok(Some(probes))
}

fn records_by_number<'a>(records: &[&'a DbfRecord]) -> BTreeMap<usize, &'a DbfRecord> {
    records
        .iter()
        .map(|record| (record.number, *record))
        .collect()
}

fn emit_matches(
    output: &mut Vec<Value>,
    request: &JoinRequest,
    outer_record: &DbfRecord,
    matches: &[usize],
    inner_records: &BTreeMap<usize, &DbfRecord>,
) -> Result<(), JoinError> {
    for record_number in matches {
        let Some(inner_record) = inner_records.get(record_number).copied() else {
            return Err(JoinError::Invalid(format!(
                "join index references missing active record: {record_number}"
            )));
        };
        if matches!(&request.join.kind, JoinType::Right) {
            emit(output, request, Some(inner_record), Some(outer_record))?;
        } else {
            emit(output, request, Some(outer_record), Some(inner_record))?;
        }
    }
    Ok(())
}

fn is_indexable(value: &Value) -> bool {
    matches!(value, Value::Bool(_) | Value::Number(_) | Value::String(_))
}
