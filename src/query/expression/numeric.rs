use super::super::QueryError;
use crate::query_path::field_value;
use serde_json::{Map, Value};

#[derive(Debug, Clone)]
pub enum NumericExpression {
    Field(String),
    Literal(serde_json::Number),
    Absolute(Box<NumericExpression>),
    Binary {
        operator: NumericOperator,
        left: Box<NumericExpression>,
        right: Box<NumericExpression>,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum NumericOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
}

impl NumericOperator {
    fn as_str(self) -> &'static str {
        match self {
            Self::Add => "$add",
            Self::Subtract => "$subtract",
            Self::Multiply => "$multiply",
            Self::Divide => "$divide",
            Self::Modulo => "$mod",
        }
    }
}

pub fn validate_operand(operand: &Value, path: &str) -> Result<(), QueryError> {
    parse_numeric_operand(operand, path).map(|_| ())
}

pub fn parse_numeric_operand(operand: &Value, path: &str) -> Result<NumericExpression, QueryError> {
    if let Some(reference) = operand.as_str().and_then(|value| value.strip_prefix('$')) {
        if reference.is_empty() {
            return Err(QueryError::Invalid(format!(
                "{path} has an empty field reference"
            )));
        }
        return Ok(NumericExpression::Field(reference.to_owned()));
    }
    if let Some(number) = operand.as_number() {
        return Ok(NumericExpression::Literal(number.clone()));
    }
    let Some(expression) = operand.as_object() else {
        return Err(QueryError::Invalid(format!(
            "{path} must be a numeric literal, field reference, or numeric expression"
        )));
    };
    let Some((operator, operands)) = expression.iter().next() else {
        return Err(QueryError::Invalid(format!("{path} cannot be empty")));
    };
    if expression.len() != 1 {
        return Err(QueryError::Invalid(format!(
            "{path} supports only $abs, $add, $subtract, $multiply, $divide, and $mod"
        )));
    }
    if operator == "$abs" {
        return Ok(NumericExpression::Absolute(Box::new(
            parse_numeric_operand(operands, &format!("{path}.$abs"))?,
        )));
    }
    let operator = match operator.as_str() {
        "$add" => NumericOperator::Add,
        "$subtract" => NumericOperator::Subtract,
        "$multiply" => NumericOperator::Multiply,
        "$divide" => NumericOperator::Divide,
        "$mod" => NumericOperator::Modulo,
        _ => {
            return Err(QueryError::Invalid(format!(
                "{path} supports only $abs, $add, $subtract, $multiply, $divide, and $mod"
            )));
        }
    };
    let operands = operands.as_array().ok_or_else(|| {
        QueryError::Invalid(format!("{path}.{} must be an array", operator.as_str()))
    })?;
    let [left, right] = operands.as_slice() else {
        return Err(QueryError::Invalid(format!(
            "{path}.{} requires two operands",
            operator.as_str()
        )));
    };
    Ok(NumericExpression::Binary {
        operator,
        left: Box::new(parse_numeric_operand(
            left,
            &format!("{path}.{}[0]", operator.as_str()),
        )?),
        right: Box::new(parse_numeric_operand(
            right,
            &format!("{path}.{}[1]", operator.as_str()),
        )?),
    })
}

pub fn resolve_operand(
    values: &Map<String, Value>,
    operand: &Value,
) -> Result<Option<Value>, QueryError> {
    let Some(reference) = operand.as_str().and_then(|value| value.strip_prefix('$')) else {
        if operand.is_object() {
            let expression = parse_numeric_operand(operand, "filter.$expr")?;
            return evaluate_numeric(values, &expression, "filter.$expr");
        }
        return Ok(Some(operand.clone()));
    };
    Ok((!reference.is_empty())
        .then(|| field_value(values, reference))
        .flatten())
}

