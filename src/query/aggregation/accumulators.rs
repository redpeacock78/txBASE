use super::super::{QueryError, aggregation_plan};
use super::standard_deviation;
use crate::dbf::DbfRecord;
use crate::query_path::field_value;
use serde_json::Value;

#[derive(Debug)]
enum AccumulatorState {
    Count(u64),
    Average {
        total: f64,
        count: u64,
    },
    StandardDeviation(standard_deviation::State),
    Sum {
        integer: i128,
        floating: Option<f64>,
    },
    Min(Option<Value>),
    Max(Option<Value>),
    First(Option<Value>),
    Last(Option<Value>),
    Values(Vec<Value>),
}

#[derive(Debug)]
pub(super) struct GroupState {
    key: Value,
    accumulators: Vec<AccumulatorState>,
}

pub(super) fn new_group(key: Value, spec: &aggregation_plan::GroupSpec) -> GroupState {
    GroupState {
        key,
        accumulators: spec
            .accumulators
            .iter()
            .map(|accumulator| match &accumulator.kind {
                aggregation_plan::AccumulatorKind::Count => AccumulatorState::Count(0),
                aggregation_plan::AccumulatorKind::Average(_) => AccumulatorState::Average {
                    total: 0.0,
                    count: 0,
                },
                aggregation_plan::AccumulatorKind::StdDevPop(_)
                | aggregation_plan::AccumulatorKind::StdDevSamp(_) => {
                    AccumulatorState::StandardDeviation(standard_deviation::State::default())
                }
                aggregation_plan::AccumulatorKind::Sum(_) => AccumulatorState::Sum {
                    integer: 0,
                    floating: None,
                },
                aggregation_plan::AccumulatorKind::Min(_) => AccumulatorState::Min(None),
                aggregation_plan::AccumulatorKind::Max(_) => AccumulatorState::Max(None),
                aggregation_plan::AccumulatorKind::First(_) => AccumulatorState::First(None),
                aggregation_plan::AccumulatorKind::Last(_) => AccumulatorState::Last(None),
                aggregation_plan::AccumulatorKind::Push(_)
                | aggregation_plan::AccumulatorKind::AddToSet(_) => {
                    AccumulatorState::Values(Vec::new())
                }
            })
            .collect(),
    }
}

pub(super) fn accumulate_record(
    group: &mut GroupState,
    record: &DbfRecord,
    spec: &aggregation_plan::GroupSpec,
    collected_values: &mut usize,
) -> Result<(), QueryError> {
    for (state, accumulator) in group.accumulators.iter_mut().zip(&spec.accumulators) {
        match (state, &accumulator.kind) {
            (AccumulatorState::Count(count), aggregation_plan::AccumulatorKind::Count) => {
                *count = count
                    .checked_add(1)
                    .ok_or_else(|| QueryError::Invalid("aggregate count overflows u64".into()))?;
            }
            (
                AccumulatorState::Average { total, count },
                aggregation_plan::AccumulatorKind::Average(expression),
            ) => {
                let path = format!("$group.{}.$avg", accumulator.name);
                let Some(number) = resolve_numeric_number(record, expression, &path)? else {
                    continue;
                };
                let Some(number) = number.as_f64() else {
                    continue;
                };
                let next_total = *total + number;
                if !next_total.is_finite() {
                    return Err(QueryError::Invalid(format!(
                        "aggregate {} exceeds finite JSON number range",
                        accumulator.name
                    )));
                }
                *total = next_total;
                *count = count.checked_add(1).ok_or_else(|| {
                    QueryError::Invalid("aggregate average count overflows u64".into())
                })?;
            }
            (
                AccumulatorState::StandardDeviation(state),
                kind @ (aggregation_plan::AccumulatorKind::StdDevPop(_)
                | aggregation_plan::AccumulatorKind::StdDevSamp(_)),
            ) => {
                let (operator, expression) = match kind {
                    aggregation_plan::AccumulatorKind::StdDevPop(expression) => {
                        ("$stdDevPop", expression)
                    }
                    aggregation_plan::AccumulatorKind::StdDevSamp(expression) => {
                        ("$stdDevSamp", expression)
                    }
                    _ => unreachable!("matched standard-deviation accumulator"),
                };
                let path = format!("$group.{}.{}", accumulator.name, operator);
                if !standard_deviation::accumulate(
                    state,
                    record,
                    expression,
                    &path,
                    &accumulator.name,
                )? {
                    continue;
                }
            }
            (
                AccumulatorState::Sum { integer, floating },
                aggregation_plan::AccumulatorKind::Sum(expression),
            ) => {
                let path = format!("$group.{}.$sum", accumulator.name);
                let Some(number) = resolve_numeric_number(record, expression, &path)? else {
                    continue;
                };
                let operand_name = accumulator.name.as_str();
                if let Some(value) = number
                    .as_i64()
                    .map(i128::from)
                    .or_else(|| number.as_u64().map(i128::from))
                {
                    if let Some(total) = floating {
                        *total = add_floating_sum(*total, value as f64, operand_name)?;
                    } else {
                        *integer = integer.checked_add(value).ok_or_else(|| {
                            QueryError::Invalid("aggregate $sum overflows i128".into())
                        })?;
                    }
                } else {
                    let value = number.as_f64().ok_or_else(|| {
                        QueryError::Invalid(format!(
                            "aggregate $sum operand {operand_name} must contain a finite JSON number"
                        ))
                    })?;
                    let total =
                        add_floating_sum(floating.unwrap_or(*integer as f64), value, operand_name)?;
                    *floating = Some(total);
                };
            }
            (AccumulatorState::Min(current), aggregation_plan::AccumulatorKind::Min(field)) => {
                update_extreme(current, record, field, true)?;
            }
            (AccumulatorState::Max(current), aggregation_plan::AccumulatorKind::Max(field)) => {
                update_extreme(current, record, field, false)?;
            }
            (AccumulatorState::First(current), aggregation_plan::AccumulatorKind::First(field)) => {
                if current.is_none() {
                    *current = Some(field_value(&record.values, field).unwrap_or(Value::Null));
                }
            }
            (AccumulatorState::Last(current), aggregation_plan::AccumulatorKind::Last(field)) => {
                *current = Some(field_value(&record.values, field).unwrap_or(Value::Null));
            }
            (AccumulatorState::Values(values), aggregation_plan::AccumulatorKind::Push(field)) => {
                append_collected_value(
                    values,
                    field_value(&record.values, field).unwrap_or(Value::Null),
                    false,
                    collected_values,
                )?;
            }
            (
                AccumulatorState::Values(values),
                aggregation_plan::AccumulatorKind::AddToSet(field),
            ) => {
                append_collected_value(
                    values,
                    field_value(&record.values, field).unwrap_or(Value::Null),
                    true,
                    collected_values,
                )?;
            }
            _ => unreachable!("validated accumulator state and specification differ"),
        }
    }
    Ok(())
}

