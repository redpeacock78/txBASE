use super::join::{
    JoinError, JoinRequest, JoinSpec, JoinType, emit, encoded_key as record_encoded_key,
};
use super::join_pipeline::{encoded_key as row_encoded_key, push_combined};
use crate::dbf::DbfRecord;
use serde_json::{Map, Value};

pub(super) fn execute_join(
    left_records: &[&DbfRecord],
    right_records: &[&DbfRecord],
    request: &JoinRequest,
    local_fields: &[String],
    foreign_fields: &[String],
) -> Result<Vec<Value>, JoinError> {
    let right_keys = right_records
        .iter()
        .map(|record| record_encoded_key(&record.values, foreign_fields))
        .collect::<Result<Vec<_>, _>>()?;
    let mut output = Vec::new();
    for &left_record in left_records {
        let left_key = record_encoded_key(&left_record.values, local_fields)?;
        let mut had_matches = false;
        if let Some(left_key) = left_key.as_ref() {
            for (&right_record, right_key) in right_records.iter().zip(&right_keys) {
                if right_key.as_ref() != Some(left_key) {
                    continue;
                }
                had_matches = true;
                match &request.join.kind {
                    JoinType::Inner | JoinType::Left => {
                        emit(&mut output, request, Some(left_record), Some(right_record))?;
                    }
                    JoinType::Semi => {
                        emit(&mut output, request, Some(left_record), None)?;
                        break;
                    }
                    JoinType::Anti => break,
                    JoinType::Right | JoinType::Cross => {
                        unreachable!("join type handled above")
                    }
                }
            }
        }
        match &request.join.kind {
            JoinType::Left if !had_matches => emit(&mut output, request, Some(left_record), None)?,
            JoinType::Anti if !had_matches => emit(&mut output, request, Some(left_record), None)?,
            JoinType::Inner
            | JoinType::Left
            | JoinType::Semi
            | JoinType::Anti
            | JoinType::Right
            | JoinType::Cross => {}
        }
    }
    Ok(output)
}

pub(super) fn execute_right_join(
    left_records: &[&DbfRecord],
    right_records: &[&DbfRecord],
    request: &JoinRequest,
    local_fields: &[String],
    foreign_fields: &[String],
) -> Result<Vec<Value>, JoinError> {
    let left_keys = left_records
        .iter()
        .map(|record| record_encoded_key(&record.values, local_fields))
        .collect::<Result<Vec<_>, _>>()?;
    let mut output = Vec::new();
    for &right_record in right_records {
        let right_key = record_encoded_key(&right_record.values, foreign_fields)?;
        let mut had_matches = false;
        if let Some(right_key) = right_key.as_ref() {
            for (&left_record, left_key) in left_records.iter().zip(&left_keys) {
                if left_key.as_ref() != Some(right_key) {
                    continue;
                }
                had_matches = true;
                emit(&mut output, request, Some(left_record), Some(right_record))?;
            }
        }
        if !had_matches {
            emit(&mut output, request, None, Some(right_record))?;
        }
    }
    Ok(output)
}

pub(super) fn execute_stage(
    left: &[Map<String, Value>],
    right: &[Map<String, Value>],
    spec: &JoinSpec,
    local_fields: &[String],
    foreign_fields: &[String],
) -> Result<Vec<Map<String, Value>>, JoinError> {
    let right_keys = right
        .iter()
        .map(|row| row_encoded_key(row, foreign_fields))
        .collect::<Result<Vec<_>, _>>()?;
    let mut output = Vec::new();
    for left_row in left {
        let left_key = row_encoded_key(left_row, local_fields)?;
        let mut had_matches = false;
        if let Some(left_key) = left_key.as_ref() {
            for (right_row, right_key) in right.iter().zip(&right_keys) {
                if right_key.as_ref() != Some(left_key) {
                    continue;
                }
                had_matches = true;
                match &spec.kind {
                    JoinType::Inner | JoinType::Left => {
                        push_combined(&mut output, Some(left_row), Some(right_row))?;
                    }
                    JoinType::Semi => {
                        push_combined(&mut output, Some(left_row), None)?;
                        break;
                    }
                    JoinType::Anti => break,
                    JoinType::Right | JoinType::Cross => {
                        unreachable!("join type handled above")
                    }
                }
            }
        }
        match &spec.kind {
            JoinType::Left if !had_matches => push_combined(&mut output, Some(left_row), None)?,
            JoinType::Anti if !had_matches => push_combined(&mut output, Some(left_row), None)?,
            JoinType::Inner
            | JoinType::Left
            | JoinType::Semi
            | JoinType::Anti
            | JoinType::Right
            | JoinType::Cross => {}
        }
    }
    Ok(output)
}

pub(super) fn execute_right_stage(
    left: &[Map<String, Value>],
    right: &[Map<String, Value>],
    _spec: &JoinSpec,
    local_fields: &[String],
    foreign_fields: &[String],
) -> Result<Vec<Map<String, Value>>, JoinError> {
    let left_keys = left
        .iter()
        .map(|row| row_encoded_key(row, local_fields))
        .collect::<Result<Vec<_>, _>>()?;
    let mut output = Vec::new();
    for right_row in right {
        let right_key = row_encoded_key(right_row, foreign_fields)?;
        let mut had_matches = false;
        if let Some(right_key) = right_key.as_ref() {
            for (left_row, left_key) in left.iter().zip(&left_keys) {
                if left_key.as_ref() != Some(right_key) {
                    continue;
                }
                had_matches = true;
                push_combined(&mut output, Some(left_row), Some(right_row))?;
            }
        }
        if !had_matches {
            push_combined(&mut output, None, Some(right_row))?;
        }
    }
    Ok(output)
}
