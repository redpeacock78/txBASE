use super::IndexKey;
use crate::json_order::compare_scalar_values;
use serde_json::Value;
use std::cmp::Ordering;

pub(super) fn compare_keys(left: &IndexKey, right: &IndexKey) -> Ordering {
    match (left, right) {
        (IndexKey::Missing, IndexKey::Missing) | (IndexKey::Null, IndexKey::Null) => {
            Ordering::Equal
        }
        (IndexKey::Missing, _) => Ordering::Less,
        (_, IndexKey::Missing) => Ordering::Greater,
        (IndexKey::Null, _) => Ordering::Less,
        (_, IndexKey::Null) => Ordering::Greater,
        (IndexKey::Scalar(left), IndexKey::Scalar(right)) => compare_scalar_values(left, right)
            .unwrap_or_else(|| scalar_type_rank(left).cmp(&scalar_type_rank(right))),
    }
}

pub(super) fn compare_key_to_value(key: &IndexKey, value: &Value) -> Option<Ordering> {
    match key {
        IndexKey::Missing => None,
        IndexKey::Null if value.is_null() => Some(Ordering::Equal),
        IndexKey::Scalar(actual) => compare_scalar_values(actual, value),
        _ => None,
    }
}

pub(super) fn in_range(
    key: &IndexKey,
    lower: Option<(&Value, bool)>,
    upper: Option<(&Value, bool)>,
) -> bool {
    if let Some((value, inclusive)) = lower {
        let Some(ordering) = compare_key_to_value(key, value) else {
            return false;
        };
        if !(inclusive && ordering.is_ge() || !inclusive && ordering.is_gt()) {
            return false;
        }
    }
    if let Some((value, inclusive)) = upper {
        let Some(ordering) = compare_key_to_value(key, value) else {
            return false;
        };
        if !(inclusive && ordering.is_le() || !inclusive && ordering.is_lt()) {
            return false;
        }
    }
    true
}

fn scalar_type_rank(value: &Value) -> u8 {
    match value {
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => 3,
        Value::Null => 0,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
    }
}
