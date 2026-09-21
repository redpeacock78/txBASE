use super::super::{QueryError, QueryRequest, matches_filter};
use super::{QueryPage, validate_cursor_snapshot};
use crate::dbf::{DbfRecord, DbfTable};
use crate::query_path::project;
use serde::{Deserialize, Serialize};

const CURSOR_VERSION: u8 = 1;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u8,
    record: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    snapshot: Option<u64>,
}

pub fn execute_page(table: &DbfTable, request: &QueryRequest) -> Result<QueryPage, QueryError> {
    let snapshot = table.representation_hash();
    let cursor = request
        .cursor
        .as_deref()
        .map(|cursor| cursor_position_at(cursor, snapshot))
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
        next_cursor: has_more
            .then(|| encode_cursor(last_number.expect("page has a last record"), snapshot)),
    })
}

pub fn apply<'a>(
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
        let cursor = cursor_position_at(cursor, snapshot)?;
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

pub(super) fn cursor_position(cursor: &str) -> Result<usize, QueryError> {
    Ok(parse_cursor(cursor)?.record)
}

fn cursor_position_at(cursor: &str, snapshot: u64) -> Result<usize, QueryError> {
    let cursor = parse_cursor(cursor)?;
    validate_cursor_snapshot(cursor.snapshot, snapshot)?;
    Ok(cursor.record)
}

fn parse_cursor(cursor: &str) -> Result<Cursor, QueryError> {
    if let Ok(record) = cursor.parse::<usize>() {
        return Ok(Cursor {
            version: 0,
            record,
            snapshot: None,
        });
    }
    let parsed = serde_json::from_str::<Cursor>(cursor)
        .map_err(|error| QueryError::Invalid(format!("invalid physical cursor: {error}")))?;
    if parsed.version != CURSOR_VERSION {
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

fn encode_cursor(record: usize, snapshot: u64) -> String {
    serde_json::to_string(&Cursor {
        version: CURSOR_VERSION,
        record,
        snapshot: Some(snapshot),
    })
    .expect("physical cursor is serializable")
}
