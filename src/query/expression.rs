use super::{QueryError, ordering::compare_values};
use serde_json::{Map, Value};

mod numeric;
pub(super) use numeric::{NumericExpression, evaluate_numeric, parse_numeric_operand};

const MAX_STRING_EXPRESSION_BYTES: usize = crate::MAX_JSON_INPUT_BYTES;

#[derive(Debug, Clone)]
pub enum ScalarExpression {
    Field(String),
    Literal(Value),
    Array(Vec<ScalarExpression>),
    Numeric(NumericExpression),
    Cond {
        condition: Value,
        then_expression: Box<ScalarExpression>,
        else_expression: Box<ScalarExpression>,
    },
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
    if let Some(values) = operand.as_array() {
        return values
            .iter()
            .enumerate()
            .map(|(index, value)| parse_scalar_operand(value, &format!("{path}[{index}]")))
            .collect::<Result<Vec<_>, _>>()
            .map(ScalarExpression::Array);
    }
    if !operand.is_object() {
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
        "$cond" => {
            let (condition, then_value, else_value) = parse_conditional_operands(value, path)?;
            validate(condition, &format!("{path}.$cond.if"))?;
            Ok(ScalarExpression::Cond {
                condition: condition.clone(),
                then_expression: Box::new(parse_scalar_operand(
                    then_value,
                    &format!("{path}.$cond.then"),
                )?),
                else_expression: Box::new(parse_scalar_operand(
                    else_value,
                    &format!("{path}.$cond.else"),
                )?),
            })
        }
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

fn parse_conditional_operands<'a>(
    value: &'a Value,
    path: &str,
) -> Result<(&'a Value, &'a Value, &'a Value), QueryError> {
    if let Some(operands) = value.as_array() {
        let [condition, then_value, else_value] = operands.as_slice() else {
            return Err(QueryError::Invalid(format!(
                "{path}.$cond requires exactly three array operands"
            )));
        };
        return Ok((condition, then_value, else_value));
    }

    if let Some(operands) = value.as_object() {
        if operands.len() != 3 {
            return Err(QueryError::Invalid(format!(
                "{path}.$cond object must contain exactly if, then, and else"
            )));
        }
        let condition = operands
            .get("if")
            .ok_or_else(|| QueryError::Invalid(format!("{path}.$cond object is missing if")))?;
        let then_value = operands
            .get("then")
            .ok_or_else(|| QueryError::Invalid(format!("{path}.$cond object is missing then")))?;
        let else_value = operands
            .get("else")
            .ok_or_else(|| QueryError::Invalid(format!("{path}.$cond object is missing else")))?;
        return Ok((condition, then_value, else_value));
    }

    Err(QueryError::Invalid(format!(
        "{path}.$cond must be an array or object"
    )))
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
        ScalarExpression::Array(expressions) => expressions
            .iter()
            .enumerate()
            .map(|(index, expression)| {
                Ok(
                    evaluate_scalar(values, expression, &format!("{path}[{index}]"))?
                        .unwrap_or(Value::Null),
                )
            })
            .collect::<Result<Vec<_>, QueryError>>()
            .map(Value::Array)
            .map(Some),
        ScalarExpression::Numeric(expression) => evaluate_numeric(values, expression, path),
        ScalarExpression::Cond {
            condition,
            then_expression,
            else_expression,
        } => {
            if matches_at(values, condition, &format!("{path}.$cond.if"))? {
                evaluate_scalar(values, then_expression, &format!("{path}.$cond.then"))
            } else {
                evaluate_scalar(values, else_expression, &format!("{path}.$cond.else"))
            }
        }
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

pub(super) fn uses_only_group_key_fields(expression: &ScalarExpression) -> bool {
    match expression {
        ScalarExpression::Field(field) => is_group_key_field(field),
        ScalarExpression::Literal(_) => true,
        ScalarExpression::Array(expressions) | ScalarExpression::Concat(expressions) => {
            expressions.iter().all(uses_only_group_key_fields)
        }
        ScalarExpression::Numeric(expression) => numeric_uses_only_group_key_fields(expression),
        ScalarExpression::Cond {
            condition,
            then_expression,
            else_expression,
        } => {
            condition_uses_only_group_key_fields(condition)
                && uses_only_group_key_fields(then_expression)
                && uses_only_group_key_fields(else_expression)
        }
        ScalarExpression::IfNull(first, fallback) => {
            uses_only_group_key_fields(first) && uses_only_group_key_fields(fallback)
        }
        ScalarExpression::ToLower(expression) | ScalarExpression::ToUpper(expression) => {
            uses_only_group_key_fields(expression)
        }
    }
}

fn is_group_key_field(field: &str) -> bool {
    field == "_id"
        || field
            .strip_prefix("_id.")
            .is_some_and(|path| !path.is_empty())
}

fn numeric_uses_only_group_key_fields(expression: &NumericExpression) -> bool {
    match expression {
        NumericExpression::Field(field) => is_group_key_field(field),
        NumericExpression::Literal(_) => true,
        NumericExpression::Absolute(expression) => numeric_uses_only_group_key_fields(expression),
        NumericExpression::Binary { left, right, .. } => {
            numeric_uses_only_group_key_fields(left) && numeric_uses_only_group_key_fields(right)
        }
    }
}

fn condition_uses_only_group_key_fields(expression: &Value) -> bool {
    match expression {
        Value::String(value) => value.strip_prefix('$').is_none_or(is_group_key_field),
        Value::Array(values) => values.iter().all(condition_uses_only_group_key_fields),
        Value::Object(object) if object.len() == 1 && object.contains_key("$literal") => true,
        Value::Object(object) => object.values().all(condition_uses_only_group_key_fields),
        _ => true,
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
    matches_at(values, expression, "filter.$expr")
}

fn matches_at(
    values: &Map<String, Value>,
    expression: &Value,
    path: &str,
) -> Result<bool, QueryError> {
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
        "$and" => {
            let expressions = operands
                .as_array()
                .ok_or_else(|| QueryError::Invalid(format!("{path}.$and must be an array")))?;
            for (index, expression) in expressions.iter().enumerate() {
                if !matches_at(values, expression, &format!("{path}.$and[{index}]"))? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        "$or" => {
            let expressions = operands
                .as_array()
                .ok_or_else(|| QueryError::Invalid(format!("{path}.$or must be an array")))?;
            for (index, expression) in expressions.iter().enumerate() {
                if matches_at(values, expression, &format!("{path}.$or[{index}]"))? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        "$not" => return Ok(!matches_at(values, operands, &format!("{path}.$not"))?),
        "$eq" | "$ne" | "$gt" | "$gte" | "$lt" | "$lte" => {}
        _ => {
            return Err(QueryError::Invalid(format!(
                "unsupported expression operator {operator}"
            )));
        }
    }
    let operands = operands
        .as_array()
        .ok_or_else(|| QueryError::Invalid(format!("{path}.{operator} must be an array")))?;
    let [left, right] = operands.as_slice() else {
        return Err(QueryError::Invalid(format!(
            "{path}.{operator} requires two operands"
        )));
    };
    let left_path = format!("{path}.{operator}[0]");
    let right_path = format!("{path}.{operator}[1]");
    let (Some(left), Some(right)) = (
        evaluate_scalar(values, &parse_scalar_operand(left, &left_path)?, &left_path)?,
        evaluate_scalar(
            values,
            &parse_scalar_operand(right, &right_path)?,
            &right_path,
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
