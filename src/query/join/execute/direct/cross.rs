use super::super::super::emit;
use super::super::super::{JoinError, JoinRequest, MAX_JOIN_ROWS};
use crate::dbf::DbfRecord;

pub(super) fn execute(
    request: &JoinRequest,
    left_records: &[&DbfRecord],
    right_records: &[&DbfRecord],
) -> Result<Vec<serde_json::Value>, JoinError> {
    let pair_count = left_records
        .len()
        .checked_mul(right_records.len())
        .ok_or_else(|| JoinError::Invalid("cross join candidate pair count overflows".into()))?;
    if pair_count > MAX_JOIN_ROWS {
        return Err(JoinError::Invalid(format!(
            "cross join candidate pairs exceed {MAX_JOIN_ROWS}"
        )));
    }
    let mut output = Vec::new();
    for &left_record in left_records {
        for &right_record in right_records {
            emit(&mut output, request, Some(left_record), Some(right_record))?;
        }
    }
    Ok(output)
}