fn resolve_numeric_number(
    record: &DbfRecord,
    expression: &crate::query::expression::NumericExpression,
    path: &str,
) -> Result<Option<serde_json::Number>, QueryError> {
    let value = crate::query::expression::evaluate_numeric(&record.values, expression, path)?;
    Ok(value.and_then(|value| value.as_number().cloned()))
}

pub(super) fn finish_group(
    group: GroupState,
    spec: &aggregation_plan::GroupSpec,
) -> Result<Value, QueryError> {
    let mut output = serde_json::Map::new();
    output.insert(String::from("_id"), group.key);
    for (state, accumulator) in group.accumulators.into_iter().zip(&spec.accumulators) {
        let value = match state {
            AccumulatorState::Count(count) => Value::Number(count.into()),
            AccumulatorState::Average { total, count } => {
                if count == 0 {
                    Value::Null
                } else {
                    let average = total / count as f64;
                    serde_json::Number::from_f64(average)
                        .map(Value::Number)
                        .ok_or_else(|| {
                            QueryError::Invalid(format!(
                                "aggregate {} does not fit JSON",
                                accumulator.name
                            ))
                        })?
                }
            }
            AccumulatorState::StandardDeviation(state) => {
                let sample = matches!(
                    &accumulator.kind,
                    aggregation_plan::AccumulatorKind::StdDevSamp(_)
                );
                standard_deviation::finish(state, sample, &accumulator.name)?
            }
            AccumulatorState::Sum { integer, floating } => match floating {
                Some(total) => serde_json::Number::from_f64(total)
                    .map(Value::Number)
                    .ok_or_else(|| {
                        QueryError::Invalid(format!(
                            "aggregate {} does not fit JSON",
                            accumulator.name
                        ))
                    })?,
                None => Value::Number(number_from_i128(integer, &accumulator.name)?),
            },
            AccumulatorState::Min(value)
            | AccumulatorState::Max(value)
            | AccumulatorState::First(value)
            | AccumulatorState::Last(value) => value.unwrap_or(Value::Null),
            AccumulatorState::Values(values) => Value::Array(values),
        };
        output.insert(accumulator.name.clone(), value);
    }
    Ok(Value::Object(output))
}

fn append_collected_value(
    values: &mut Vec<Value>,
    value: Value,
    distinct: bool,
    collected_values: &mut usize,
) -> Result<(), QueryError> {
    // ponytail: bounded arrays make linear JSON equality sufficient; use a keyed set only if this limit grows.
    if distinct && values.iter().any(|existing| existing == &value) {
        return Ok(());
    }
    if *collected_values >= super::MAX_COLLECTED_VALUES {
        return Err(QueryError::Invalid(format!(
            "aggregate collected value count exceeds {}",
            super::MAX_COLLECTED_VALUES
        )));
    }
    values.push(value);
    *collected_values += 1;
    Ok(())
}

fn add_floating_sum(total: f64, value: f64, field: &str) -> Result<f64, QueryError> {
    let total = total + value;
    total.is_finite().then_some(total).ok_or_else(|| {
        QueryError::Invalid(format!(
            "aggregate $sum field {field} exceeds finite JSON number range"
        ))
    })
}

fn update_extreme(
    current: &mut Option<Value>,
    record: &DbfRecord,
    field: &str,
    choose_min: bool,
) -> Result<(), QueryError> {
    let Some(value) = field_value(&record.values, field) else {
        return Ok(());
    };
    if value.is_null() {
        return Ok(());
    }
    let Some(current_value) = current.as_ref() else {
        *current = Some(value);
        return Ok(());
    };
    let ordering =
        super::super::ordering::compare_values(current_value, &value).ok_or_else(|| {
            QueryError::Invalid(format!(
                "aggregate extreme field {field} contains incomparable values"
            ))
        })?;
    let replace = if choose_min {
        ordering.is_gt()
    } else {
        ordering.is_lt()
    };
    if replace {
        *current = Some(value);
    }
    Ok(())
}

fn number_from_i128(value: i128, name: &str) -> Result<serde_json::Number, QueryError> {
    if value >= 0 {
        u64::try_from(value)
            .map(serde_json::Number::from)
            .map_err(|_| QueryError::Invalid(format!("aggregate {name} does not fit JSON")))
    } else {
        i64::try_from(value)
            .map(serde_json::Number::from)
            .map_err(|_| QueryError::Invalid(format!("aggregate {name} does not fit JSON")))
    }
}
