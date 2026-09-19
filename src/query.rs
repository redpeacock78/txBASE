use crate::dbf::DbfTable;
use crate::query_path::{field_value, project};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

mod aggregation;
pub mod join;
mod ordering;
mod pagination;
mod planner;
mod validation;

use ordering::{compare_records, compare_values, sort_ordered_prefix};
pub use pagination::QueryPage;
pub use planner::QueryPlan;

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
    let access = planner::choose(dbf_path.as_ref(), request);
    execute_query_with_records(table, request, access.records, access.ordered_prefix)
}

pub fn explain_query_at(
    dbf_path: impl AsRef<std::path::Path>,
    request: &QueryRequest,
) -> Result<QueryPlan, QueryError> {
    validation::validate(request)?;
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

    let (records, next_cursor) = pagination::apply(records, request)?;

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

pub(crate) fn matches_filter(
    values: &Map<String, Value>,
    filter: &Map<String, Value>,
) -> Result<bool, QueryError> {
    for (field, condition) in filter {
        let matches = match field.as_str() {
            "$and" => condition
                .as_array()
                .ok_or_else(|| QueryError::Invalid("$and must be an array".into()))?
                .iter()
                .map(|clause| {
                    let clause = clause.as_object().ok_or_else(|| {
                        QueryError::Invalid("$and clause must be an object".into())
                    })?;
                    matches_filter(values, clause)
                })
                .try_fold(true, |matched, result| result.map(|value| matched && value))?,
            "$or" => condition
                .as_array()
                .ok_or_else(|| QueryError::Invalid("$or must be an array".into()))?
                .iter()
                .map(|clause| {
                    let clause = clause.as_object().ok_or_else(|| {
                        QueryError::Invalid("$or clause must be an object".into())
                    })?;
                    matches_filter(values, clause)
                })
                .try_fold(false, |matched, result| {
                    result.map(|value| matched || value)
                })?,
            "$not" => {
                let clause = condition
                    .as_object()
                    .ok_or_else(|| QueryError::Invalid("$not must be an object".into()))?;
                !matches_filter(values, clause)?
            }
            "$expr" => matches_expression(values, condition)?,
            _ => {
                let actual = field_value(values, field);
                matches_condition(actual.as_ref(), condition)?
            }
        };
        if !matches {
            return Ok(false);
        }
    }
    Ok(true)
}

fn matches_expression(values: &Map<String, Value>, expression: &Value) -> Result<bool, QueryError> {
    let expression = expression
        .as_object()
        .ok_or_else(|| QueryError::Invalid("filter.$expr must be an object".into()))?;
    let Some((operator, operands)) = expression.iter().next() else {
        return Err(QueryError::Invalid("filter.$expr cannot be empty".into()));
    };
    let operands = operands
        .as_array()
        .ok_or_else(|| QueryError::Invalid(format!("filter.$expr.{operator} must be an array")))?;
    let [left, right] = operands.as_slice() else {
        return Err(QueryError::Invalid(format!(
            "filter.$expr.{operator} requires two operands"
        )));
    };
    let (Some(left), Some(right)) = (
        resolve_expression_operand(values, left),
        resolve_expression_operand(values, right),
    ) else {
        return Ok(false);
    };
    Ok(match operator.as_str() {
        "$eq" => left == right,
        "$ne" => left != right,
        "$gt" => compare_values(&left, &right).is_some_and(|ordering| ordering.is_gt()),
        "$gte" => compare_values(&left, &right).is_some_and(|ordering| ordering.is_ge()),
        "$lt" => compare_values(&left, &right).is_some_and(|ordering| ordering.is_lt()),
        "$lte" => compare_values(&left, &right).is_some_and(|ordering| ordering.is_le()),
        _ => {
            return Err(QueryError::Invalid(format!(
                "unsupported expression operator {operator}"
            )));
        }
    })
}

fn resolve_expression_operand(values: &Map<String, Value>, operand: &Value) -> Option<Value> {
    let Some(reference) = operand.as_str().and_then(|value| value.strip_prefix('$')) else {
        return Some(operand.clone());
    };
    (!reference.is_empty())
        .then(|| field_value(values, reference))
        .flatten()
}

fn matches_condition(actual: Option<&Value>, condition: &Value) -> Result<bool, QueryError> {
    let Some(operators) = condition.as_object() else {
        return Ok(actual.is_some_and(|value| equality_matches(value, condition)));
    };
    if !operators.keys().any(|key| key.starts_with('$')) {
        return Ok(actual.is_some_and(|value| equality_matches(value, condition)));
    }

    operators
        .iter()
        .try_fold(true, |matched, (operator, operand)| {
            if !matched {
                return Ok(false);
            }
            match operator.as_str() {
                "$eq" => Ok(actual.is_some_and(|value| equality_matches(value, operand))),
                "$ne" => Ok(actual.is_none_or(|value| !equality_matches(value, operand))),
                "$gt" => Ok(compare_any(actual, operand, |ordering| ordering.is_gt())),
                "$gte" => Ok(compare_any(actual, operand, |ordering| ordering.is_ge())),
                "$lt" => Ok(compare_any(actual, operand, |ordering| ordering.is_lt())),
                "$lte" => Ok(compare_any(actual, operand, |ordering| ordering.is_le())),
                "$in" => {
                    let values = operand
                        .as_array()
                        .ok_or_else(|| QueryError::Invalid("$in must be an array".into()))?;
                    Ok(actual.is_some_and(|value| {
                        values
                            .iter()
                            .any(|expected| equality_matches(value, expected))
                    }))
                }
                "$nin" => {
                    let values = operand
                        .as_array()
                        .ok_or_else(|| QueryError::Invalid("$nin must be an array".into()))?;
                    Ok(actual.is_none_or(|value| {
                        values
                            .iter()
                            .all(|expected| !equality_matches(value, expected))
                    }))
                }
                "$not" => Ok(!matches_condition(actual, operand)?),
                _ => Err(QueryError::Invalid(format!(
                    "unsupported operator {operator}"
                ))),
            }
        })
}

fn equality_matches(actual: &Value, expected: &Value) -> bool {
    actual == expected
        || actual
            .as_array()
            .is_some_and(|values| values.iter().any(|value| value == expected))
}

fn compare_any(
    actual: Option<&Value>,
    expected: &Value,
    predicate: impl Fn(Ordering) -> bool,
) -> bool {
    let Some(actual) = actual else {
        return false;
    };
    if let Some(values) = actual.as_array() {
        values
            .iter()
            .any(|value| compare_values(value, expected).is_some_and(&predicate))
    } else {
        compare_values(actual, expected).is_some_and(predicate)
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
mod aggregation_tests;

#[cfg(test)]
mod join_tests;
