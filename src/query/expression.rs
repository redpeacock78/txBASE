use super::{QueryError, ordering::compare_values};
use serde_json::{Map, Value};

mod array;
mod boolean;
mod numeric;
mod provenance;
mod scope;

use array::ArrayExpression;
use boolean::{BooleanExpression, evaluate_boolean, parse_boolean_expression};
use numeric::evaluate_numeric_in_context;
pub(super) use numeric::{NumericExpression, evaluate_numeric, parse_numeric_operand};
pub(super) use provenance::uses_only_group_key_fields;
use scope::{EvaluationContext, VariableReference, parse_reference, validate_variable_name};

const MAX_STRING_EXPRESSION_BYTES: usize = crate::MAX_JSON_INPUT_BYTES;

#[derive(Debug, Clone)]
pub enum ScalarExpression {
    Reference(ExpressionReference),
    Literal(Value),
    Array(Vec<ScalarExpression>),
    Numeric(NumericExpression),
    Cond {
        condition: Box<BooleanExpression>,
        then_expression: Box<ScalarExpression>,
        else_expression: Box<ScalarExpression>,
    },
    IfNull(Box<ScalarExpression>, Box<ScalarExpression>),
    Concat(Vec<ScalarExpression>),
    ToLower(Box<ScalarExpression>),
    ToUpper(Box<ScalarExpression>),
    ArrayOperation(Box<ArrayExpression>),
    Let {
        bindings: Vec<(String, ScalarExpression)>,
        in_expression: Box<ScalarExpression>,
    },
}

#[derive(Debug, Clone)]
pub(super) enum ExpressionReference {
    Field(String),
    Variable(VariableReference),
}

pub(super) fn validate(expression: &Value, path: &str) -> Result<(), QueryError> {
    parse_boolean_expression(expression, path).map(|_| ())
}