pub fn evaluate_numeric(
    values: &Map<String, Value>,
    expression: &NumericExpression,
    path: &str,
) -> Result<Option<Value>, QueryError> {
    match expression {
        NumericExpression::Field(field) => Ok(field_value(values, field)),
        NumericExpression::Literal(number) => Ok(Some(Value::Number(number.clone()))),
        NumericExpression::Absolute(operand) => {
            let Some(value) = evaluate_numeric(values, operand, &format!("{path}.$abs"))? else {
                return Ok(None);
            };
            apply_absolute_expression(&value, &format!("{path}.$abs"))
        }
        NumericExpression::Binary {
            operator,
            left,
            right,
        } => {
            let operator_name = operator.as_str();
            let expression_path = format!("{path}.{operator_name}");
            let (Some(left), Some(right)) = (
                evaluate_numeric(values, left, &format!("{expression_path}[0]"))?,
                evaluate_numeric(values, right, &format!("{expression_path}[1]"))?,
            ) else {
                return Ok(None);
            };
            apply_numeric_expression(*operator, &left, &right, &expression_path)
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum NumericValue {
    Integer(i128),
    Float(f64),
}

fn apply_numeric_expression(
    operator: NumericOperator,
    left: &Value,
    right: &Value,
    path: &str,
) -> Result<Option<Value>, QueryError> {
    let (Some(left), Some(right)) = (as_numeric(left), as_numeric(right)) else {
        return Ok(None);
    };
    if matches!(operator, NumericOperator::Divide | NumericOperator::Modulo) && is_zero(right) {
        let message = if matches!(operator, NumericOperator::Divide) {
            format!("{path} cannot divide by zero")
        } else {
            format!("{path} cannot use zero as the divisor")
        };
        return Err(QueryError::Invalid(message));
    }
    match (left, right) {
        (NumericValue::Integer(left), NumericValue::Integer(right)) => {
            if matches!(operator, NumericOperator::Divide) && left % right != 0 {
                return finite_json_number(
                    path,
                    numeric_as_f64(NumericValue::Integer(left))
                        / numeric_as_f64(NumericValue::Integer(right)),
                );
            }
            let value = match operator {
                NumericOperator::Add => left.checked_add(right),
                NumericOperator::Subtract => left.checked_sub(right),
                NumericOperator::Multiply => left.checked_mul(right),
                NumericOperator::Divide => left.checked_div(right),
                NumericOperator::Modulo => left.checked_rem(right),
            }
            .ok_or_else(|| QueryError::Invalid(format!("{path} integer result overflows")))?;
            Ok(Some(Value::Number(number_from_i128(value, path)?)))
        }
        (left, right) => {
            let left = numeric_as_f64(left);
            let right = numeric_as_f64(right);
            let value = match operator {
                NumericOperator::Add => left + right,
                NumericOperator::Subtract => left - right,
                NumericOperator::Multiply => left * right,
                NumericOperator::Divide => left / right,
                NumericOperator::Modulo => left % right,
            };
            finite_json_number(path, value)
        }
    }
}

fn apply_absolute_expression(value: &Value, path: &str) -> Result<Option<Value>, QueryError> {
    let Some(value) = as_numeric(value) else {
        return Ok(None);
    };
    match value {
        NumericValue::Integer(value) => {
            let value = value
                .checked_abs()
                .ok_or_else(|| QueryError::Invalid(format!("{path} integer result overflows")))?;
            Ok(Some(Value::Number(number_from_i128(value, path)?)))
        }
        NumericValue::Float(value) => finite_json_number(path, value.abs()),
    }
}

fn finite_json_number(path: &str, value: f64) -> Result<Option<Value>, QueryError> {
    if !value.is_finite() {
        return Err(QueryError::Invalid(format!(
            "{path} result is not a finite JSON number"
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

fn number_from_i128(value: i128, path: &str) -> Result<serde_json::Number, QueryError> {
    if value >= 0 {
        u64::try_from(value)
            .map(serde_json::Number::from)
            .map_err(|_| QueryError::Invalid(format!("{path} numeric result does not fit JSON")))
    } else {
        i64::try_from(value)
            .map(serde_json::Number::from)
            .map_err(|_| QueryError::Invalid(format!("{path} numeric result does not fit JSON")))
    }
}
