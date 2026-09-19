use super::{QueryError, aggregation_plan, matches_filter};
use crate::dbf::DbfRecord;
use crate::query_path::field_value;
use indexmap::IndexMap;
use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::collections::BTreeMap;

pub(super) const MAX_GROUPS: usize = 10_000;
pub(super) use super::aggregation_plan::validate;

#[derive(Debug)]
enum AccumulatorState {
    Count(u64),
    Sum(i128),
    Min(Option<Value>),
    Max(Option<Value>),
}

#[derive(Debug)]
struct GroupState {
    key: Value,
    accumulators: Vec<AccumulatorState>,
}

pub(super) fn execute(
    records: &[&DbfRecord],
    stages: &[Map<String, Value>],
) -> Result<Vec<Value>, QueryError> {
    let plan = aggregation_plan::parse(stages)?;
    let mut records = records.to_vec();
    for filter in &plan.matches {
        let mut filtered = Vec::with_capacity(records.len());
        for record in records {
            if matches_filter(&record.values, filter)? {
                filtered.push(record);
            }
        }
        records = filtered;
    }

    let spec = &plan.group;
    let mut groups = BTreeMap::<String, GroupState>::new();
    if spec.key_field.is_none() {
        groups.insert(String::from("null"), new_group(Value::Null, spec));
    }

    for record in records {
        let key = spec
            .key_field
            .as_deref()
            .map(|field| field_value(&record.values, field).unwrap_or(Value::Null))
            .unwrap_or(Value::Null);
        let encoded_key = serde_json::to_string(&key)
            .map_err(|error| QueryError::Invalid(format!("group key encoding failed: {error}")))?;
        if !groups.contains_key(&encoded_key) {
            if groups.len() >= MAX_GROUPS {
                return Err(QueryError::Invalid(format!(
                    "aggregate group count exceeds {MAX_GROUPS}"
                )));
            }
            groups.insert(encoded_key.clone(), new_group(key, spec));
        }
        let group = groups
            .get_mut(&encoded_key)
            .expect("group was inserted or already present");
        for (state, accumulator) in group.accumulators.iter_mut().zip(&spec.accumulators) {
            match (state, &accumulator.kind) {
                (AccumulatorState::Count(count), aggregation_plan::AccumulatorKind::Count) => {
                    *count = count.checked_add(1).ok_or_else(|| {
                        QueryError::Invalid("aggregate count overflows u64".into())
                    })?;
                }
                (AccumulatorState::Sum(total), aggregation_plan::AccumulatorKind::Sum(field)) => {
                    let Some(value) = field_value(&record.values, field) else {
                        continue;
                    };
                    if value.is_null() {
                        continue;
                    }
                    let Some(number) = value.as_number() else {
                        continue;
                    };
                    let integer = number
                        .as_i64()
                        .map(i128::from)
                        .or_else(|| number.as_u64().map(i128::from));
                    let Some(integer) = integer else {
                        return Err(QueryError::Invalid(format!(
                            "aggregate $sum field {field} must contain integer numbers"
                        )));
                    };
                    *total = total.checked_add(integer).ok_or_else(|| {
                        QueryError::Invalid("aggregate $sum overflows i128".into())
                    })?;
                }
                (AccumulatorState::Min(current), aggregation_plan::AccumulatorKind::Min(field)) => {
                    update_extreme(current, record, field, true)?;
                }
                (AccumulatorState::Max(current), aggregation_plan::AccumulatorKind::Max(field)) => {
                    update_extreme(current, record, field, false)?;
                }
                _ => unreachable!("validated accumulator state and specification differ"),
            }
        }
    }

    let mut output = groups
        .into_values()
        .map(|group| finish_group(group, spec))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(sort) = &plan.sort {
        output.sort_by(|left, right| compare_output_values(left, right, sort));
    }
    Ok(output)
}

fn new_group(key: Value, spec: &aggregation_plan::GroupSpec) -> GroupState {
    GroupState {
        key,
        accumulators: spec
            .accumulators
            .iter()
            .map(|accumulator| match &accumulator.kind {
                aggregation_plan::AccumulatorKind::Count => AccumulatorState::Count(0),
                aggregation_plan::AccumulatorKind::Sum(_) => AccumulatorState::Sum(0),
                aggregation_plan::AccumulatorKind::Min(_) => AccumulatorState::Min(None),
                aggregation_plan::AccumulatorKind::Max(_) => AccumulatorState::Max(None),
            })
            .collect(),
    }
}

fn finish_group(
    group: GroupState,
    spec: &aggregation_plan::GroupSpec,
) -> Result<Value, QueryError> {
    let mut output = Map::new();
    output.insert(String::from("_id"), group.key);
    for (state, accumulator) in group.accumulators.into_iter().zip(&spec.accumulators) {
        let value = match state {
            AccumulatorState::Count(count) => Value::Number(count.into()),
            AccumulatorState::Sum(total) => {
                Value::Number(number_from_i128(total, &accumulator.name)?)
            }
            AccumulatorState::Min(value) | AccumulatorState::Max(value) => {
                value.unwrap_or(Value::Null)
            }
        };
        output.insert(accumulator.name.clone(), value);
    }
    Ok(Value::Object(output))
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
    let ordering = super::ordering::compare_values(current_value, &value).ok_or_else(|| {
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

fn compare_output_values(left: &Value, right: &Value, sort: &IndexMap<String, i8>) -> Ordering {
    let left = left.as_object();
    let right = right.as_object();
    for (field, direction) in sort {
        let ordering = super::ordering::compare_for_sort(
            left.and_then(|values| values.get(field)),
            right.and_then(|values| values.get(field)),
        );
        if ordering != Ordering::Equal {
            return if *direction == 1 {
                ordering
            } else {
                ordering.reverse()
            };
        }
    }
    Ordering::Equal
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
