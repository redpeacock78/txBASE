use super::super::{
    Collation, QueryError, QueryRequest, ordering::compare_for_sort_with_collation,
};
use super::validate_cursor_snapshot;
use crate::dbf::DbfRecord;
use crate::query_path::field_value;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;

const LEGACY_CURSOR_VERSION: u8 = 1;
const CURSOR_VERSION: u8 = 2;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u8,
    record: usize,
    keys: Vec<CursorKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    collation: Option<Collation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    snapshot: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CursorKey {
    field: String,
    direction: i8,
    present: bool,
    value: Value,
}

pub fn apply<'a>(
    mut records: Vec<&'a DbfRecord>,
    request: &QueryRequest,
    snapshot: u64,
) -> Result<(Vec<&'a DbfRecord>, Option<String>), QueryError> {
    let cursor = request.cursor.as_deref().map(parse_cursor).transpose()?;
    if let Some(cursor) = cursor.as_ref() {
        validate_parsed_cursor(cursor, &request.sort, request.collation)?;
        validate_cursor_snapshot(cursor.snapshot, snapshot)?;
        records.retain(|record| {
            compare_record_to_cursor(record, cursor, &request.sort, request.collation).is_gt()
        });
    }
    if let Some(limit) = request.limit {
        records.truncate(limit.try_into().unwrap_or(usize::MAX));
    }

    let page_size = request
        .page_size
        .expect("sorted pagination validation requires page_size") as usize;
    let has_more = records.len() > page_size;
    if has_more {
        records.truncate(page_size);
    }
    let next_cursor = if has_more {
        records
            .last()
            .map(|record| encode_cursor(record, &request.sort, request.collation, snapshot))
            .transpose()?
    } else {
        None
    };
    Ok((records, next_cursor))
}

pub(super) fn validate_cursor(
    cursor: &str,
    sort: &IndexMap<String, i8>,
    collation: Option<Collation>,
) -> Result<(), QueryError> {
    let cursor = parse_cursor(cursor)?;
    validate_parsed_cursor(&cursor, sort, collation)
}

fn parse_cursor(cursor: &str) -> Result<Cursor, QueryError> {
    serde_json::from_str(cursor)
        .map_err(|error| QueryError::Invalid(format!("invalid sorted cursor: {error}")))
}

fn validate_parsed_cursor(
    cursor: &Cursor,
    sort: &IndexMap<String, i8>,
    collation: Option<Collation>,
) -> Result<(), QueryError> {
    if !matches!(cursor.version, LEGACY_CURSOR_VERSION | CURSOR_VERSION) {
        return Err(QueryError::Invalid(format!(
            "unsupported sorted cursor version {}",
            cursor.version
        )));
    }
    if cursor.version == CURSOR_VERSION && cursor.snapshot.is_none() {
        return Err(QueryError::Invalid(
            "sorted cursor snapshot is missing".into(),
        ));
    }
    if cursor.keys.len() != sort.len() {
        return Err(QueryError::Invalid(
            "sorted cursor does not match the sort definition".into(),
        ));
    }
    if cursor.collation != collation {
        return Err(QueryError::Invalid(
            "sorted cursor does not match the collation".into(),
        ));
    }
    for ((field, direction), key) in sort.iter().zip(&cursor.keys) {
        if key.field != *field || key.direction != *direction {
            return Err(QueryError::Invalid(
                "sorted cursor does not match the sort definition".into(),
            ));
        }
        if !key.present && !key.value.is_null() {
            return Err(QueryError::Invalid(
                "missing sorted cursor values must be null".into(),
            ));
        }
    }
    Ok(())
}

fn encode_cursor(
    record: &DbfRecord,
    sort: &IndexMap<String, i8>,
    collation: Option<Collation>,
    snapshot: u64,
) -> Result<String, QueryError> {
    let keys = sort
        .iter()
        .map(|(field, direction)| {
            let value = field_value(&record.values, field);
            CursorKey {
                field: field.clone(),
                direction: *direction,
                present: value.is_some(),
                value: value.unwrap_or(Value::Null),
            }
        })
        .collect();
    serde_json::to_string(&Cursor {
        version: CURSOR_VERSION,
        record: record.number,
        keys,
        collation,
        snapshot: Some(snapshot),
    })
    .map_err(|error| QueryError::Invalid(format!("sorted cursor encoding failed: {error}")))
}

fn compare_record_to_cursor(
    record: &DbfRecord,
    cursor: &Cursor,
    sort: &IndexMap<String, i8>,
    collation: Option<Collation>,
) -> Ordering {
    for ((field, direction), key) in sort.iter().zip(&cursor.keys) {
        let value = field_value(&record.values, field);
        let cursor_value = key.present.then_some(&key.value);
        let ordering = compare_for_sort_with_collation(value.as_ref(), cursor_value, collation);
        if !ordering.is_eq() {
            return if *direction == 1 {
                ordering
            } else {
                ordering.reverse()
            };
        }
    }
    record.number.cmp(&cursor.record)
}
