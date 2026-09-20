use super::{QueryError, ordering::compare_values};
use crate::query_path::field_value;
use serde_json::{Map, Value};

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
    let Some(expression) = operand.as_object() else {
        return Err(QueryError::Invalid(format!(
            "{path} must be a scalar, field reference, or numeric expression"
        )));
    };
    let Some((operator, operands)) = expression.iter().next() else {
        return Err(QueryError::Invalid(format!("{path} cannot be empty")));
    };
    if expression.len() != 1
        || !matches!(
            operator.as_str(),
            "$add" | "$subtract" | "$multiply" | "$divide" | "$mod"
        )
    {
        return Err(QueryError::Invalid(format!(
            "{path} supports only $add, $subtract, $multiply, $divide, and $mod"
        )));
    }
    let operands = operands
        .as_array()
        .ok_or_else(|| QueryError::Invalid(format!("{path}.{operator} must be an array")))?;
    if operands.len() != 2 {
        return Err(QueryError::Invalid(format!(
            "{path}.{operator} requires two operands"
        )));
    }
    for (index, operand) in operands.iter().enumerate() {
        validate_numeric_expression_operand(operand, &format!("{path}.{operator}[{index}]"))?;
    }
    Ok(())
}

fn validate_numeric_expression_operand(operand: &Value, path: &str) -> Result<(), QueryError> {
    if let Some(reference) = operand.as_str().and_then(|value| value.strip_prefix('$')) {
        if reference.is_empty() {
            return Err(QueryError::Invalid(format!(
                "{path} has an empty field reference"
            )));
        }
        return Ok(());
    }
    if operand.is_number() {
        return Ok(());
    }
    if operand.is_object() {
        return validate_expression_operand(operand, path);
    }
    Err(QueryError::Invalid(format!(
        "{path} must be a numeric literal, field reference, or numeric expression"
    )))
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
        resolve_operand(values, left)?,
        resolve_operand(values, right)?,
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

fn resolve_operand(
    values: &Map<String, Value>,
    operand: &Value,
) -> Result<Option<Value>, QueryError> {
    let Some(reference) = operand.as_str().and_then(|value| value.strip_prefix('$')) else {
        return resolve_numeric_expression(values, operand);
    };
    Ok((!reference.is_empty())
        .then(|| field_value(values, reference))
        .flatten())
}

fn resolve_numeric_expression(
    values: &Map<String, Value>,
    operand: &Value,
) -> Result<Option<Value>, QueryError> {
    let Some(expression) = operand.as_object() else {
        return Ok(Some(operand.clone()));
    };
    let Some((operator, operands)) = expression.iter().next() else {
        return Err(QueryError::Invalid(
            "filter.$expr numeric expression cannot be empty".into(),
        ));
    };
    if expression.len() != 1
        || !matches!(
            operator.as_str(),
            "$add" | "$subtract" | "$multiply" | "$divide" | "$mod"
        )
    {
        return Err(QueryError::Invalid(format!(
            "unsupported numeric expression operator {operator}"
        )));
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
        resolve_operand(values, left)?,
        resolve_operand(values, right)?,
    ) else {
        return Ok(None);
    };
    apply_numeric_expression(operator, &left, &right)
}

#[derive(Debug, Clone, Copy)]
enum NumericValue {
    Integer(i128),
    Float(f64),
}

fn apply_numeric_expression(
    operator: &str,
    left: &Value,
    right: &Value,
) -> Result<Option<Value>, QueryError> {
    let (Some(left), Some(right)) = (as_numeric(left), as_numeric(right)) else {
        return Ok(None);
    };
    if matches!(operator, "$divide" | "$mod") && is_zero(right) {
        let message = if operator == "$divide" {
            format!("filter.$expr.{operator} cannot divide by zero")
        } else {
            format!("filter.$expr.{operator} cannot use zero as the divisor")
        };
        return Err(QueryError::Invalid(message));
    }
    match (left, right) {
        (NumericValue::Integer(left), NumericValue::Integer(right)) => {
            if operator == "$divide" && left % right != 0 {
                return finite_json_number(
                    operator,
                    numeric_as_f64(NumericValue::Integer(left))
                        / numeric_as_f64(NumericValue::Integer(right)),
                );
            }
            let value = match operator {
                "$add" => left.checked_add(right),
                "$subtract" => left.checked_sub(right),
                "$multiply" => left.checked_mul(right),
                "$divide" => left.checked_div(right),
                "$mod" => left.checked_rem(right),
                _ => unreachable!("validated numeric expression operator"),
            }
            .ok_or_else(|| {
                QueryError::Invalid(format!("filter.$expr.{operator} integer result overflows"))
            })?;
            Ok(Some(Value::Number(number_from_i128(value)?)))
        }
        (left, right) => {
            let left = numeric_as_f64(left);
            let right = numeric_as_f64(right);
            let value = match operator {
                "$add" => left + right,
                "$subtract" => left - right,
                "$multiply" => left * right,
                "$divide" => left / right,
                "$mod" => left % right,
                _ => unreachable!("validated numeric expression operator"),
            };
            finite_json_number(operator, value)
        }
    }
}

fn finite_json_number(operator: &str, value: f64) -> Result<Option<Value>, QueryError> {
    if !value.is_finite() {
        return Err(QueryError::Invalid(format!(
            "filter.$expr.{operator} result is not a finite JSON number"
        )));
    }
    Ok(serde_json::Number::from_f64(value).map(Value::Number))
}

fn is_zero(value: NumericValue) -> bool {
    match value {
        NumericValue::Integer(value) => value == 0,
        NumericValue::Float(value) => value == 0.0,
    }
}

fn as_numeric(value: &Value) -> Option<NumericValue> {
    let number = value.as_number()?;
    number
        .as_i64()
        .map(i128::from)
        .map(NumericValue::Integer)
        .or_else(|| number.as_u64().map(i128::from).map(NumericValue::Integer))
        .or_else(|| number.as_f64().map(NumericValue::Float))
}

fn numeric_as_f64(value: NumericValue) -> f64 {
    match value {
        NumericValue::Integer(value) => value as f64,
        NumericValue::Float(value) => value,
    }
}

fn number_from_i128(value: i128) -> Result<serde_json::Number, QueryError> {
    if value >= 0 {
        u64::try_from(value)
            .map(serde_json::Number::from)
            .map_err(|_| {
                QueryError::Invalid("filter.$expr numeric result does not fit JSON".into())
            })
    } else {
        i64::try_from(value)
            .map(serde_json::Number::from)
            .map_err(|_| {
                QueryError::Invalid("filter.$expr numeric result does not fit JSON".into())
            })
    }
}
