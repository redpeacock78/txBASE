use super::{QueryError, ordering::compare_values};
use serde_json::{Map, Value};

mod numeric;

pub(super) fn validate(expression: &Value, path: &str) -> Result<(), QueryError> {
    let expression = expression
        .as_object()
        .ok_or_else(|| QueryError::Invalid(format!("{path} must be an object")))?;
    let Some((operator, operands)) = expression.iter().next() else {
        return Err(QueryError::Invalid(format!("{path} cannot be empty")));
    };
    if expression.len() != 1 {
        return Err(QueryError::Invalid(format!(
            "{path} supports one expression operator"
        )));
    }
    match operator.as_str() {
        "$and" | "$or" => {
            let expressions = operands.as_array().ok_or_else(|| {
                QueryError::Invalid(format!("{path}.{operator} must be an array"))
            })?;
            for (index, expression) in expressions.iter().enumerate() {
                validate(expression, &format!("{path}.{operator}[{index}]"))?;
            }
        }
        "$not" => validate(operands, &format!("{path}.$not"))?,
        "$eq" | "$ne" | "$gt" | "$gte" | "$lt" | "$lte" => {
            validate_comparison_operands(operands, operator, path)?;
        }
        _ => {
            return Err(QueryError::Invalid(format!(
                "{path} supports only boolean and comparison operators"
            )));
        }
    }
    Ok(())
}

fn validate_comparison_operands(
    value: &Value,
    operator: &str,
    path: &str,
) -> Result<(), QueryError> {
    let operands = value
        .as_array()
        .ok_or_else(|| QueryError::Invalid(format!("{path}.{operator} must be an array")))?;
    if operands.len() != 2 {
        return Err(QueryError::Invalid(format!(
            "{path}.{operator} requires two operands"
        )));
    }
    for (index, operand) in operands.iter().enumerate() {
        validate_expression_operand(operand, &format!("{path}.{operator}[{index}]"))?;
    }
    Ok(())
}

fn validate_expression_operand(operand: &Value, path: &str) -> Result<(), QueryError> {
    if let Some(reference) = operand.as_str().and_then(|value| value.strip_prefix('$')) {
        if reference.is_empty() {
            return Err(QueryError::Invalid(format!(
                "{path} has an empty field reference"
            )));
        }
        return Ok(());
    }
    if operand.is_number() || operand.is_boolean() || operand.is_string() || operand.is_null() {
        return Ok(());
    }
    numeric::validate_operand(operand, path)
}

pub(super) fn matches(values: &Map<String, Value>, expression: &Value) -> Result<bool, QueryError> {
    let expression = expression
        .as_object()
        .ok_or_else(|| QueryError::Invalid("filter.$expr must be an object".into()))?;
    let Some((operator, operands)) = expression.iter().next() else {
        return Err(QueryError::Invalid("filter.$expr cannot be empty".into()));
    };
    if expression.len() != 1 {
        return Err(QueryError::Invalid(
            "filter.$expr supports one expression operator".into(),
        ));
    }
    match operator.as_str() {
        "$and" => {
            let expressions = operands
                .as_array()
                .ok_or_else(|| QueryError::Invalid("filter.$expr.$and must be an array".into()))?;
            for expression in expressions {
                if !matches(values, expression)? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        "$or" => {
            let expressions = operands
                .as_array()
                .ok_or_else(|| QueryError::Invalid("filter.$expr.$or must be an array".into()))?;
            for expression in expressions {
                if matches(values, expression)? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        "$not" => return Ok(!matches(values, operands)?),
        "$eq" | "$ne" | "$gt" | "$gte" | "$lt" | "$lte" => {}
        _ => {
            return Err(QueryError::Invalid(format!(
                "unsupported expression operator {operator}"
            )));
        }
    }
    let operands = operands
        .as_array()
        .ok_or_else(|| QueryError::Invalid(format!("filter.$expr.{operator} must be an array")))?;
    let [left, right] = operands.as_slice() else {
        return Err(QueryError::Invalid(format!(
            "filter.$expr.{operator} requires two operands"
        )));
    };
    let (Some(left), Some(right)) = (
        numeric::resolve_operand(values, left)?,
        numeric::resolve_operand(values, right)?,
    ) else {
        return Ok(false);
    };
    Ok(match operator.as_str() {
        "$eq" => left == right,
        "$ne" => left != right,
        "$gt" => compare_values(&left, &right).is_some_and(|ordering| ordering.is_gt()),
        "$gte" => compare_values(&left, &right).is_some_and(|ordering| ordering.is_ge()),
        "$lt" => compare_values(&left, &right).is_some_and(|ordering| ordering.is_lt()),
        "$lte" => compare_values(&left, &right).is_some_and(|ordering| ordering.is_le()),
        _ => unreachable!("validated expression operator"),
    })
}
