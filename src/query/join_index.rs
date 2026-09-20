use super::join::{JoinError, JoinRequest, JoinSpec, JoinType, emit};
use super::join_pipeline::push_combined;
use crate::catalog::Catalog;
use crate::dbf::DbfRecord;
use crate::index::IndexFile;
use crate::query_path::field_value;
use serde_json::{Map, Value};
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

pub(super) fn execute_stage(
    left: &[Map<String, Value>],
    right: &[Map<String, Value>],
    right_numbers: &[usize],
    spec: &JoinSpec,
    index: &IndexFile,
    local_field: &str,
    index_field: &str,
) -> Result<Option<Vec<Map<String, Value>>>, JoinError> {
    let Some(probes) = probe_rows(left, local_field, index_field, index)? else {
        return Ok(None);
    };
    if right.len() != right_numbers.len() {
        return Err(JoinError::Invalid(
            "join index row numbers do not match loaded rows".into(),
        ));
    }
    let right_positions = right_numbers
        .iter()
        .enumerate()
        .map(|(position, number)| (*number, position))
        .collect::<BTreeMap<_, _>>();
    let mut output = Vec::new();
    for (left_row, matches) in left.iter().zip(&probes) {
        match &spec.kind {
            JoinType::Inner => {
                push_row_matches(&mut output, left_row, matches, right, &right_positions)?;
            }
            JoinType::Left => {
                if matches.is_empty() {
                    push_combined(&mut output, Some(left_row), None)?;
                } else {
                    push_row_matches(&mut output, left_row, matches, right, &right_positions)?;
                }
            }
            JoinType::Semi if !matches.is_empty() => {
                push_combined(&mut output, Some(left_row), None)?;
            }
            JoinType::Anti if matches.is_empty() => {
                push_combined(&mut output, Some(left_row), None)?;
            }
            JoinType::Semi | JoinType::Anti => {}
            JoinType::Right | JoinType::Cross => return Ok(None),
        }
    }
    Ok(Some(output))
}

pub(super) fn execute_right_stage(
    left: &[Map<String, Value>],
    right: &[Map<String, Value>],
    right_numbers: &[usize],
    index: &IndexFile,
    local_field: &str,
    index_field: &str,
) -> Result<Option<Vec<Map<String, Value>>>, JoinError> {
    let Some(probes) = probe_rows(left, local_field, index_field, index)? else {
        return Ok(None);
    };
    if right.len() != right_numbers.len() {
        return Err(JoinError::Invalid(
            "join index row numbers do not match loaded rows".into(),
        ));
    }
    let right_positions = right_numbers
        .iter()
        .enumerate()
        .map(|(position, number)| (*number, position))
        .collect::<BTreeMap<_, _>>();
    let mut matches_by_right = BTreeMap::<usize, Vec<usize>>::new();
    for (left_position, matches) in probes.iter().enumerate() {
        for record_number in matches {
            if !right_positions.contains_key(record_number) {
                return Err(JoinError::Invalid(format!(
                    "join index references missing active row: {record_number}"
                )));
            }
            matches_by_right
                .entry(*record_number)
                .or_default()
                .push(left_position);
        }
    }

    let mut output = Vec::new();
    for (right_row, record_number) in right.iter().zip(right_numbers) {
        if let Some(left_positions) = matches_by_right.get(record_number) {
            for left_position in left_positions {
                push_combined(&mut output, Some(&left[*left_position]), Some(right_row))?;
            }
        } else {
            push_combined(&mut output, None, Some(right_row))?;
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

fn probe_rows(
    rows: &[Map<String, Value>],
    probe_field: &str,
    index_field: &str,
    index: &IndexFile,
) -> Result<Option<Vec<Vec<usize>>>, JoinError> {
    let mut probes = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(value) = field_value(row, probe_field) else {
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

fn push_row_matches(
    output: &mut Vec<Map<String, Value>>,
    left_row: &Map<String, Value>,
    matches: &[usize],
    right: &[Map<String, Value>],
    right_positions: &BTreeMap<usize, usize>,
) -> Result<(), JoinError> {
    for record_number in matches {
        let Some(position) = right_positions.get(record_number).copied() else {
            return Err(JoinError::Invalid(format!(
                "join index references missing active row: {record_number}"
            )));
        };
        push_combined(output, Some(left_row), Some(&right[position]))?;
    }
    Ok(())
}

fn is_indexable(value: &Value) -> bool {
    matches!(value, Value::Bool(_) | Value::Number(_) | Value::String(_))
}
