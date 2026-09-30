use super::super::QueryError;
use crate::query::expression::{ScalarExpression, evaluate_scalar};
use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

#[derive(Debug)]
pub(super) struct State {
    values: BinaryHeap<Entry>,
    limit: usize,
    choose_min: bool,
    next_order: usize,
}

#[derive(Debug)]
struct Entry {
    value: Value,
    order: usize,
    choose_min: bool,
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl Eq for Entry {}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        let ordering = compare_values(&self.value, &other.value);
        let ordering = if self.choose_min {
            ordering
        } else {
            ordering.reverse()
        };
        ordering.then_with(|| self.order.cmp(&other.order))
    }
}

impl State {
    pub(super) fn new(
        group_key: &Value,
        expression: &ScalarExpression,
        choose_min: bool,
        path: &str,
    ) -> Result<Self, QueryError> {
        let mut values = Map::new();
        values.insert(String::from("_id"), group_key.clone());
        let count = evaluate_scalar(&values, expression, path)?
            .and_then(|value| value.as_number().and_then(|number| number.as_f64()))
            .filter(|count| {
                count.is_finite()
                    && *count > 0.0
                    && count.fract() == 0.0
                    && *count <= super::MAX_COLLECTED_VALUES as f64
            })
            .ok_or_else(|| {
                QueryError::Invalid(format!(
                    "{path} must resolve to a positive integer no greater than {}",
                    super::MAX_COLLECTED_VALUES
                ))
            })?;

        Ok(Self {
            values: BinaryHeap::new(),
            limit: count as usize,
            choose_min,
            next_order: 0,
        })
    }

    pub(super) fn insert(
        &mut self,
        value: Value,
        collected_values: &mut usize,
    ) -> Result<(), QueryError> {
        let is_full = self.values.len() == self.limit;
        let replace_worst = is_full
            && self.values.peek().is_some_and(|worst| {
                let ordering = compare_values(&value, &worst.value);
                if self.choose_min {
                    ordering.is_lt()
                } else {
                    ordering.is_gt()
                }
            });
        if is_full && !replace_worst {
            return Ok(());
        }
        if !is_full && *collected_values >= super::MAX_COLLECTED_VALUES {
            return Err(QueryError::Invalid(format!(
                "aggregate collected value count exceeds {}",
                super::MAX_COLLECTED_VALUES
            )));
        }
        let order = self.next_order;
        self.next_order = self.next_order.checked_add(1).ok_or_else(|| {
            QueryError::Invalid("aggregate N-value input order overflows usize".into())
        })?;

        if is_full {
            self.values.pop();
        } else {
            *collected_values += 1;
        }
        self.values.push(Entry {
            value,
            order,
            choose_min: self.choose_min,
        });
        Ok(())
    }

    pub(super) fn finish(self) -> Value {
        Value::Array(
            self.values
                .into_sorted_vec()
                .into_iter()
                .map(|entry| entry.value)
                .collect(),
        )
    }
}

pub(super) fn compare_values(left: &Value, right: &Value) -> Ordering {
    let type_order = json_type_order(left).cmp(&json_type_order(right));
    if type_order != Ordering::Equal {
        return type_order;
    }

    match (left, right) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Bool(left), Value::Bool(right)) => left.cmp(right),
        (Value::Number(_), Value::Number(_)) | (Value::String(_), Value::String(_)) => {
            crate::json_order::compare_scalar_values(left, right)
                .expect("same JSON scalar types are comparable")
        }
        (Value::Array(left), Value::Array(right)) => left
            .iter()
            .zip(right)
            .map(|(left, right)| compare_values(left, right))
            .find(|ordering| *ordering != Ordering::Equal)
            .unwrap_or_else(|| left.len().cmp(&right.len())),
        (Value::Object(left), Value::Object(right)) => {
            let mut left = left.iter();
            let mut right = right.iter();
            loop {
                match (left.next(), right.next()) {
                    (Some((left_key, left_value)), Some((right_key, right_value))) => {
                        let order = json_type_order(left_value).cmp(&json_type_order(right_value));
                        if order != Ordering::Equal {
                            return order;
                        }
                        let order = left_key.cmp(right_key);
                        if order != Ordering::Equal {
                            return order;
                        }
                        let order = compare_values(left_value, right_value);
                        if order != Ordering::Equal {
                            return order;
                        }
                    }
                    (None, None) => return Ordering::Equal,
                    (None, Some(_)) => return Ordering::Less,
                    (Some(_), None) => return Ordering::Greater,
                }
            }
        }
        _ => Ordering::Equal,
    }
}

fn json_type_order(value: &Value) -> u8 {
    match value {
        Value::Null => 0,
        Value::Number(_) => 1,
        Value::String(_) => 2,
        Value::Object(_) => 3,
        Value::Array(_) => 4,
        Value::Bool(_) => 5,
    }
}
