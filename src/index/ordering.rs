use super::IndexKey;
use crate::Collation;
use crate::json_order::compare_scalar_values;
use serde_json::Value;
use std::cmp::Ordering;

pub(super) fn compare_keys(left: &IndexKey, right: &IndexKey) -> Ordering {
    compare_keys_with_collation(left, right, None)
}

fn compare_keys_with_collation(
    left: &IndexKey,
    right: &IndexKey,
    collation: Option<Collation>,
) -> Ordering {
    match (left, right) {
        (IndexKey::Missing, IndexKey::Missing) | (IndexKey::Null, IndexKey::Null) => {
            Ordering::Equal
        }
        (IndexKey::Missing, _) => Ordering::Less,
        (_, IndexKey::Missing) => Ordering::Greater,
        (IndexKey::Null, _) => Ordering::Less,
        (_, IndexKey::Null) => Ordering::Greater,
        (IndexKey::Scalar(left), IndexKey::Scalar(right)) => match (left, right, collation) {
            (Value::String(left), Value::String(right), Some(collation)) => {
                collation.compare(left, right)
            }
            _ => compare_scalar_values(left, right)
                .unwrap_or_else(|| scalar_type_rank(left).cmp(&scalar_type_rank(right))),
        },
        (IndexKey::Compound(left), IndexKey::Compound(right)) => left
            .iter()
            .zip(right)
            .map(|(left, right)| compare_keys_with_collation(left, right, collation))
            .find(|ordering| !ordering.is_eq())
            .unwrap_or_else(|| left.len().cmp(&right.len())),
        (IndexKey::Compound(_), _) => Ordering::Greater,
        (_, IndexKey::Compound(_)) => Ordering::Less,
    }
}

pub(super) fn compare_index_keys(
    left: &IndexKey,
    right: &IndexKey,
    directions: &[i8],
    collation: Option<Collation>,
) -> Ordering {
    match (left, right) {
        (IndexKey::Compound(left), IndexKey::Compound(right)) => left
            .iter()
            .zip(right)
            .enumerate()
            .map(|(position, (left, right))| {
                let ordering = compare_keys_with_collation(left, right, collation);
                if directions.get(position) == Some(&-1) {
                    ordering.reverse()
                } else {
                    ordering
                }
            })
            .find(|ordering| !ordering.is_eq())
            .unwrap_or_else(|| left.len().cmp(&right.len())),
        _ => compare_keys_with_collation(left, right, collation),
    }
}

pub(super) fn key_domain(key: &IndexKey) -> u8 {
    match key {
        IndexKey::Missing => 0,
        IndexKey::Null => 1,
        IndexKey::Scalar(value) => value_domain(value),
        IndexKey::Compound(_) => 5,
    }
}

pub(super) fn value_domain(value: &Value) -> u8 {
    match value {
        Value::Null => 1,
        Value::Bool(_) => 2,
        Value::Number(_) => 3,
        Value::String(_) => 4,
        Value::Array(_) => 5,
        Value::Object(_) => 6,
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

#[cfg(test)]
mod tests {
    use super::super::IndexKey;
    use super::compare_index_keys;
    use crate::Collation;
    use serde_json::Value;
    use std::cmp::Ordering;

    #[test]
    fn locale_index_keys_follow_collation_and_field_direction() {
        let a = IndexKey::Scalar(Value::String("阿".into()));
        let z = IndexKey::Scalar(Value::String("中".into()));
        assert_eq!(
            compare_index_keys(&a, &z, &[1], Some(Collation::Icu4x211Zh)),
            Ordering::Less
        );
        assert_eq!(
            compare_index_keys(&a, &z, &[-1], Some(Collation::Icu4x211Zh)),
            Ordering::Greater
        );
        assert_eq!(compare_index_keys(&a, &z, &[1], None), Ordering::Greater);
    }
}
