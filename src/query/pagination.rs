use super::{QueryError, QueryRequest, matches_filter};
use crate::dbf::{DbfRecord, DbfTable};
use crate::query_path::project;
use serde_json::Value;

pub(super) const MAX_PAGE_SIZE: u64 = 1_000;

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
    if paginated && !request.sort.is_empty() {
        return Err(QueryError::Invalid(
            "cursor pagination does not support sort yet".into(),
        ));
    }
    if paginated && request.skip.is_some() {
        return Err(QueryError::Invalid(
            "cursor pagination cannot be combined with skip".into(),
        ));
    }
    if let Some(cursor) = request.cursor.as_deref() {
        cursor_position(cursor)?;
    }
    Ok(())
}

pub(super) fn is_physical_page(request: &QueryRequest) -> bool {
    request.page_size.is_some() || request.cursor.is_some()
}

pub(super) fn execute_physical_page(
    table: &DbfTable,
    request: &QueryRequest,
) -> Result<QueryPage, QueryError> {
    let cursor = request
        .cursor
        .as_deref()
        .map(cursor_position)
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
        next_cursor: has_more.then(|| last_number.expect("page has a last record").to_string()),
    })
}

pub(super) fn apply<'a>(
    mut records: Vec<&'a DbfRecord>,
    request: &QueryRequest,
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
        let cursor = cursor_position(cursor)?;
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
    cursor
        .parse::<usize>()
        .map_err(|_| QueryError::Invalid("cursor must be a decimal physical record number".into()))
}
