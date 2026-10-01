use super::{
    BooleanExpression, EvaluationContext, QueryError, ScalarExpression, evaluate_boolean,
    evaluate_scalar_in_context, parse_boolean_expression, parse_scalar_operand,
};
use serde_json::{Map, Value};

const MAX_ARRAY_RESULT_BYTES: usize = crate::MAX_JSON_INPUT_BYTES;

#[derive(Debug, Clone)]
pub(in crate::query) enum ArrayExpression {
    Map {
        input: Box<ScalarExpression>,
        variable: String,
        array_index_variable: Option<String>,
        in_expression: Box<ScalarExpression>,
    },
    Filter {
        input: Box<ScalarExpression>,
        variable: String,
        array_index_variable: Option<String>,
        condition: BooleanExpression,
        limit: Option<Box<ScalarExpression>>,
    },
    Reduce {
        input: Box<ScalarExpression>,
        initial_value: Box<ScalarExpression>,
        variable: String,
        value_variable: String,
        array_index_variable: Option<String>,
        in_expression: Box<ScalarExpression>,
    },
}

pub(super) fn parse(
    operator: &str,
    value: &Value,
    path: &str,
) -> Result<ArrayExpression, QueryError> {
    let object = value
        .as_object()
        .ok_or_else(|| QueryError::Invalid(format!("{path}.{operator} must be an object")))?;
    match operator {
        "$map" => parse_map(object, path),
        "$filter" => parse_filter(object, path),
        "$reduce" => parse_reduce(object, path),
        _ => unreachable!("array expression operator was matched by caller"),
    }
}

fn parse_map(object: &Map<String, Value>, path: &str) -> Result<ArrayExpression, QueryError> {
    check_options(object, &["input", "as", "arrayIndexAs", "in"], "$map", path)?;
    let variable = parse_variable_option(object, "as", "this", "$map", path)?;
    let array_index_variable = parse_index_variable(object, "$map", path)?;
    reject_alias_collision(&variable, array_index_variable.as_deref(), "$map", path)?;
    Ok(ArrayExpression::Map {
        input: Box::new(parse_required_expression(object, "input", "$map", path)?),
        variable,
        array_index_variable,
        in_expression: Box::new(parse_required_expression(object, "in", "$map", path)?),
    })
}

fn parse_filter(object: &Map<String, Value>, path: &str) -> Result<ArrayExpression, QueryError> {
    check_options(
        object,
        &["input", "as", "arrayIndexAs", "cond", "limit"],
        "$filter",
        path,
    )?;
    let variable = parse_variable_option(object, "as", "this", "$filter", path)?;
    let array_index_variable = parse_index_variable(object, "$filter", path)?;
    reject_alias_collision(&variable, array_index_variable.as_deref(), "$filter", path)?;
    let condition_value = required(object, "cond", "$filter", path)?;
    Ok(ArrayExpression::Filter {
        input: Box::new(parse_required_expression(object, "input", "$filter", path)?),
        variable,
        array_index_variable,
        condition: parse_boolean_expression(condition_value, &format!("{path}.$filter.cond"))?,
        limit: object
            .get("limit")
            .map(|value| parse_scalar_operand(value, &format!("{path}.$filter.limit")))
            .transpose()?
            .map(Box::new),
    })
}

fn parse_reduce(object: &Map<String, Value>, path: &str) -> Result<ArrayExpression, QueryError> {
    check_options(
        object,
        &[
            "input",
            "initialValue",
            "in",
            "as",
            "valueAs",
            "arrayIndexAs",
        ],
        "$reduce",
        path,
    )?;
    let variable = parse_variable_option(object, "as", "this", "$reduce", path)?;
    let value_variable = parse_variable_option(object, "valueAs", "value", "$reduce", path)?;
    let array_index_variable = parse_index_variable(object, "$reduce", path)?;
    if variable == value_variable
        || array_index_variable
            .as_deref()
            .is_some_and(|index| index == variable || index == value_variable)
    {
        return Err(QueryError::Invalid(format!(
            "{path}.$reduce variable names must be distinct"
        )));
    }
    Ok(ArrayExpression::Reduce {
        input: Box::new(parse_required_expression(object, "input", "$reduce", path)?),
        initial_value: Box::new(parse_required_expression(
            object,
            "initialValue",
            "$reduce",
            path,
        )?),
        variable,
        value_variable,
        array_index_variable,
        in_expression: Box::new(parse_required_expression(object, "in", "$reduce", path)?),
    })
}

