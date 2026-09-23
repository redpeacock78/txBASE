use super::super::super::join::{JoinError, JoinSpec, JoinType};
use super::super::{push_combined, qualified_value};
use crate::json_order::compare_scalar_values;
use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ops::Range;

struct OrderedRow {
    position: usize,
    key: Vec<Value>,
}

pub(super) fn execute(
    left: &[Map<String, Value>],
    right: &[Map<String, Value>],
    right_numbers: &[usize],
    right_order: &[usize],
    spec: &JoinSpec,
    local_fields: &[String],
    foreign_fields: &[String],
) -> Result<Vec<Map<String, Value>>, JoinError> {
    if right.len() != right_numbers.len() {
        return Err(JoinError::Invalid(
            "join index row numbers do not match loaded rows".into(),
        ));
    }
    let left_sorted = ordered_left(left, local_fields);
    let right_sorted = ordered_right(right, right_numbers, right_order, foreign_fields)?;
    let left_matches = matching_ranges(&left_sorted, &right_sorted, left.len());
    let right_matches = matching_ranges(&right_sorted, &left_sorted, right.len());

    let mut output = Vec::new();
    match &spec.kind {
        JoinType::Inner => {
            for (left_position, left_row) in left.iter().enumerate() {
                if let Some(right_range) = left_matches[left_position].as_ref() {
                    for right_row in &right_sorted[right_range.start..right_range.end] {
                        push_combined(
                            &mut output,
                            Some(left_row),
                            Some(&right[right_row.position]),
                        )?;
                    }
                }
            }
        }
        JoinType::Left => {
            for (left_position, left_row) in left.iter().enumerate() {
                if let Some(right_range) = left_matches[left_position].as_ref() {
                    for right_row in &right_sorted[right_range.start..right_range.end] {
                        push_combined(
                            &mut output,
                            Some(left_row),
                            Some(&right[right_row.position]),
                        )?;
                    }
                } else {
                    push_combined(&mut output, Some(left_row), None)?;
                }
            }
        }
        JoinType::Semi => {
            for (left_position, left_row) in left.iter().enumerate() {
                if left_matches[left_position].is_some() {
                    push_combined(&mut output, Some(left_row), None)?;
                }
            }
        }
        JoinType::Anti => {
            for (left_position, left_row) in left.iter().enumerate() {
                if left_matches[left_position].is_none() {
                    push_combined(&mut output, Some(left_row), None)?;
                }
            }
        }
        JoinType::Right => {
            for (right_position, right_row) in right.iter().enumerate() {
                if let Some(left_range) = right_matches[right_position].as_ref() {
                    for left_row in &left_sorted[left_range.start..left_range.end] {
                        push_combined(
                            &mut output,
                            Some(&left[left_row.position]),
                            Some(right_row),
                        )?;
                    }
                } else {
                    push_combined(&mut output, None, Some(right_row))?;
                }
            }
        }
        JoinType::Full | JoinType::Cross => {
            return Err(JoinError::Invalid(
                "ordered merge does not support this join type".into(),
            ));
        }
    }
    Ok(output)
}

fn ordered_left(rows: &[Map<String, Value>], fields: &[String]) -> Vec<OrderedRow> {
    let mut ordered = rows
        .iter()
        .enumerate()
        .filter_map(|(position, row)| {
            merge_key(row, fields).map(|key| OrderedRow { position, key })
        })
        .collect::<Vec<_>>();
    ordered.sort_by(|left, right| compare_keys(&left.key, &right.key));
    ordered
}

fn ordered_right(
    rows: &[Map<String, Value>],
    numbers: &[usize],
    order: &[usize],
    fields: &[String],
) -> Result<Vec<OrderedRow>, JoinError> {
    let positions = numbers
        .iter()
        .enumerate()
        .map(|(position, number)| (*number, position))
        .collect::<BTreeMap<_, _>>();
    let mut ordered = Vec::with_capacity(order.len());
    for number in order {
        let Some(position) = positions.get(number).copied() else {
            return Err(JoinError::Invalid(format!(
                "join index references missing active row: {number}"
            )));
        };
        if let Some(key) = merge_key(&rows[position], fields) {
            ordered.push(OrderedRow { position, key });
        }
    }
    Ok(ordered)
}

