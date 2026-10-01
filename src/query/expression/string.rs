use super::{EvaluationContext, QueryError, ScalarExpression, evaluate_scalar_in_context};
use serde_json::Value;

mod parser;

pub(super) fn parse(
    operator: &str,
    value: &Value,
    path: &str,
) -> Result<StringExpression, QueryError> {
    parser::parse(operator, value, path)
}

#[derive(Debug, Clone)]
pub(in crate::query) enum StringExpression {
    Length(ScalarExpression),
    Substring {
        input: ScalarExpression,
        start: ScalarExpression,
        count: ScalarExpression,
    },
    Split {
        input: ScalarExpression,
        delimiter: ScalarExpression,
    },
    IndexOf {
        input: ScalarExpression,
        search: ScalarExpression,
        start: Option<ScalarExpression>,
        end: Option<ScalarExpression>,
    },
    Replace {
        input: ScalarExpression,
        find: ScalarExpression,
        replacement: ScalarExpression,
        all: bool,
    },
}

pub(super) fn evaluate(
    context: &mut EvaluationContext<'_>,
    expression: &StringExpression,
    path: &str,
) -> Result<Option<Value>, QueryError> {
    match expression {
        StringExpression::Length(input) => evaluate_length(context, input, path),
        StringExpression::Substring {
            input,
            start,
            count,
        } => evaluate_substring(context, input, start, count, path),
        StringExpression::Split { input, delimiter } => {
            evaluate_split(context, input, delimiter, path)
        }
        StringExpression::IndexOf {
            input,
            search,
            start,
            end,
        } => evaluate_index(context, input, search, start.as_ref(), end.as_ref(), path),
        StringExpression::Replace {
            input,
            find,
            replacement,
            all,
        } => evaluate_replace(context, input, find, replacement, *all, path),
    }
}

fn evaluate_operand(
    context: &mut EvaluationContext<'_>,
    expression: &ScalarExpression,
    path: &str,
) -> Result<Option<Value>, QueryError> {
    evaluate_scalar_in_context(context, expression, path)
}

fn evaluate_length(
    context: &mut EvaluationContext<'_>,
    expression: &ScalarExpression,
    path: &str,
) -> Result<Option<Value>, QueryError> {
    let expression_path = format!("{path}.$strLenCP");
    let value = evaluate_operand(context, expression, &expression_path)?;
    let Some(Value::String(input)) = value else {
        return Err(QueryError::Invalid(format!(
            "{expression_path} requires a string"
        )));
    };
    let length = u64::try_from(input.chars().count()).map_err(|_| {
        QueryError::Invalid(format!(
            "{expression_path} result exceeds the integer range"
        ))
    })?;
    Ok(Some(Value::Number(length.into())))
}

fn evaluate_substring(
    context: &mut EvaluationContext<'_>,
    input_expression: &ScalarExpression,
    start_expression: &ScalarExpression,
    count_expression: &ScalarExpression,
    path: &str,
) -> Result<Option<Value>, QueryError> {
    let expression_path = format!("{path}.$substrCP");
    let input = evaluate_operand(context, input_expression, &format!("{expression_path}[0]"))?;
    if input.as_ref().is_none_or(Value::is_null) {
        return Ok(Some(Value::String(String::new())));
    }
    let Some(Value::String(input)) = input else {
        return Err(QueryError::Invalid(format!(
            "{expression_path}[0] must resolve to a string or null"
        )));
    };
    let start = non_negative_integer(
        evaluate_operand(context, start_expression, &format!("{expression_path}[1]"))?,
        &format!("{expression_path}[1]"),
    )?;
    let count = non_negative_integer(
        evaluate_operand(context, count_expression, &format!("{expression_path}[2]"))?,
        &format!("{expression_path}[2]"),
    )?;
    let start = usize::try_from(start).unwrap_or(usize::MAX);
    let count = usize::try_from(count).unwrap_or(usize::MAX);
    let mut output = String::new();
    for character in input.chars().skip(start).take(count) {
        push_character(&mut output, character, &expression_path)?;
    }
    Ok(Some(Value::String(output)))
}

fn evaluate_split(
    context: &mut EvaluationContext<'_>,
    input_expression: &ScalarExpression,
    delimiter_expression: &ScalarExpression,
    path: &str,
) -> Result<Option<Value>, QueryError> {
    let expression_path = format!("{path}.$split");
    let input = evaluate_operand(context, input_expression, &format!("{expression_path}[0]"))?;
    let delimiter = evaluate_operand(
        context,
        delimiter_expression,
        &format!("{expression_path}[1]"),
    )?;
    let input = require_string(input, &format!("{expression_path}[0]"))?;
    let delimiter = require_string(delimiter, &format!("{expression_path}[1]"))?;
    if delimiter.is_empty() {
        return Err(QueryError::Invalid(format!(
            "{expression_path}[1] must not be empty"
        )));
    }
    let mut output = Vec::new();
    let mut output_bytes = 2;
    for part in input.split(&delimiter) {
        super::array::push_array_value(
            &mut output,
            &mut output_bytes,
            Value::String(part.to_owned()),
            &format!("{expression_path} result"),
        )?;
    }
    Ok(Some(Value::Array(output)))
}