fn check_options(
    object: &Map<String, Value>,
    allowed: &[&str],
    operator: &str,
    path: &str,
) -> Result<(), QueryError> {
    if let Some(option) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(QueryError::Invalid(format!(
            "{path}.{operator} has an unsupported option {option}"
        )));
    }
    Ok(())
}

fn required<'a>(
    object: &'a Map<String, Value>,
    name: &str,
    operator: &str,
    path: &str,
) -> Result<&'a Value, QueryError> {
    object
        .get(name)
        .ok_or_else(|| QueryError::Invalid(format!("{path}.{operator} is missing {name}")))
}

fn parse_required_expression(
    object: &Map<String, Value>,
    name: &str,
    operator: &str,
    path: &str,
) -> Result<ScalarExpression, QueryError> {
    parse_scalar_operand(
        required(object, name, operator, path)?,
        &format!("{path}.{operator}.{name}"),
    )
}

fn parse_variable_option(
    object: &Map<String, Value>,
    option: &str,
    default: &str,
    operator: &str,
    path: &str,
) -> Result<String, QueryError> {
    let Some(value) = object.get(option) else {
        return Ok(default.to_owned());
    };
    let name = value.as_str().ok_or_else(|| {
        QueryError::Invalid(format!(
            "{path}.{operator}.{option} must be a variable name"
        ))
    })?;
    super::validate_variable_name(name, &format!("{path}.{operator}.{option}"))?;
    Ok(name.to_owned())
}

fn parse_index_variable(
    object: &Map<String, Value>,
    operator: &str,
    path: &str,
) -> Result<Option<String>, QueryError> {
    object
        .get("arrayIndexAs")
        .map(|value| {
            let name = value.as_str().ok_or_else(|| {
                QueryError::Invalid(format!(
                    "{path}.{operator}.arrayIndexAs must be a variable name"
                ))
            })?;
            super::validate_variable_name(name, &format!("{path}.{operator}.arrayIndexAs"))?;
            Ok(name.to_owned())
        })
        .transpose()
}

fn reject_alias_collision(
    variable: &str,
    index_variable: Option<&str>,
    operator: &str,
    path: &str,
) -> Result<(), QueryError> {
    if index_variable == Some(variable) {
        return Err(QueryError::Invalid(format!(
            "{path}.{operator} variable names must be distinct"
        )));
    }
    Ok(())
}

