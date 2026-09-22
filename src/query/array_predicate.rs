use super::{
    QueryError,
    predicate::{equality_matches, matches_condition, matches_filter},
};
use serde_json::Value;

pub(super) fn matches_all(actual: Option<&Value>, expected: &Value) -> Result<bool, QueryError> {
    let expected = expected
        .as_array()
        .ok_or_else(|| QueryError::Invalid("$all must be an array".into()))?;
    let Some(actual) = actual.and_then(Value::as_array) else {
        return Ok(false);
    };
    if expected.is_empty() {
        return Ok(false);
    }
    for candidate in expected {
        if let Some(criteria) = candidate
            .as_object()
            .filter(|object| object.len() == 1)
            .and_then(|object| object.get("$elemMatch"))
        {
            let mut matched = false;
            for element in actual {
                if matches_element(element, criteria)? {
                    matched = true;
                    break;
                }
            }
            if !matched {
                return Ok(false);
            }
        } else if !actual
            .iter()
            .any(|value| equality_matches(value, candidate))
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn matches_elem_match(
    actual: Option<&Value>,
    expected: &Value,
) -> Result<bool, QueryError> {
    let criteria = expected
        .as_object()
        .ok_or_else(|| QueryError::Invalid("$elemMatch must be an object".into()))?;
    let Some(actual) = actual.and_then(Value::as_array) else {
        return Ok(false);
    };
    for element in actual {
        if matches_element(element, &Value::Object(criteria.clone()))? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn matches_size(actual: Option<&Value>, expected: &Value) -> Result<bool, QueryError> {
    let size = expected
        .as_u64()
        .and_then(|size| usize::try_from(size).ok())
        .ok_or_else(|| QueryError::Invalid("$size must be a non-negative integer".into()))?;
    Ok(actual
        .and_then(Value::as_array)
        .is_some_and(|values| values.len() == size))
}

fn matches_element(element: &Value, criteria: &Value) -> Result<bool, QueryError> {
    let criteria = criteria
        .as_object()
        .ok_or_else(|| QueryError::Invalid("$elemMatch must be an object".into()))?;
    if criteria.is_empty() {
        return Ok(true);
    }
    if criteria.keys().all(|key| key.starts_with('$')) {
        matches_condition(Some(element), &Value::Object(criteria.clone()))
    } else {
        let Some(fields) = element.as_object() else {
            return Ok(false);
        };
        matches_filter(fields, criteria)
    }
}
