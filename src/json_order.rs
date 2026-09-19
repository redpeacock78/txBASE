use serde_json::{Number, Value};
use std::cmp::Ordering;

pub(crate) fn compare_scalar_values(left: &Value, right: &Value) -> Option<Ordering> {
    match (left, right) {
        (Value::Null, Value::Null) => Some(Ordering::Equal),
        (Value::Bool(left), Value::Bool(right)) => Some(left.cmp(right)),
        (Value::Number(left), Value::Number(right)) => compare_numbers(left, right),
        (Value::String(left), Value::String(right)) => Some(left.cmp(right)),
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