pub(super) fn parse_scalar_operand(
    operand: &Value,
    path: &str,
) -> Result<ScalarExpression, QueryError> {
    if let Some(reference) = operand.as_str().filter(|value| value.starts_with('$')) {
        return Ok(ScalarExpression::Reference(parse_reference(
            reference, path,
        )?));
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
            Ok(ScalarExpression::Cond {
                condition: Box::new(parse_boolean_expression(
                    condition,
                    &format!("{path}.$cond.if"),
                )?),
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
        "$abs" | "$ceil" | "$floor" | "$add" | "$subtract" | "$multiply" | "$divide" | "$mod" => {
            Ok(ScalarExpression::Numeric(parse_numeric_operand(
                operand, path,
            )?))
        }
        "$map" | "$filter" | "$reduce" => Ok(ScalarExpression::ArrayOperation(Box::new(
            array::parse(operator, value, path)?,
        ))),
        "$let" => parse_let(value, path),
        _ => Err(QueryError::Invalid(format!(
            "{path} supports only $literal, $ifNull, $cond, $concat, $toLower, $toUpper, bounded numeric operators, $map, $filter, $reduce, and $let"
        ))),
    }
}

fn parse_let(value: &Value, path: &str) -> Result<ScalarExpression, QueryError> {
    let object = value
        .as_object()
        .ok_or_else(|| QueryError::Invalid(format!("{path}.$let must be an object")))?;
    for option in object.keys() {
        if !matches!(option.as_str(), "vars" | "in") {
            return Err(QueryError::Invalid(format!(
                "{path}.$let has an unsupported option {option}"
            )));
        }
    }
    let variables = object
        .get("vars")
        .and_then(Value::as_object)
        .ok_or_else(|| QueryError::Invalid(format!("{path}.$let.vars must be an object")))?;
    let in_value = object
        .get("in")
        .ok_or_else(|| QueryError::Invalid(format!("{path}.$let is missing in")))?;
    let mut bindings = Vec::with_capacity(variables.len());
    for (name, value) in variables {
        validate_variable_name(name, &format!("{path}.$let.vars"))?;
        bindings.push((
            name.clone(),
            parse_scalar_operand(value, &format!("{path}.$let.vars.{name}"))?,
        ));
    }
    Ok(ScalarExpression::Let {
        bindings,
        in_expression: Box::new(parse_scalar_operand(in_value, &format!("{path}.$let.in"))?),
    })
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
    let mut context = EvaluationContext::new(values);
    evaluate_scalar_in_context(&mut context, expression, path)
}

fn evaluate_scalar_in_context(
    context: &mut EvaluationContext<'_>,
    expression: &ScalarExpression,
    path: &str,
) -> Result<Option<Value>, QueryError> {
    match expression {
        ScalarExpression::Reference(reference) => context.resolve(reference, path),
        ScalarExpression::Literal(value) => Ok(Some(value.clone())),
        ScalarExpression::Array(expressions) => {
            let mut output = Vec::with_capacity(expressions.len());
            let mut output_bytes = 2;
            for (index, expression) in expressions.iter().enumerate() {
                let value =
                    evaluate_scalar_in_context(context, expression, &format!("{path}[{index}]"))?
                        .unwrap_or(Value::Null);
                array::push_array_value(
                    &mut output,
                    &mut output_bytes,
                    value,
                    &format!("{path} array constructor"),
                )?;
            }
            Ok(Some(Value::Array(output)))
        }
        ScalarExpression::Numeric(expression) => {
            evaluate_numeric_in_context(context, expression, path)
        }
        ScalarExpression::Cond {
            condition,
            then_expression,
            else_expression,
        } => {
            if evaluate_boolean(context, condition, &format!("{path}.$cond.if"))? {
                evaluate_scalar_in_context(context, then_expression, &format!("{path}.$cond.then"))
            } else {
                evaluate_scalar_in_context(context, else_expression, &format!("{path}.$cond.else"))
            }
        }
        ScalarExpression::IfNull(first, fallback) => {
            let value = evaluate_scalar_in_context(context, first, &format!("{path}.$ifNull[0]"))?;
            if value.as_ref().is_none_or(Value::is_null) {
                evaluate_scalar_in_context(context, fallback, &format!("{path}.$ifNull[1]"))
            } else {
                Ok(value)
            }
        }
        ScalarExpression::Concat(expressions) => {
            let mut result = String::new();
            for (index, expression) in expressions.iter().enumerate() {
                let Some(value) = evaluate_scalar_in_context(
                    context,
                    expression,
                    &format!("{path}.$concat[{index}]"),
                )?
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
            evaluate_case_expression(context, expression, path, false)
        }
        ScalarExpression::ToUpper(expression) => {
            evaluate_case_expression(context, expression, path, true)
        }
        ScalarExpression::ArrayOperation(expression) => array::evaluate(context, expression, path),
        ScalarExpression::Let {
            bindings,
            in_expression,
        } => {
            let mut variables = Vec::with_capacity(bindings.len());
            for (name, expression) in bindings {
                let value = evaluate_scalar_in_context(
                    context,
                    expression,
                    &format!("{path}.$let.vars.{name}"),
                )?;
                variables.push((name.clone(), value));
            }
            context.with_variables(variables, |context| {
                evaluate_scalar_in_context(context, in_expression, &format!("{path}.$let.in"))
            })
        }
    }
}

fn evaluate_case_expression(
    context: &mut EvaluationContext<'_>,
    expression: &ScalarExpression,
    path: &str,
    uppercase: bool,
) -> Result<Option<Value>, QueryError> {
    let Some(value) = evaluate_scalar_in_context(context, expression, path)? else {
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
    let expression = parse_boolean_expression(expression, "filter.$expr")?;
    let mut context = EvaluationContext::new(values);
    evaluate_boolean(&mut context, &expression, "filter.$expr")
}

#[cfg(test)]
mod tests;
