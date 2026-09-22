use super::{AccumulatorKind, AccumulatorSpec, GroupSpec, QueryError, field_reference};
use crate::query::expression::parse_numeric_operand;
use serde_json::Value;

pub(super) fn parse_group(definition: &Value) -> Result<GroupSpec, QueryError> {
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
            "$avg" => AccumulatorKind::Average(parse_numeric_operand(
                operand,
                &format!("$group.{name}.$avg"),
            )?),
            "$sum" => AccumulatorKind::Sum(parse_numeric_operand(
                operand,
                &format!("$group.{name}.$sum"),
            )?),
            "$min" | "$max" | "$first" | "$last" | "$push" | "$addToSet" => {
                let field = field_reference(
                    operand.as_str().ok_or_else(|| {
                        QueryError::Invalid(format!(
                            "$group.{name}.{operator} must be a field reference"
                        ))
                    })?,
                    &format!("$group.{name}.{operator}"),
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
