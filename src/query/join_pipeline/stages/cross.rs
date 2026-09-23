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
