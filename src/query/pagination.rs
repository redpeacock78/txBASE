use super::{
    Collation, QueryError, QueryRequest, matches_filter, ordering::compare_for_sort_with_collation,
};
use crate::dbf::{DbfRecord, DbfTable};
use crate::query_path::{field_value, project};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;

pub(super) const MAX_PAGE_SIZE: u64 = 1_000;
const PHYSICAL_CURSOR_VERSION: u8 = 1;
const LEGACY_SORTED_CURSOR_VERSION: u8 = 1;
const SORTED_CURSOR_VERSION: u8 = 2;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PhysicalCursor {
    version: u8,
    record: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    snapshot: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SortedCursor {
    version: u8,
    record: usize,
    keys: Vec<SortedCursorKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    collation: Option<Collation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    snapshot: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SortedCursorKey {
    field: String,
    direction: i8,
    present: bool,
    value: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryPage {
    pub records: Vec<Value>,
    pub next_cursor: Option<String>,
}

pub(super) fn validate(request: &QueryRequest) -> Result<(), QueryError> {
    let paginated = request.page_size.is_some() || request.cursor.is_some();
    if let Some(page_size) = request.page_size {
        if !(1..=MAX_PAGE_SIZE).contains(&page_size) {
            return Err(QueryError::Invalid(format!(
                "page_size must be between 1 and {MAX_PAGE_SIZE}"
            )));
        }
    }
    if request.cursor.is_some() && request.page_size.is_none() {
        return Err(QueryError::Invalid(
            "cursor requires page_size for the next page boundary".into(),
        ));
    }
    if paginated && request.skip.is_some() {
        return Err(QueryError::Invalid(
            "cursor pagination cannot be combined with skip".into(),
        ));
    }
    if let Some(cursor) = request.cursor.as_deref() {
        if request.sort.is_empty() {
            cursor_position(cursor)?;
        } else {
            validate_sorted_cursor(
                &parse_sorted_cursor(cursor)?,
                &request.sort,
                request.collation,
            )?;
        }
    }
    Ok(())
}

pub(super) fn is_physical_page(request: &QueryRequest) -> bool {
    is_page(request) && request.sort.is_empty()
}

pub(super) fn is_sorted_page(request: &QueryRequest) -> bool {
    is_page(request) && !request.sort.is_empty()
}

fn is_page(request: &QueryRequest) -> bool {
    request.page_size.is_some() || request.cursor.is_some()
}

pub(super) fn execute_physical_page(
    table: &DbfTable,
    request: &QueryRequest,
) -> Result<QueryPage, QueryError> {
    let snapshot = table.representation_hash();
    let cursor = request
        .cursor
        .as_deref()
        .map(|cursor| physical_cursor_position(cursor, snapshot))
        .transpose()?
        .unwrap_or_default();
    let page_size = request
        .page_size
        .expect("physical page validation requires page_size")
        .try_into()
        .unwrap_or(usize::MAX);
    let limit = request
        .limit
        .map(|limit| limit.try_into().unwrap_or(usize::MAX));
    let mut records = Vec::with_capacity(page_size);
    let mut matched = 0usize;
    let mut has_more = false;
    let mut last_number = None;

    for record in table.active_records() {
        if record.number <= cursor {
            continue;
        }
        if limit.is_some_and(|limit| matched >= limit) {
            break;
        }
        if !matches_filter(&record.values, &request.filter)? {
            continue;
        }
        matched = matched.saturating_add(1);
        if records.len() == page_size {
            has_more = true;
            break;
        }
        last_number = Some(record.number);
        records.push(project(record, &request.projection));
    }

    Ok(QueryPage {
        records,
        next_cursor: has_more.then(|| {
            encode_physical_cursor(last_number.expect("page has a last record"), snapshot)
        }),
    })
}

pub(super) fn apply<'a>(
    mut records: Vec<&'a DbfRecord>,
    request: &QueryRequest,
    snapshot: u64,
) -> Result<(Vec<&'a DbfRecord>, Option<String>), QueryError> {
    let paginated = request.page_size.is_some() || request.cursor.is_some();
    if !paginated {
        let skip = request
            .skip
            .unwrap_or_default()
            .try_into()
            .unwrap_or(usize::MAX);
        let limit = request
            .limit
            .map(|limit| limit.try_into().unwrap_or(usize::MAX));
        let records = records.into_iter().skip(skip);
        let records = match limit {
            Some(limit) => records.take(limit).collect(),
            None => records.collect(),
        };
        return Ok((records, None));
    }

    records.sort_unstable_by_key(|record| record.number);
    if let Some(cursor) = request.cursor.as_deref() {
        let cursor = physical_cursor_position(cursor, snapshot)?;
        records.retain(|record| record.number > cursor);
    }
    if let Some(limit) = request.limit {
        records.truncate(limit.try_into().unwrap_or(usize::MAX));
    }

    let page_size = request
        .page_size
        .expect("pagination validation requires page_size") as usize;
    let has_more = records.len() > page_size;
    if has_more {
        records.truncate(page_size);
    }
    let next_cursor = has_more
        .then(|| records.last().map(|record| record.number.to_string()))
        .flatten();
    Ok((records, next_cursor))
}

pub(super) fn apply_sorted<'a>(
    mut records: Vec<&'a DbfRecord>,
    request: &QueryRequest,
    snapshot: u64,
) -> Result<(Vec<&'a DbfRecord>, Option<String>), QueryError> {
    let cursor = request
        .cursor
        .as_deref()
        .map(parse_sorted_cursor)
        .transpose()?;
    if let Some(cursor) = cursor.as_ref() {
        validate_sorted_cursor(cursor, &request.sort, request.collation)?;
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
            .map(|record| encode_sorted_cursor(record, &request.sort, request.collation, snapshot))
            .transpose()?
    } else {
        None
    };
    Ok((records, next_cursor))
}

fn parse_sorted_cursor(cursor: &str) -> Result<SortedCursor, QueryError> {
    serde_json::from_str(cursor)
        .map_err(|error| QueryError::Invalid(format!("invalid sorted cursor: {error}")))
}

fn validate_sorted_cursor(
    cursor: &SortedCursor,
    sort: &IndexMap<String, i8>,
    collation: Option<Collation>,
) -> Result<(), QueryError> {
    if !matches!(
        cursor.version,
        LEGACY_SORTED_CURSOR_VERSION | SORTED_CURSOR_VERSION
    ) {
        return Err(QueryError::Invalid(format!(
            "unsupported sorted cursor version {}",
            cursor.version
        )));
    }
    if cursor.version == SORTED_CURSOR_VERSION && cursor.snapshot.is_none() {
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

fn encode_sorted_cursor(
    record: &DbfRecord,
    sort: &IndexMap<String, i8>,
    collation: Option<Collation>,
    snapshot: u64,
) -> Result<String, QueryError> {
    let keys = sort
        .iter()
        .map(|(field, direction)| {
            let value = field_value(&record.values, field);
            SortedCursorKey {
                field: field.clone(),
                direction: *direction,
                present: value.is_some(),
                value: value.unwrap_or(Value::Null),
            }
        })
        .collect();
    serde_json::to_string(&SortedCursor {
        version: SORTED_CURSOR_VERSION,
        record: record.number,
        keys,
        collation,
        snapshot: Some(snapshot),
    })
    .map_err(|error| QueryError::Invalid(format!("sorted cursor encoding failed: {error}")))
}

fn compare_record_to_cursor(
    record: &DbfRecord,
    cursor: &SortedCursor,
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

pub(super) fn cursor_position(cursor: &str) -> Result<usize, QueryError> {
    Ok(parse_physical_cursor(cursor)?.record)
}

fn physical_cursor_position(cursor: &str, snapshot: u64) -> Result<usize, QueryError> {
    let cursor = parse_physical_cursor(cursor)?;
    validate_cursor_snapshot(cursor.snapshot, snapshot)?;
    Ok(cursor.record)
}

fn parse_physical_cursor(cursor: &str) -> Result<PhysicalCursor, QueryError> {
    if let Ok(record) = cursor.parse::<usize>() {
        return Ok(PhysicalCursor {
            version: 0,
            record,
            snapshot: None,
        });
    }
    let parsed = serde_json::from_str::<PhysicalCursor>(cursor)
        .map_err(|error| QueryError::Invalid(format!("invalid physical cursor: {error}")))?;
    if parsed.version != PHYSICAL_CURSOR_VERSION {
        return Err(QueryError::Invalid(format!(
            "unsupported physical cursor version {}",
            parsed.version
        )));
    }
    if parsed.snapshot.is_none() {
        return Err(QueryError::Invalid(
            "physical cursor snapshot is missing".into(),
        ));
    }
    Ok(parsed)
}

fn encode_physical_cursor(record: usize, snapshot: u64) -> String {
    serde_json::to_string(&PhysicalCursor {
        version: PHYSICAL_CURSOR_VERSION,
        record,
        snapshot: Some(snapshot),
    })
    .expect("physical cursor is serializable")
}

fn validate_cursor_snapshot(snapshot: Option<u64>, current: u64) -> Result<(), QueryError> {
    if snapshot.is_some_and(|snapshot| snapshot != current) {
        return Err(QueryError::Invalid(
            "cursor belongs to a different table snapshot".into(),
        ));
    }
    Ok(())
}
