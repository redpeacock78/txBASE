use super::super::super::join::JoinError;
use super::super::{encoded_key, push_combined};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub(super) fn execute(
    left: Vec<Map<String, Value>>,
    right: &[Map<String, Value>],
    local_fields: &[String],
    foreign_fields: &[String],
) -> Result<Vec<Map<String, Value>>, JoinError> {
    let mut right_by_key = BTreeMap::<String, Vec<usize>>::new();
    for (index, right_row) in right.iter().enumerate() {
        let Some(key) = encoded_key(right_row, foreign_fields)? else {
            continue;
        };
        right_by_key.entry(key).or_default().push(index);
    }

    let mut matched_right = vec![false; right.len()];
    let mut output = Vec::new();
    for left_row in &left {
        let matches = encoded_key(left_row, local_fields)?.and_then(|key| right_by_key.get(&key));
        if let Some(matches) = matches {
            for &index in matches {
                matched_right[index] = true;
                push_combined(&mut output, Some(left_row), Some(&right[index]))?;
            }
        } else {
            push_combined(&mut output, Some(left_row), None)?;
        }
    }

    for (index, right_row) in right.iter().enumerate() {
        if !matched_right[index] {
            push_combined(&mut output, None, Some(right_row))?;
        }
    }
    Ok(output)
}
