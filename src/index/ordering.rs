use super::IndexKey;
use crate::json_order::compare_scalar_values;
use serde_json::Value;
use std::cmp::Ordering;

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
