use crate::dbf::{DbfRecord, DbfTable};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

pub const JSON_QUERY_MEDIA_TYPE: &str = "application/json";
pub const SUPPORTED_FILTER_OPERATORS: &[&str] = &[
    "$eq", "$ne", "$gt", "$gte", "$lt", "$lte", "$in", "$nin", "$and", "$or", "$not",
];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QueryRequest {
    #[serde(default)]
    pub filter: Map<String, Value>,
    #[serde(default)]
    pub sort: BTreeMap<String, i8>,
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
    validate(&request)?;
    Ok(request)
}

fn validate(request: &QueryRequest) -> Result<(), QueryError> {
    for (field, direction) in &request.sort {
        if !matches!(direction, -1 | 1) {
            return Err(QueryError::Invalid(format!(
                "sort direction for {field} must be 1 or -1"
            )));
        }
    }
    for (field, inclusion) in &request.projection {
        if !matches!(inclusion, 0 | 1) {
            return Err(QueryError::Invalid(format!(
                "projection value for {field} must be 0 or 1"
            )));
        }
    }
    let has_inclusion = request.projection.values().any(|value| *value == 1);
    let has_exclusion = request.projection.values().any(|value| *value == 0);
    if has_inclusion && has_exclusion {
        return Err(QueryError::Invalid(
            "projection cannot mix inclusion and exclusion".into(),
        ));
    }
    validate_filter(&request.filter, "filter")
}

fn validate_filter(filter: &Map<String, Value>, path: &str) -> Result<(), QueryError> {
    for (field, condition) in filter {
        match field.as_str() {
            "$and" | "$or" => {
                let clauses = condition.as_array().ok_or_else(|| {
                    QueryError::Invalid(format!("{path}.{field} must be an array"))
                })?;
                for (index, clause) in clauses.iter().enumerate() {
                    let clause = clause.as_object().ok_or_else(|| {
                        QueryError::Invalid(format!("{path}.{field}[{index}] must be an object"))
                    })?;
                    validate_filter(clause, &format!("{path}.{field}[{index}]"))?;
                }
            }
            "$not" => {
                let clause = condition
                    .as_object()
                    .ok_or_else(|| QueryError::Invalid(format!("{path}.$not must be an object")))?;
                validate_filter(clause, &format!("{path}.$not"))?;
            }
            field if field.starts_with('$') => {
                return Err(QueryError::Invalid(format!(
                    "unsupported logical operator {field}"
                )));
            }
            _ => validate_condition(condition, &format!("{path}.{field}"))?,
        }
    }
    Ok(())
}

fn validate_condition(condition: &Value, path: &str) -> Result<(), QueryError> {
    let Some(operators) = condition.as_object() else {
        return Ok(());
    };
    let has_operator = operators.keys().any(|key| key.starts_with('$'));
    if !has_operator {
        return Ok(());
    }
    if operators.keys().any(|key| !key.starts_with('$')) {
        return Err(QueryError::Invalid(format!(
            "{path} cannot mix operators and fields"
        )));
    }
    for (operator, operand) in operators {
        match operator.as_str() {
            "$eq" | "$ne" | "$gt" | "$gte" | "$lt" | "$lte" => {}
            "$in" | "$nin" => {
                if !operand.is_array() {
                    return Err(QueryError::Invalid(format!(
                        "{path}.{operator} must be an array"
                    )));
                }
            }
            "$not" => {
                if !operand.is_object() {
                    return Err(QueryError::Invalid(format!(
                        "{path}.$not must be an object"
                    )));
                }
                validate_condition(operand, &format!("{path}.$not"))?;
            }
            _ => {
                return Err(QueryError::Invalid(format!(
                    "unsupported operator {operator} at {path}"
                )));
            }
        }
    }
    Ok(())
}

