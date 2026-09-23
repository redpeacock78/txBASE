use super::super::super::join::{JoinError, JoinSpec, JoinType, MAX_JOIN_ROWS};
use super::super::{encoded_key, push_combined};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub(super) fn execute_right(
    left: &[Map<String, Value>],
    right: &[Map<String, Value>],
    local_fields: &[String],
    foreign_fields: &[String],
) -> Result<Vec<Map<String, Value>>, JoinError> {
    let mut left_by_key = BTreeMap::<String, Vec<usize>>::new();
    for (index, left_row) in left.iter().enumerate() {
        let Some(key) = encoded_key(left_row, local_fields)? else {
            continue;
        };
        left_by_key.entry(key).or_default().push(index);
    }

    let mut output = Vec::new();
    for right_row in right {
        let matches = encoded_key(right_row, foreign_fields)?.and_then(|key| left_by_key.get(&key));
        if let Some(matches) = matches {
            for &index in matches {
                push_combined(&mut output, Some(&left[index]), Some(right_row))?;
            }
        } else {
            push_combined(&mut output, None, Some(right_row))?;
        }
    }
    Ok(output)
}

pub(super) fn execute_left(
    left: &[Map<String, Value>],
    right: &[Map<String, Value>],
    spec: &JoinSpec,
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

    let mut output = Vec::new();
    for left_row in left {
        let matches = encoded_key(left_row, local_fields)?.and_then(|key| right_by_key.get(&key));
        let had_matches = matches.is_some_and(|rows| !rows.is_empty());
        match &spec.kind {
            JoinType::Inner => {
                if let Some(matches) = matches {
                    for &index in matches {
                        push_combined(&mut output, Some(left_row), Some(&right[index]))?;
                    }
                }
            }
            JoinType::Left => {
                if let Some(matches) = matches {
                    for &index in matches {
                        push_combined(&mut output, Some(left_row), Some(&right[index]))?;
                    }
                }
                if !had_matches {
                    push_combined(&mut output, Some(left_row), None)?;
                }
            }
            JoinType::Semi if had_matches => output.push(left_row.clone()),
            JoinType::Anti if !had_matches => output.push(left_row.clone()),
            JoinType::Semi | JoinType::Anti => {}
            JoinType::Right | JoinType::Full | JoinType::Cross => {
                unreachable!("join type handled above")
            }
        }
        if output.len() > MAX_JOIN_ROWS {
            return Err(JoinError::Invalid(format!(
                "join result exceeds {MAX_JOIN_ROWS} rows"
            )));
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::{execute_left, execute_right};
    use crate::query::join::{JoinSpec, JoinType};
    use serde_json::{Map, json};
    use std::collections::BTreeMap;

    fn row(table: &str, key: Option<i64>) -> Map<String, serde_json::Value> {
        let mut values = Map::new();
        if let Some(key) = key {
            values.insert(format!("{table}.ID"), json!(key));
        }
        values
    }

    fn spec(kind: JoinType) -> JoinSpec {
        JoinSpec {
            kind,
            table: "right".to_owned(),
            on: BTreeMap::new(),
        }
    }

    #[test]
    fn executes_left_hash_variants() {
        let left = [
            row("left", Some(1)),
            row("left", Some(2)),
            row("left", None),
        ];
        let right = [row("right", Some(1)), row("right", Some(3))];
        let local_fields = ["left.ID".to_owned()];
        let foreign_fields = ["right.ID".to_owned()];

        assert_eq!(
            execute_left(
                &left,
                &right,
                &spec(JoinType::Inner),
                &local_fields,
                &foreign_fields,
            )
            .unwrap()
            .len(),
            1
        );
        assert_eq!(
            execute_left(
                &left,
                &right,
                &spec(JoinType::Left),
                &local_fields,
                &foreign_fields,
            )
            .unwrap()
            .len(),
            3
        );
        assert_eq!(
            execute_left(
                &left,
                &right,
                &spec(JoinType::Semi),
                &local_fields,
                &foreign_fields,
            )
            .unwrap()
            .len(),
            1
        );
        assert_eq!(
            execute_left(
                &left,
                &right,
                &spec(JoinType::Anti),
                &local_fields,
                &foreign_fields,
            )
            .unwrap()
            .len(),
            2
        );
    }

    #[test]
    fn executes_right_hash_with_unmatched_rows() {
        let left = [row("left", Some(1))];
        let right = [row("right", Some(1)), row("right", Some(3))];
        let local_fields = ["left.ID".to_owned()];
        let foreign_fields = ["right.ID".to_owned()];

        assert_eq!(
            execute_right(&left, &right, &local_fields, &foreign_fields)
                .unwrap()
                .len(),
            2
        );
    }
}
