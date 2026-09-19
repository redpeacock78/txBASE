use crate::dbf::{DbfRecord, DbfTable};
use crate::query_path::{field_value, project};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

mod planner;
mod validation;

pub use planner::QueryPlan;

pub const JSON_QUERY_MEDIA_TYPE: &str = "application/json";
pub const SUPPORTED_FILTER_OPERATORS: &[&str] = &[
    "$eq", "$ne", "$gt", "$gte", "$lt", "$lte", "$in", "$nin", "$and", "$or", "$not",
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
    validation::validate(request)?;
    execute_query_with_records(table, request, None, false)
}

pub fn execute_query_at(
    table: &DbfTable,
    dbf_path: impl AsRef<std::path::Path>,
    request: &QueryRequest,
) -> Result<Vec<Value>, QueryError> {
    validation::validate(request)?;
    let access = planner::choose(dbf_path.as_ref(), request);
    execute_query_with_records(table, request, access.records, access.ordered)
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
    ordered: bool,
) -> Result<Vec<Value>, QueryError> {
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

    if !ordered {
        records.sort_by(|left, right| compare_records(left, right, &request.sort));
    }

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
        Some(limit) => records.take(limit).collect::<Vec<_>>(),
        None => records.collect::<Vec<_>>(),
    };

    Ok(records
        .into_iter()
        .map(|record| project(record, &request.projection))
        .collect())
}

impl QueryExecutor for DbfTable {
    fn execute(&self, request: &QueryRequest) -> Result<Vec<Value>, QueryError> {
        execute_query(self, request)
    }
}

fn matches_filter(
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

fn compare_records(left: &DbfRecord, right: &DbfRecord, sort: &IndexMap<String, i8>) -> Ordering {
    for (field, direction) in sort {
        let left_value = field_value(&left.values, field);
        let right_value = field_value(&right.values, field);
        let ordering = compare_for_sort(left_value.as_ref(), right_value.as_ref());
        if ordering != Ordering::Equal {
            return if *direction == 1 {
                ordering
            } else {
                ordering.reverse()
            };
        }
    }
    Ordering::Equal
}

fn compare_for_sort(left: Option<&Value>, right: Option<&Value>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => compare_values(left, right).unwrap_or_else(|| {
            type_rank(left)
                .cmp(&type_rank(right))
                .then_with(|| left.to_string().cmp(&right.to_string()))
        }),
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
    }
}

fn compare_values(left: &Value, right: &Value) -> Option<Ordering> {
    match (left, right) {
        (Value::Array(left), Value::Array(right)) => Some(json_text(left).cmp(&json_text(right))),
        (Value::Object(left), Value::Object(right)) => Some(json_text(left).cmp(&json_text(right))),
        _ => crate::json_order::compare_scalar_values(left, right),
    }
}

fn json_text<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn type_rank(value: &Value) -> u8 {
    match value {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => 3,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
    }
}

pub trait QueryExecutor {
    fn execute(&self, request: &QueryRequest) -> Result<Vec<Value>, QueryError>;
}

#[cfg(test)]
mod malformed_tests;

#[cfg(test)]
mod tests;
