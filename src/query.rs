use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
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
    Ok(())
}

pub trait QueryExecutor {
    fn execute(&self, request: &QueryRequest) -> Result<Vec<Value>, QueryError>;
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
