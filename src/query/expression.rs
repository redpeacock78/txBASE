use super::{QueryError, ordering::compare_values};
use serde_json::{Map, Value};

mod numeric;
pub(super) use numeric::{NumericExpression, evaluate_numeric, parse_numeric_operand};

const MAX_STRING_EXPRESSION_BYTES: usize = crate::MAX_JSON_INPUT_BYTES;

#[derive(Debug, Clone)]
pub enum ScalarExpression {
    Field(String),
    Literal(Value),
    Numeric(NumericExpression),
    IfNull(Box<ScalarExpression>, Box<ScalarExpression>),
    Concat(Vec<ScalarExpression>),
    ToLower(Box<ScalarExpression>),
    ToUpper(Box<ScalarExpression>),
}

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
        parse_scalar_operand(operand, &format!("{path}.{operator}[{index}]"))?;
    }
    Ok(())
}

pub(super) fn parse_scalar_operand(
    operand: &Value,
    path: &str,
) -> Result<ScalarExpression, QueryError> {
    if let Some(reference) = operand.as_str().and_then(|value| value.strip_prefix('$')) {
        if reference.is_empty() {
            return Err(QueryError::Invalid(format!(
                "{path} has an empty field reference"
            )));
        }
        return Ok(ScalarExpression::Field(reference.to_owned()));
    }
    if !operand.is_object() {
        if operand.is_array() {
            return Err(QueryError::Invalid(format!(
                "{path} must be a scalar literal, field reference, or supported expression"
            )));
        }
        return Ok(ScalarExpression::Literal(operand.clone()));
    }
    let object = operand.as_object().expect("object checked above");
    let Some((operator, value)) = object.iter().next() else {
        return Err(QueryError::Invalid(format!("{path} cannot be empty")));
    };
    if object.len() != 1 {
        return Err(QueryError::Invalid(format!(
            "{path} supports one expression operator"
        )));
    }
    match operator.as_str() {
        "$literal" => Ok(ScalarExpression::Literal(value.clone())),
        "$ifNull" => {
            let operands = value
                .as_array()
                .ok_or_else(|| QueryError::Invalid(format!("{path}.$ifNull must be an array")))?;
            let [first, fallback] = operands.as_slice() else {
                return Err(QueryError::Invalid(format!(
                    "{path}.$ifNull requires two operands"
                )));
            };
            Ok(ScalarExpression::IfNull(
                Box::new(parse_scalar_operand(first, &format!("{path}.$ifNull[0]"))?),
                Box::new(parse_scalar_operand(
                    fallback,
                    &format!("{path}.$ifNull[1]"),
                )?),
            ))
        }
        "$concat" => {
            let operands = value
                .as_array()
                .ok_or_else(|| QueryError::Invalid(format!("{path}.$concat must be an array")))?;
            if operands.len() < 2 {
                return Err(QueryError::Invalid(format!(
                    "{path}.$concat requires at least two operands"
                )));
            }
            operands
                .iter()
                .enumerate()
                .map(|(index, operand)| {
                    parse_scalar_operand(operand, &format!("{path}.$concat[{index}]"))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(ScalarExpression::Concat)
        }
        "$toLower" | "$toUpper" => {
            let operand = parse_unary_scalar_operand(value, operator, path)?;
            if operator == "$toLower" {
                Ok(ScalarExpression::ToLower(Box::new(operand)))
            } else {
                Ok(ScalarExpression::ToUpper(Box::new(operand)))
            }
        }
        "$abs" | "$add" | "$subtract" | "$multiply" | "$divide" | "$mod" => Ok(
            ScalarExpression::Numeric(parse_numeric_operand(operand, path)?),
        ),
        _ => Err(QueryError::Invalid(format!(
            "{path} supports only $literal, $ifNull, $concat, $toLower, $toUpper, $abs, $add, $subtract, $multiply, $divide, and $mod"
        ))),
    }
}

fn parse_unary_scalar_operand(
    value: &Value,
    operator: &str,
    path: &str,
) -> Result<ScalarExpression, QueryError> {
    let operand = parse_scalar_operand(value, &format!("{path}.{operator}"))?;
    if matches!(&operand, ScalarExpression::Literal(value) if !value.is_string()) {
        return Err(QueryError::Invalid(format!(
            "{path}.{operator} requires a string literal, field reference, or scalar expression"
        )));
    }
    Ok(operand)
}

pub(super) fn evaluate_scalar(
    values: &Map<String, Value>,
    expression: &ScalarExpression,
    path: &str,
) -> Result<Option<Value>, QueryError> {
    match expression {
        ScalarExpression::Field(field) => Ok(crate::query_path::field_value(values, field)),
        ScalarExpression::Literal(value) => Ok(Some(value.clone())),
        ScalarExpression::Numeric(expression) => evaluate_numeric(values, expression, path),
        ScalarExpression::IfNull(first, fallback) => {
            let value = evaluate_scalar(values, first, &format!("{path}.$ifNull[0]"))?;
            if value.as_ref().is_none_or(Value::is_null) {
                evaluate_scalar(values, fallback, &format!("{path}.$ifNull[1]"))
            } else {
                Ok(value)
            }
        }
        ScalarExpression::Concat(expressions) => {
            let mut result = String::new();
            for (index, expression) in expressions.iter().enumerate() {
                let Some(value) =
                    evaluate_scalar(values, expression, &format!("{path}.$concat[{index}]"))?
                else {
                    return Ok(None);
                };
                let Some(value) = value.as_str() else {
                    return Ok(None);
                };
                if value.len() > MAX_STRING_EXPRESSION_BYTES - result.len() {
                    return Err(QueryError::Invalid(format!(
                        "{path}.$concat result exceeds {MAX_STRING_EXPRESSION_BYTES} bytes"
                    )));
                }
                result.push_str(value);
            }
            Ok(Some(Value::String(result)))
        }
        ScalarExpression::ToLower(expression) => {
            evaluate_case_expression(values, expression, path, false)
        }
        ScalarExpression::ToUpper(expression) => {
            evaluate_case_expression(values, expression, path, true)
        }
    }
}

fn evaluate_case_expression(
    values: &Map<String, Value>,
    expression: &ScalarExpression,
    path: &str,
    uppercase: bool,
) -> Result<Option<Value>, QueryError> {
    let Some(value) = evaluate_scalar(values, expression, path)? else {
        return Ok(None);
    };
    let Some(value) = value.as_str() else {
        return Ok(None);
    };
    let mut result = String::new();
    for character in value.chars() {
        if uppercase {
            for mapped in character.to_uppercase() {
                if mapped.len_utf8() > MAX_STRING_EXPRESSION_BYTES - result.len() {
                    return Err(QueryError::Invalid(format!(
                        "{path} result exceeds {MAX_STRING_EXPRESSION_BYTES} bytes"
                    )));
                }
                result.push(mapped);
            }
        } else {
            for mapped in character.to_lowercase() {
                if mapped.len_utf8() > MAX_STRING_EXPRESSION_BYTES - result.len() {
                    return Err(QueryError::Invalid(format!(
                        "{path} result exceeds {MAX_STRING_EXPRESSION_BYTES} bytes"
                    )));
                }
                result.push(mapped);
            }
        }
    }
    Ok(Some(Value::String(result)))
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
        evaluate_scalar(
            values,
            &parse_scalar_operand(left, "filter.$expr")?,
            "filter.$expr",
        )?,
        evaluate_scalar(
            values,
            &parse_scalar_operand(right, "filter.$expr")?,
            "filter.$expr",
        )?,
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