fn merge_key(values: &Map<String, Value>, fields: &[String]) -> Option<Vec<Value>> {
    fields
        .iter()
        .map(|field| {
            let value = qualified_value(values, field)?;
            matches!(&value, Value::Bool(_) | Value::Number(_) | Value::String(_)).then_some(value)
        })
        .collect()
}

fn matching_ranges(
    outer_sorted: &[OrderedRow],
    inner_sorted: &[OrderedRow],
    outer_count: usize,
) -> Vec<Option<Range<usize>>> {
    let mut matches = (0..outer_count).map(|_| None).collect::<Vec<_>>();
    let mut outer_start = 0;
    let mut inner_start = 0;
    while outer_start < outer_sorted.len() && inner_start < inner_sorted.len() {
        match compare_keys(
            &outer_sorted[outer_start].key,
            &inner_sorted[inner_start].key,
        ) {
            Ordering::Less => outer_start += 1,
            Ordering::Greater => inner_start += 1,
            Ordering::Equal => {
                let outer_end = key_end(outer_sorted, outer_start);
                let inner_end = key_end(inner_sorted, inner_start);
                for row in &outer_sorted[outer_start..outer_end] {
                    matches[row.position] = Some(inner_start..inner_end);
                }
                outer_start = outer_end;
                inner_start = inner_end;
            }
        }
    }
    matches
}

fn key_end(rows: &[OrderedRow], start: usize) -> usize {
    let key = &rows[start].key;
    let mut end = start + 1;
    while end < rows.len() && compare_keys(&rows[end].key, key) == Ordering::Equal {
        end += 1;
    }
    end
}

fn compare_keys(left: &[Value], right: &[Value]) -> Ordering {
    left.iter()
        .zip(right)
        .map(|(left, right)| compare_values(left, right))
        .find(|ordering| !ordering.is_eq())
        .unwrap_or_else(|| left.len().cmp(&right.len()))
}

fn compare_values(left: &Value, right: &Value) -> Ordering {
    compare_scalar_values(left, right).unwrap_or_else(|| value_rank(left).cmp(&value_rank(right)))
}

fn value_rank(value: &Value) -> u8 {
    match value {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => 3,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::execute;
    use crate::query::join::{JoinSpec, JoinType};
    use serde_json::{Map, json};
    use std::collections::BTreeMap;

    fn row(table: &str, key: i64) -> Map<String, serde_json::Value> {
        Map::from_iter([(format!("{table}.ID"), json!(key))])
    }

    fn spec(kind: JoinType) -> JoinSpec {
        JoinSpec {
            kind,
            table: "right".to_owned(),
            on: BTreeMap::new(),
        }
    }

    #[test]
    fn preserves_left_order_and_matches_index_order() {
        let left = [row("left", 2), row("left", 1), row("left", 9)];
        let right = [row("right", 1), row("right", 2), row("right", 7)];
        let numbers = [10, 11, 12];
        let order = [11, 10, 12];
        let local = ["left.ID".to_owned()];
        let foreign = ["right.ID".to_owned()];

        let output = execute(
            &left,
            &right,
            &numbers,
            &order,
            &spec(JoinType::Left),
            &local,
            &foreign,
        )
        .unwrap();

        assert_eq!(output.len(), 3);
        assert_eq!(output[0]["left.ID"], 2);
        assert_eq!(output[0]["right.ID"], 2);
        assert_eq!(output[1]["left.ID"], 1);
        assert_eq!(output[1]["right.ID"], 1);
        assert_eq!(output[2]["left.ID"], 9);
        assert!(output[2].get("right.ID").is_none());
    }

    #[test]
    fn emits_right_major_rows_for_right_join() {
        let left = [row("left", 2), row("left", 1)];
        let right = [row("right", 1), row("right", 3)];
        let numbers = [20, 21];
        let order = [20, 21];
        let local = ["left.ID".to_owned()];
        let foreign = ["right.ID".to_owned()];

        let output = execute(
            &left,
            &right,
            &numbers,
            &order,
            &spec(JoinType::Right),
            &local,
            &foreign,
        )
        .unwrap();

        assert_eq!(output.len(), 2);
        assert_eq!(output[0]["right.ID"], 1);
        assert_eq!(output[0]["left.ID"], 1);
        assert_eq!(output[1]["right.ID"], 3);
        assert!(output[1].get("left.ID").is_none());
    }
}