fn evaluate_index(
    context: &mut EvaluationContext<'_>,
    input_expression: &ScalarExpression,
    search_expression: &ScalarExpression,
    start_expression: Option<&ScalarExpression>,
    end_expression: Option<&ScalarExpression>,
    path: &str,
) -> Result<Option<Value>, QueryError> {
    let expression_path = format!("{path}.$indexOfCP");
    let input = evaluate_operand(context, input_expression, &format!("{expression_path}[0]"))?;
    if input.as_ref().is_none_or(Value::is_null) {
        return Ok(Some(Value::Null));
    }
    let input = require_string(input, &format!("{expression_path}[0]"))?;
    let search = require_string(
        evaluate_operand(context, search_expression, &format!("{expression_path}[1]"))?,
        &format!("{expression_path}[1]"),
    )?;
    let length = u64::try_from(input.chars().count()).expect("a string length fits in u64");
    let start = start_expression
        .map(|expression| {
            non_negative_integer(
                evaluate_operand(context, expression, &format!("{expression_path}[2]"))?,
                &format!("{expression_path}[2]"),
            )
        })
        .transpose()?
        .unwrap_or(0);
    let end = end_expression
        .map(|expression| {
            non_negative_integer(
                evaluate_operand(context, expression, &format!("{expression_path}[3]"))?,
                &format!("{expression_path}[3]"),
            )
        })
        .transpose()?
        .unwrap_or(length);
    let result = if start > length || start > end {
        -1
    } else {
        let end = end.min(length);
        let start_byte = byte_offset(&input, start);
        let end_byte = byte_offset(&input, end);
        input[start_byte..end_byte]
            .find(&search)
            .map(|byte_index| {
                let prefix = &input[start_byte..start_byte + byte_index];
                i64::try_from(start + prefix.chars().count() as u64)
                    .expect("string code-point indices fit in i64")
            })
            .unwrap_or(-1)
    };
    Ok(Some(Value::Number(result.into())))
}

fn evaluate_replace(
    context: &mut EvaluationContext<'_>,
    input_expression: &ScalarExpression,
    find_expression: &ScalarExpression,
    replacement_expression: &ScalarExpression,
    all: bool,
    path: &str,
) -> Result<Option<Value>, QueryError> {
    let operator = if all { "$replaceAll" } else { "$replaceOne" };
    let expression_path = format!("{path}.{operator}");
    let input = evaluate_operand(
        context,
        input_expression,
        &format!("{expression_path}.input"),
    )?;
    let find = evaluate_operand(context, find_expression, &format!("{expression_path}.find"))?;
    let replacement = evaluate_operand(
        context,
        replacement_expression,
        &format!("{expression_path}.replacement"),
    )?;
    let (Some(input), Some(find), Some(replacement)) = (input, find, replacement) else {
        return Ok(Some(Value::Null));
    };
    if input.is_null() || find.is_null() || replacement.is_null() {
        return Ok(Some(Value::Null));
    }
    let input = require_string(Some(input), &format!("{expression_path}.input"))?;
    let find = require_string(Some(find), &format!("{expression_path}.find"))?;
    let replacement = require_string(Some(replacement), &format!("{expression_path}.replacement"))?;
    if find.is_empty() {
        return Err(QueryError::Invalid(format!(
            "{expression_path}.find must not be empty"
        )));
    }
    Ok(Some(Value::String(replace_string(
        &input,
        &find,
        &replacement,
        all,
        &expression_path,
    )?)))
}

fn replace_string(
    input: &str,
    find: &str,
    replacement: &str,
    all: bool,
    path: &str,
) -> Result<String, QueryError> {
    let mut output = String::new();
    let mut copied_until = 0;
    for (match_start, matched) in input.match_indices(find) {
        push_string(&mut output, &input[copied_until..match_start], path)?;
        push_string(&mut output, replacement, path)?;
        copied_until = match_start + matched.len();
        if !all {
            break;
        }
    }
    push_string(&mut output, &input[copied_until..], path)?;
    Ok(output)
}

fn require_string(value: Option<Value>, path: &str) -> Result<String, QueryError> {
    match value {
        Some(Value::String(value)) => Ok(value),
        _ => Err(QueryError::Invalid(format!(
            "{path} must resolve to a string"
        ))),
    }
}

fn non_negative_integer(value: Option<Value>, path: &str) -> Result<u64, QueryError> {
    let Some(Value::Number(number)) = value else {
        return Err(QueryError::Invalid(format!(
            "{path} must resolve to a non-negative integer"
        )));
    };
    if let Some(value) = number.as_u64() {
        return Ok(value);
    }
    let value = number.as_f64().filter(|value| {
        value.is_finite() && *value >= 0.0 && value.fract() == 0.0 && *value < u64::MAX as f64
    });
    value.map(|value| value as u64).ok_or_else(|| {
        QueryError::Invalid(format!("{path} must resolve to a non-negative integer"))
    })
}

fn byte_offset(input: &str, codepoint_index: u64) -> usize {
    usize::try_from(codepoint_index)
        .ok()
        .and_then(|index| input.char_indices().nth(index).map(|(offset, _)| offset))
        .unwrap_or(input.len())
}

fn push_character(output: &mut String, character: char, path: &str) -> Result<(), QueryError> {
    if character.len_utf8() > super::MAX_STRING_EXPRESSION_BYTES.saturating_sub(output.len()) {
        return Err(string_limit_error(path));
    }
    output.push(character);
    Ok(())
}

fn push_string(output: &mut String, value: &str, path: &str) -> Result<(), QueryError> {
    if value.len() > super::MAX_STRING_EXPRESSION_BYTES.saturating_sub(output.len()) {
        return Err(string_limit_error(path));
    }
    output.push_str(value);
    Ok(())
}

fn string_limit_error(path: &str) -> QueryError {
    QueryError::Invalid(format!(
        "{path} result exceeds {} bytes",
        super::MAX_STRING_EXPRESSION_BYTES
    ))
}

#[cfg(test)]
mod tests;
