use super::super::QueryError;
use crate::dbf::DbfRecord;
use crate::query::expression::NumericExpression;
use serde_json::Value;

#[derive(Debug, Default)]
pub(super) struct State {
    mean: f64,
    sum_squared_diffs: f64,
    count: u64,
}

pub(super) fn accumulate(
    state: &mut State,
    record: &DbfRecord,
    expression: &NumericExpression,
    path: &str,
    name: &str,
) -> Result<bool, QueryError> {
    let value = crate::query::expression::evaluate_numeric(&record.values, expression, path)?;
    let Some(value) = value.and_then(|value| value.as_number().and_then(|number| number.as_f64()))
    else {
        return Ok(false);
    };
    if !value.is_finite() {
        return Err(QueryError::Invalid(format!(
            "aggregate {name} contains a non-finite number"
        )));
    }

    let next_count = state
        .count
        .checked_add(1)
        .ok_or_else(|| QueryError::Invalid(format!("aggregate {name} count overflows u64")))?;
    let delta = value - state.mean;
    let next_mean = state.mean + delta / next_count as f64;
    let next_sum_squared_diffs = state.sum_squared_diffs + delta * (value - next_mean);
    if !next_mean.is_finite() || !next_sum_squared_diffs.is_finite() {
        return Err(QueryError::Invalid(format!(
            "aggregate {name} exceeds finite JSON number range"
        )));
    }
    state.mean = next_mean;
    state.sum_squared_diffs = next_sum_squared_diffs;
    state.count = next_count;
    Ok(true)
}

pub(super) fn finish(state: State, sample: bool, name: &str) -> Result<Value, QueryError> {
    let denominator = if sample {
        state.count.checked_sub(1)
    } else {
        Some(state.count)
    };
    match denominator {
        None | Some(0) => Ok(Value::Null),
        Some(denominator) => {
            let deviation = (state.sum_squared_diffs / denominator as f64)
                .max(0.0)
                .sqrt();
            serde_json::Number::from_f64(deviation)
                .map(Value::Number)
                .ok_or_else(|| QueryError::Invalid(format!("aggregate {name} does not fit JSON")))
        }
    }
}