pub(super) fn evaluate(
    context: &mut EvaluationContext<'_>,
    expression: &ArrayExpression,
    path: &str,
) -> Result<Option<Value>, QueryError> {
    match expression {
        ArrayExpression::Map {
            input,
            variable,
            array_index_variable,
            in_expression,
        } => {
            let Some(items) = evaluate_input(context, input, "$map", path)? else {
                return Ok(Some(Value::Null));
            };
            let mut output = Vec::new();
            let mut output_bytes = 2;
            for (index, item) in items.into_iter().enumerate() {
                context.visit_array_item(&format!("{path}.$map"))?;
                let bindings = iteration_bindings(variable, array_index_variable, index, item);
                let value = context.with_variables(bindings, |context| {
                    evaluate_scalar_in_context(context, in_expression, &format!("{path}.$map.in"))
                })?;
                push_array_value(
                    &mut output,
                    &mut output_bytes,
                    value.unwrap_or(Value::Null),
                    &format!("{path}.$map result"),
                )?;
            }
            Ok(Some(Value::Array(output)))
        }
        ArrayExpression::Filter {
            input,
            variable,
            array_index_variable,
            condition,
            limit,
        } => {
            let Some(items) = evaluate_input(context, input, "$filter", path)? else {
                return Ok(Some(Value::Null));
            };
            let limit = limit
                .as_ref()
                .map(|limit| {
                    evaluate_scalar_in_context(context, limit, &format!("{path}.$filter.limit"))
                        .and_then(|value| parse_limit(value, path))
                })
                .transpose()?
                .flatten();
            let mut output = Vec::new();
            let mut output_bytes = 2;
            for (index, item) in items.into_iter().enumerate() {
                if limit.is_some_and(|limit| output.len() as u64 >= limit) {
                    break;
                }
                context.visit_array_item(&format!("{path}.$filter"))?;
                let bindings =
                    iteration_bindings(variable, array_index_variable, index, item.clone());
                let matches = context.with_variables(bindings, |context| {
                    evaluate_boolean(context, condition, &format!("{path}.$filter.cond"))
                })?;
                if matches {
                    push_array_value(
                        &mut output,
                        &mut output_bytes,
                        item,
                        &format!("{path}.$filter result"),
                    )?;
                }
            }
            Ok(Some(Value::Array(output)))
        }
        ArrayExpression::Reduce {
            input,
            initial_value,
            variable,
            value_variable,
            array_index_variable,
            in_expression,
        } => {
            let Some(items) = evaluate_input(context, input, "$reduce", path)? else {
                return Ok(Some(Value::Null));
            };
            let mut accumulator = evaluate_scalar_in_context(
                context,
                initial_value,
                &format!("{path}.$reduce.initialValue"),
            )?;
            for (index, item) in items.into_iter().enumerate() {
                context.visit_array_item(&format!("{path}.$reduce"))?;
                let mut bindings = vec![
                    (variable.clone(), Some(item)),
                    (value_variable.clone(), accumulator.clone()),
                ];
                bindings.push(index_binding(array_index_variable, index));
                accumulator = context.with_variables(bindings, |context| {
                    evaluate_scalar_in_context(
                        context,
                        in_expression,
                        &format!("{path}.$reduce.in"),
                    )
                })?;
            }
            Ok(accumulator)
        }
    }
}

pub(super) fn push_array_value(
    output: &mut Vec<Value>,
    output_bytes: &mut usize,
    value: Value,
    path: &str,
) -> Result<(), QueryError> {
    let encoded = serde_json::to_vec(&value)
        .map_err(|error| QueryError::Invalid(format!("{path} cannot be encoded: {error}")))?;
    let additional_bytes = encoded.len() + usize::from(!output.is_empty());
    if additional_bytes > MAX_ARRAY_RESULT_BYTES.saturating_sub(*output_bytes) {
        return Err(QueryError::Invalid(format!(
            "{path} exceeds {MAX_ARRAY_RESULT_BYTES} encoded bytes"
        )));
    }
    *output_bytes += additional_bytes;
    output.push(value);
    Ok(())
}

fn evaluate_input(
    context: &mut EvaluationContext<'_>,
    expression: &ScalarExpression,
    operator: &str,
    path: &str,
) -> Result<Option<Vec<Value>>, QueryError> {
    let input =
        evaluate_scalar_in_context(context, expression, &format!("{path}.{operator}.input"))?;
    match input {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => Ok(Some(items)),
        Some(_) => Err(QueryError::Invalid(format!(
            "{path}.{operator}.input must resolve to an array or null"
        ))),
    }
}

fn parse_limit(value: Option<Value>, path: &str) -> Result<Option<u64>, QueryError> {
    let Some(value) = value else {
        return Err(QueryError::Invalid(format!(
            "{path}.$filter.limit must resolve to a positive integer or null"
        )));
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(limit) = value.as_u64().filter(|limit| *limit > 0) else {
        return Err(QueryError::Invalid(format!(
            "{path}.$filter.limit must resolve to a positive integer or null"
        )));
    };
    Ok(Some(limit))
}

fn iteration_bindings(
    variable: &str,
    array_index_variable: &Option<String>,
    index: usize,
    item: Value,
) -> Vec<(String, Option<Value>)> {
    vec![
        (variable.to_owned(), Some(item)),
        index_binding(array_index_variable, index),
    ]
}

fn index_binding(array_index_variable: &Option<String>, index: usize) -> (String, Option<Value>) {
    (
        array_index_variable
            .clone()
            .unwrap_or_else(|| "IDX".to_owned()),
        Some(Value::from(index as u64)),
    )
}
