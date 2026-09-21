use super::{QueryError, QueryRequest};
use serde_json::Value;

mod physical;
mod sorted;

pub(super) const MAX_PAGE_SIZE: u64 = 1_000;

pub(super) use physical::{apply, execute_page as execute_physical_page};
pub(super) use sorted::apply as apply_sorted;

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
            physical::cursor_position(cursor)?;
        } else {
            sorted::validate_cursor(cursor, &request.sort, request.collation)?;
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
fn validate_cursor_snapshot(snapshot: Option<u64>, current: u64) -> Result<(), QueryError> {
    if snapshot.is_some_and(|snapshot| snapshot != current) {
        return Err(QueryError::Invalid(
            "cursor belongs to a different table snapshot".into(),
        ));
    }
    Ok(())
}
