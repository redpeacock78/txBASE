use super::SetExpression;
use crate::query::{QueryError, expression};
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) fn parse_set(
    value: &Value,
    index: usize,
    operator: &str,
) -> Result<BTreeMap<String, SetExpression>, QueryError> {
    let object = value.as_object().ok_or_else(|| {
        QueryError::Invalid(format!(
            "aggregate stage {index}.{operator} must be an object"
        ))
    })?;
    if object.is_empty() {
        return Err(QueryError::Invalid(format!(
            "aggregate stage {index}.{operator} cannot be empty"
        )));
    }
    let mut expressions = BTreeMap::new();
    for (field, expression) in object {
        if field.is_empty() || field.starts_with('$') || field.contains('.') {
            return Err(QueryError::Invalid(format!(
                "aggregate stage {index}.{operator} output fields must be top-level names"
            )));
        }
        expressions.insert(
            field.clone(),
            parse_set_expression(
                expression,
                &format!("aggregate stage {index}.{operator}.{field}"),
            )?,
        );
    }
    Ok(expressions)
}

fn parse_set_expression(value: &Value, path: &str) -> Result<SetExpression, QueryError> {
    if let Some(reference) = value.as_str().and_then(|value| value.strip_prefix('$')) {
        if reference.is_empty() {
            return Err(QueryError::Invalid(format!(
                "{path} has an empty field reference"
            )));
        }
        return Ok(SetExpression::Field(reference.to_owned()));
    }
    if !value.is_object() {
        if value.is_array() {
            return Err(QueryError::Invalid(format!(
                "{path} must be a literal, field reference, or supported expression"
            )));
        }
        return Ok(SetExpression::Literal(value.clone()));
    }
    let object = value.as_object().expect("object checked above");
    let Some((operator, operand)) = object.iter().next() else {
        return Err(QueryError::Invalid(format!("{path} cannot be empty")));
    };
    if object.len() != 1 {
        return Err(QueryError::Invalid(format!(
            "{path} supports one expression operator"
        )));
    }
    match operator.as_str() {
        "$literal" => Ok(SetExpression::Literal(operand.clone())),
        "$ifNull" => {
            let operands = operand
                .as_array()
                .ok_or_else(|| QueryError::Invalid(format!("{path}.$ifNull must be an array")))?;
            let [first, fallback] = operands.as_slice() else {
                return Err(QueryError::Invalid(format!(
                    "{path}.$ifNull requires two operands"
                )));
            };
            Ok(SetExpression::IfNull(
                Box::new(parse_set_expression(first, &format!("{path}.$ifNull[0]"))?),
                Box::new(parse_set_expression(
                    fallback,
                    &format!("{path}.$ifNull[1]"),
                )?),
            ))
        }
        "$abs" | "$add" | "$subtract" | "$multiply" | "$divide" | "$mod" => Ok(
            SetExpression::Numeric(expression::parse_numeric_operand(value, path)?),
        ),
        _ => Err(QueryError::Invalid(format!(
            "{path} supports only $literal, $ifNull, $abs, $add, $subtract, $multiply, $divide, and $mod"
        ))),
    }
}
