use super::{QueryError, QueryRequest};
use crate::dbf::DbfRecord;
use crate::query_path::field_value;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub(super) const MAX_GROUPS: usize = 10_000;

#[derive(Debug, Clone)]
struct GroupSpec {
    key_field: Option<String>,
    accumulators: Vec<AccumulatorSpec>,
}

#[derive(Debug, Clone)]
struct AccumulatorSpec {
    name: String,
    kind: AccumulatorKind,
}

#[derive(Debug, Clone)]
enum AccumulatorKind {
    Count,
    Sum(String),
}

#[derive(Debug)]
enum AccumulatorState {
    Count(u64),
    Sum(i128),
}

#[derive(Debug)]
struct GroupState {
    key: Value,
    accumulators: Vec<AccumulatorState>,
}

pub(super) fn validate(request: &QueryRequest) -> Result<(), QueryError> {
    let Some(stages) = request.aggregate.as_ref() else {
        return Ok(());
    };
    if !request.sort.is_empty()
        || !request.projection.is_empty()
        || request.limit.is_some()
        || request.skip.is_some()
        || request.page_size.is_some()
        || request.cursor.is_some()
    {
        return Err(QueryError::Invalid(
            "aggregate cannot be combined with sort, projection, skip, limit, page_size, or cursor"
                .into(),
        ));
    }
    parse(stages).map(|_| ())
}

pub(super) fn execute(
    records: &[&DbfRecord],
    stages: &[Map<String, Value>],
) -> Result<Vec<Value>, QueryError> {
    let spec = parse(stages)?;
    let mut groups = BTreeMap::<String, GroupState>::new();
    if spec.key_field.is_none() {
        groups.insert(String::from("null"), new_group(Value::Null, &spec));
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
            groups.insert(encoded_key.clone(), new_group(key, &spec));
        }
        let group = groups
            .get_mut(&encoded_key)
            .expect("group was inserted or already present");
        for (state, accumulator) in group.accumulators.iter_mut().zip(&spec.accumulators) {
            match (state, &accumulator.kind) {
                (AccumulatorState::Count(count), AccumulatorKind::Count) => {
                    *count = count.checked_add(1).ok_or_else(|| {
                        QueryError::Invalid("aggregate count overflows u64".into())
                    })?;
                }
                (AccumulatorState::Sum(total), AccumulatorKind::Sum(field)) => {
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
                _ => unreachable!("validated accumulator state and specification differ"),
            }
        }
    }

    groups
        .into_values()
        .map(|group| finish_group(group, &spec))
        .collect()
}

fn parse(stages: &[Map<String, Value>]) -> Result<GroupSpec, QueryError> {
    let [stage] = stages else {
        return Err(QueryError::Invalid(
            "aggregate currently requires exactly one $group stage".into(),
        ));
    };
    if stage.len() != 1 || !stage.contains_key("$group") {
        return Err(QueryError::Invalid(
            "aggregate stage must contain only $group".into(),
        ));
    }
    let Some(definition) = stage.get("$group") else {
        return Err(QueryError::Invalid(
            "aggregate stage must contain $group".into(),
        ));
    };
    let definition = definition
        .as_object()
        .ok_or_else(|| QueryError::Invalid("$group must be an object".into()))?;
    let key = definition
        .get("_id")
        .ok_or_else(|| QueryError::Invalid("$group requires _id".into()))?;
    let key_field = match key {
        Value::Null => None,
        Value::String(value) => Some(field_reference(value, "$group._id")?),
        _ => {
            return Err(QueryError::Invalid(
                "$group._id must be null or a field reference".into(),
            ));
        }
    };

    let mut accumulators = Vec::new();
    for (name, value) in definition {
        if name == "_id" {
            continue;
        }
        if name.is_empty() || name.starts_with('$') || name.contains('.') {
            return Err(QueryError::Invalid(format!(
                "$group output field {name} is invalid"
            )));
        }
        let operators = value
            .as_object()
            .ok_or_else(|| QueryError::Invalid(format!("$group.{name} must be an object")))?;
        if operators.len() != 1 {
            return Err(QueryError::Invalid(format!(
                "$group.{name} must contain one accumulator"
            )));
        }
        let (operator, operand) = operators.iter().next().expect("one accumulator");
        let kind = match operator.as_str() {
            "$count" if operand.as_object().is_some_and(|object| object.is_empty()) => {
                AccumulatorKind::Count
            }
            "$sum" => AccumulatorKind::Sum(field_reference(
                operand.as_str().ok_or_else(|| {
                    QueryError::Invalid(format!("$group.{name}.$sum must be a field reference"))
                })?,
                &format!("$group.{name}.$sum"),
            )?),
            _ => {
                return Err(QueryError::Invalid(format!(
                    "unsupported aggregate accumulator {operator}"
                )));
            }
        };
        accumulators.push(AccumulatorSpec {
            name: name.clone(),
            kind,
        });
    }
    Ok(GroupSpec {
        key_field,
        accumulators,
    })
}

fn field_reference(value: &str, path: &str) -> Result<String, QueryError> {
    let Some(field) = value.strip_prefix('$') else {
        return Err(QueryError::Invalid(format!(
            "{path} must be a field reference"
        )));
    };
    if field.is_empty() {
        return Err(QueryError::Invalid(format!(
            "{path} has an empty field reference"
        )));
    }
    Ok(field.to_owned())
}

fn new_group(key: Value, spec: &GroupSpec) -> GroupState {
    GroupState {
        key,
        accumulators: spec
            .accumulators
            .iter()
            .map(|accumulator| match &accumulator.kind {
                AccumulatorKind::Count => AccumulatorState::Count(0),
                AccumulatorKind::Sum(_) => AccumulatorState::Sum(0),
            })
            .collect(),
    }
}

fn finish_group(group: GroupState, spec: &GroupSpec) -> Result<Value, QueryError> {
    let mut output = Map::new();
    output.insert(String::from("_id"), group.key);
    for (state, accumulator) in group.accumulators.into_iter().zip(&spec.accumulators) {
        let value = match state {
            AccumulatorState::Count(count) => Value::Number(count.into()),
            AccumulatorState::Sum(total) => {
                Value::Number(number_from_i128(total, &accumulator.name)?)
            }
        };
        output.insert(accumulator.name.clone(), value);
    }
    Ok(Value::Object(output))
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
