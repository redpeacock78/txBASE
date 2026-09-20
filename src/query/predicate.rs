use super::{QueryError, ordering::compare_values};
use crate::query_path::field_value;
use serde_json::{Map, Value};
use std::cmp::Ordering;

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
            "$expr" => super::expression::matches(values, condition)?,
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

pub(crate) fn matches_condition(
    actual: Option<&Value>,
    condition: &Value,
) -> Result<bool, QueryError> {
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
