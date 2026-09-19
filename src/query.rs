use crate::dbf::DbfTable;
use crate::query_path::project;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

mod aggregation;
mod aggregation_plan;
pub mod join;
mod ordering;
mod pagination;
mod planner;
mod predicate;
mod stream;
mod validation;

use ordering::{compare_records, sort_ordered_prefix};
pub use pagination::QueryPage;
pub use planner::QueryPlan;
#[cfg(test)]
pub(crate) use predicate::matches_condition;
pub(crate) use predicate::matches_filter;
pub use stream::{QuerySnapshotStream, QueryStream, stream_query, stream_query_snapshot};

#[cfg(test)]
use crate::query_path::field_value;
#[cfg(test)]
use std::cmp::Ordering;

pub const JSON_QUERY_MEDIA_TYPE: &str = "application/json";
pub const SUPPORTED_FILTER_OPERATORS: &[&str] = &[
    "$eq", "$ne", "$gt", "$gte", "$lt", "$lte", "$in", "$nin", "$and", "$or", "$not", "$expr",
];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryRequest {
    #[serde(default)]
    pub filter: Map<String, Value>,
    #[serde(default)]
    pub sort: IndexMap<String, i8>,
    #[serde(default)]
    pub projection: BTreeMap<String, i8>,
    pub limit: Option<u64>,
    pub skip: Option<u64>,
    pub page_size: Option<u64>,
    pub cursor: Option<String>,
    pub aggregate: Option<Vec<Map<String, Value>>>,
}

#[derive(Debug)]
pub enum QueryError {
    InvalidJson(serde_json::Error),
    Invalid(String),
}

impl Display for QueryError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(f, "invalid query JSON: {error}"),
            Self::Invalid(message) => write!(f, "invalid query: {message}"),
        }
    }
}

impl Error for QueryError {}

pub fn parse(body: &[u8]) -> Result<QueryRequest, QueryError> {
    let request = serde_json::from_slice::<QueryRequest>(body).map_err(QueryError::InvalidJson)?;
    validation::validate(&request)?;
    Ok(request)
}

pub fn execute_query(table: &DbfTable, request: &QueryRequest) -> Result<Vec<Value>, QueryError> {
    Ok(execute_query_page(table, request)?.records)
}

pub fn execute_query_page(
    table: &DbfTable,
    request: &QueryRequest,
) -> Result<QueryPage, QueryError> {
    validation::validate(request)?;
    if pagination::is_physical_page(request) {
        return pagination::execute_physical_page(table, request);
    }
    execute_query_with_records(table, request, None, 0)
}

pub fn execute_query_at(
    table: &DbfTable,
    dbf_path: impl AsRef<std::path::Path>,
    request: &QueryRequest,
) -> Result<Vec<Value>, QueryError> {
    Ok(execute_query_at_page(table, dbf_path, request)?.records)
}

pub fn execute_query_at_page(
    table: &DbfTable,
    dbf_path: impl AsRef<std::path::Path>,
    request: &QueryRequest,
) -> Result<QueryPage, QueryError> {
    validation::validate(request)?;
    if pagination::is_physical_page(request) {
        return pagination::execute_physical_page(table, request);
    }
    let access = planner::choose(dbf_path.as_ref(), request);
    execute_query_with_records(table, request, access.records, access.ordered_prefix)
}

pub fn explain_query_at(
    dbf_path: impl AsRef<std::path::Path>,
    request: &QueryRequest,
) -> Result<QueryPlan, QueryError> {
    validation::validate(request)?;
    if pagination::is_physical_page(request) {
        return Ok(QueryPlan::TableScan);
    }
    Ok(planner::choose(dbf_path.as_ref(), request).plan)
}

fn execute_query_with_records(
    table: &DbfTable,
    request: &QueryRequest,
    candidate_numbers: Option<Vec<usize>>,
    ordered_prefix: usize,
) -> Result<QueryPage, QueryError> {
    let mut records = Vec::new();
    let candidates = candidate_numbers
        .map(|numbers| {
            numbers
                .into_iter()
                .filter_map(|number| table.active_record(number))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| table.active_records().collect());
    for record in candidates {
        if matches_filter(&record.values, &request.filter)? {
            records.push(record);
        }
    }

    if let Some(stages) = request.aggregate.as_deref() {
        return Ok(QueryPage {
            records: aggregation::execute(&records, stages)?,
            next_cursor: None,
        });
    }

    if ordered_prefix == 0 {
        records.sort_by(|left, right| compare_records(left, right, &request.sort));
    } else if ordered_prefix < request.sort.len() {
        sort_ordered_prefix(&mut records, &request.sort, ordered_prefix);
    }

    let (records, next_cursor) = if pagination::is_sorted_page(request) {
        pagination::apply_sorted(records, request)?
    } else {
        pagination::apply(records, request)?
    };

    Ok(QueryPage {
        records: records
            .into_iter()
            .map(|record| project(record, &request.projection))
            .collect(),
        next_cursor,
    })
}

impl QueryExecutor for DbfTable {
    fn execute(&self, request: &QueryRequest) -> Result<Vec<Value>, QueryError> {
        execute_query(self, request)
    }
}

pub trait QueryExecutor {
    fn execute(&self, request: &QueryRequest) -> Result<Vec<Value>, QueryError>;
}

#[cfg(test)]
mod malformed_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod planner_tests;

#[cfg(test)]
mod planner_compound_tests;

#[cfg(test)]
mod field_expression_tests;

#[cfg(test)]
mod cursor_tests;

#[cfg(test)]
mod stream_tests;

#[cfg(test)]
mod aggregation_tests;

#[cfg(test)]
mod join_tests;
