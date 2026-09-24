use super::{AccumulatorKind, AccumulatorSpec, GroupSpec, QueryError, field_reference};
use crate::query::expression::{parse_numeric_operand, parse_scalar_operand};
use serde_json::{Map, Value};

pub(super) fn parse_group(definition: &Value) -> Result<GroupSpec, QueryError> {
    let definition = definition
        .as_object()
        .ok_or_else(|| QueryError::Invalid("$group must be an object".into()))?;
    let key = definition
        .get("_id")
        .ok_or_else(|| QueryError::Invalid("$group requires _id".into()))?;
    let key_expression = match key {
        Value::Null => None,
        _ => Some(parse_scalar_operand(key, "$group._id")?),
    };

    Ok(GroupSpec {
        key_expression,
        accumulators: parse_accumulators(definition, "$group")?,
    })
}

pub(super) fn parse_accumulators(
    definition: &Map<String, Value>,
    prefix: &str,
) -> Result<Vec<AccumulatorSpec>, QueryError> {
    let mut accumulators = Vec::new();
    for (name, value) in definition {
        if name == "_id" {
            continue;
        }
        if name.is_empty() || name.starts_with('$') || name.contains('.') {
            return Err(QueryError::Invalid(format!(
                "{prefix} output field {name} is invalid"
            )));
        }
        let operators = value
            .as_object()
            .ok_or_else(|| QueryError::Invalid(format!("{prefix}.{name} must be an object")))?;
        if operators.len() != 1 {
            return Err(QueryError::Invalid(format!(
                "{prefix}.{name} must contain one accumulator"
            )));
        }
        let (operator, operand) = operators.iter().next().expect("one accumulator");
        let path = format!("{prefix}.{name}.{operator}");
        let kind = match operator.as_str() {
            "$count" if operand.as_object().is_some_and(|object| object.is_empty()) => {
                AccumulatorKind::Count
            }
            "$avg" => AccumulatorKind::Average(parse_numeric_operand(operand, &path)?),
            "$stdDevPop" => AccumulatorKind::StdDevPop(parse_numeric_operand(operand, &path)?),
            "$stdDevSamp" => AccumulatorKind::StdDevSamp(parse_numeric_operand(operand, &path)?),
            "$sum" => AccumulatorKind::Sum(parse_numeric_operand(operand, &path)?),
            "$min" | "$max" | "$first" | "$last" | "$push" | "$addToSet" => {
                let field = field_reference(
                    operand.as_str().ok_or_else(|| {
                        QueryError::Invalid(format!(
                            "{prefix}.{name}.{operator} must be a field reference"
                        ))
                    })?,
                    &path,
                )?;
                match operator.as_str() {
                    "$min" => AccumulatorKind::Min(field),
                    "$max" => AccumulatorKind::Max(field),
                    "$first" => AccumulatorKind::First(field),
                    "$last" => AccumulatorKind::Last(field),
                    "$push" => AccumulatorKind::Push(field),
                    "$addToSet" => AccumulatorKind::AddToSet(field),
                    _ => unreachable!("matched accumulator operator"),
                }
            }
            _ => {
                return Err(QueryError::Invalid(format!(
                    "unsupported aggregate accumulator {operator} at {prefix}"
                )));
            }
        };
        accumulators.push(AccumulatorSpec {
            name: name.clone(),
            kind,
        });
    }
    Ok(accumulators)
}
