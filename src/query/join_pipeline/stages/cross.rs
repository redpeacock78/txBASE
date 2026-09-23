use super::super::super::join::{JoinError, MAX_JOIN_ROWS};
use super::super::push_combined;
use serde_json::{Map, Value};

pub(super) fn execute(
    left: &[Map<String, Value>],
    right: &[Map<String, Value>],
) -> Result<Vec<Map<String, Value>>, JoinError> {
    let pair_count = left
        .len()
        .checked_mul(right.len())
        .ok_or_else(|| JoinError::Invalid("cross join candidate pair count overflows".into()))?;
    if pair_count > MAX_JOIN_ROWS {
        return Err(JoinError::Invalid(format!(
            "cross join candidate pairs exceed {MAX_JOIN_ROWS}"
        )));
    }
    let mut output = Vec::with_capacity(pair_count);
    for left_row in left {
        for right_row in right {
            push_combined(&mut output, Some(left_row), Some(right_row))?;
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::execute;
    use serde_json::{Map, json};

    fn row(table: &str, key: i64) -> Map<String, serde_json::Value> {
        Map::from_iter([(format!("{table}.ID"), json!(key))])
    }

    #[test]
    fn emits_the_bounded_cartesian_product() {
        let left = [row("left", 1), row("left", 2)];
        let right = [row("right", 1), row("right", 2)];

        assert_eq!(execute(&left, &right).unwrap().len(), 4);
    }
}