pub fn execute_query(table: &DbfTable, request: &QueryRequest) -> Result<Vec<Value>, QueryError> {
    validate(request)?;
    let mut records = Vec::new();
    for record in table.active_records() {
        if matches_filter(&record.values, &request.filter)? {
            records.push(record);
        }
    }

    records.sort_by(|left, right| compare_records(left, right, &request.sort));

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
            _ => matches_condition(values.get(field), condition)?,
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

fn compare_records(left: &DbfRecord, right: &DbfRecord, sort: &BTreeMap<String, i8>) -> Ordering {
    for (field, direction) in sort {
        let ordering = compare_for_sort(left.values.get(field), right.values.get(field));
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
        (Value::Null, Value::Null) => Some(Ordering::Equal),
        (Value::Bool(left), Value::Bool(right)) => Some(left.cmp(right)),
        (Value::Number(left), Value::Number(right)) => compare_numbers(left, right),
        (Value::String(left), Value::String(right)) => Some(left.cmp(right)),
        (Value::Array(left), Value::Array(right)) => Some(json_text(left).cmp(&json_text(right))),
        (Value::Object(left), Value::Object(right)) => Some(json_text(left).cmp(&json_text(right))),
        _ => None,
    }
}

fn compare_numbers(left: &Number, right: &Number) -> Option<Ordering> {
    if let (Some(left), Some(right)) = (left.as_i64(), right.as_i64()) {
        return Some(left.cmp(&right));
    }
    if let (Some(left), Some(right)) = (left.as_u64(), right.as_u64()) {
        return Some(left.cmp(&right));
    }
    if let (Some(left), Some(right)) = (left.as_i64(), right.as_u64()) {
        return Some(if left < 0 {
            Ordering::Less
        } else {
            (left as u64).cmp(&right)
        });
    }
    if let (Some(left), Some(right)) = (left.as_u64(), right.as_i64()) {
        return Some(if right < 0 {
            Ordering::Greater
        } else {
            left.cmp(&(right as u64))
        });
    }
    left.as_f64()?.partial_cmp(&right.as_f64()?)
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

fn project(record: &DbfRecord, projection: &BTreeMap<String, i8>) -> Value {
    if projection.is_empty() {
        return Value::Object(record.values.clone());
    }
    if projection.values().any(|value| *value == 1) {
        return Value::Object(
            projection
                .iter()
                .filter(|(_, inclusion)| **inclusion == 1)
                .filter_map(|(field, _)| {
                    record
                        .values
                        .get(field)
                        .map(|value| (field.clone(), value.clone()))
                })
                .collect(),
        );
    }
    let mut values = record.values.clone();
    for field in projection
        .iter()
        .filter_map(|(field, exclusion)| (*exclusion == 0).then_some(field))
    {
        values.remove(field);
    }
    Value::Object(values)
}

pub trait QueryExecutor {
    fn execute(&self, request: &QueryRequest) -> Result<Vec<Value>, QueryError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_with_two_active_records() -> DbfTable {
        let mut bytes = include_str!("../tests/fixtures/users.dbf.hex")
            .split_whitespace()
            .map(|token| u8::from_str_radix(token, 16).unwrap())
            .collect::<Vec<_>>();
        bytes[179] = b' ';
        DbfTable::from_bytes(&bytes).unwrap()
    }

    #[test]
    fn parses_query_shape() {
        let query = parse(
            br#"{
                "filter": {"age": {"$gte": 20}},
                "sort": {"age": 1},
                "projection": {"name": 1},
                "limit": 10
            }"#,
        )
        .unwrap();

        assert_eq!(query.sort["age"], 1);
        assert_eq!(query.limit, Some(10));
    }

    #[test]
    fn rejects_invalid_sort_direction() {
        let error = parse(br#"{"sort":{"age":2}}"#).unwrap_err();
        assert!(error.to_string().contains("must be 1 or -1"));
    }

    #[test]
    fn executes_filter_sort_projection_and_pagination() {
        let table = table_with_two_active_records();
        let request = parse(
            br#"{
                "filter": {"AGE": {"$gte": 7}},
                "sort": {"AGE": -1},
                "projection": {"NAME": 1, "AGE": 1},
                "skip": 1,
                "limit": 1
            }"#,
        )
        .unwrap();

        assert_eq!(
            execute_query(&table, &request).unwrap(),
            vec![serde_json::json!({
                "NAME": "Bob",
                "AGE": 7
            })]
        );
    }

    #[test]
    fn executes_logical_and_membership_predicates() {
        let table = table_with_two_active_records();
        let request =
            parse(br#"{"filter":{"$and":[{"AGE":{"$in":[29]}},{"ACTIVE":true}]}}"#).unwrap();

        assert_eq!(execute_query(&table, &request).unwrap().len(), 1);
    }

    #[test]
    fn rejects_unknown_and_mixed_projection_operators() {
        assert!(parse(br#"{"filter":{"AGE":{"$regex":"2"}}}"#).is_err());
        assert!(parse(br#"{"projection":{"NAME":1,"AGE":0}}"#).is_err());
    }

    #[test]
    fn compares_large_integer_values_exactly() {
        let maximum = Value::Number(Number::from(u64::MAX));
        let condition = serde_json::json!({"$gt": u64::MAX - 1});

        assert!(matches_condition(Some(&maximum), &condition).unwrap());
    }
}
