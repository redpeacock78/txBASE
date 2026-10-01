use super::super::{QueryError, ScalarExpression, parse_scalar_operand};
use super::StringExpression;
use serde_json::{Map, Value};

pub(super) fn parse(
    operator: &str,
    value: &Value,
    path: &str,
) -> Result<StringExpression, QueryError> {
    match operator {
        "$strLenCP" => Ok(StringExpression::Length(parse_scalar_operand(
            value,
            &format!("{path}.$strLenCP"),
        )?)),
        "$substrCP" => {
            let [input, start, count] = parse_array_operands(operator, value, path, 3, 3)?
                .try_into()
                .expect("operand count was checked");
            Ok(StringExpression::Substring {
                input,
                start,
                count,
            })
        }
        "$split" => {
            let [input, delimiter] = parse_array_operands(operator, value, path, 2, 2)?
                .try_into()
                .expect("operand count was checked");
            Ok(StringExpression::Split { input, delimiter })
        }
        "$indexOfCP" => {
            let operands = parse_array_operands(operator, value, path, 2, 4)?;
            let mut operands = operands.into_iter();
            let input = operands.next().expect("minimum operand count was checked");
            let search = operands.next().expect("minimum operand count was checked");
            let start = operands.next();
            let end = operands.next();
            Ok(StringExpression::IndexOf {
                input,
                search,
                start,
                end,
            })
        }
        "$replaceOne" | "$replaceAll" => {
            let object = value.as_object().ok_or_else(|| {
                QueryError::Invalid(format!("{path}.{operator} must be an object"))
            })?;
            parse_replace(operator, object, path)
        }
        _ => unreachable!("string expression operator was matched by caller"),
    }
}

fn parse_array_operands(
    operator: &str,
    value: &Value,
    path: &str,
    minimum: usize,
    maximum: usize,
) -> Result<Vec<ScalarExpression>, QueryError> {
    let operands = value
        .as_array()
        .ok_or_else(|| QueryError::Invalid(format!("{path}.{operator} must be an array")))?;
    if !(minimum..=maximum).contains(&operands.len()) {
        let expected = if minimum == maximum {
            format!("exactly {minimum}")
        } else {
            format!("between {minimum} and {maximum}")
        };
        return Err(QueryError::Invalid(format!(
            "{path}.{operator} requires {expected} operands"
        )));
    }
    operands
        .iter()
        .enumerate()
        .map(|(index, operand)| {
            parse_scalar_operand(operand, &format!("{path}.{operator}[{index}]"))
        })
        .collect()
}

fn parse_replace(
    operator: &str,
    object: &Map<String, Value>,
    path: &str,
) -> Result<StringExpression, QueryError> {
    if let Some(option) = object
        .keys()
        .find(|key| !matches!(key.as_str(), "input" | "find" | "replacement"))
    {
        return Err(QueryError::Invalid(format!(
            "{path}.{operator} has an unsupported option {option}"
        )));
    }
    let parse_required = |name: &str| {
        let value = object
            .get(name)
            .ok_or_else(|| QueryError::Invalid(format!("{path}.{operator} is missing {name}")))?;
        parse_scalar_operand(value, &format!("{path}.{operator}.{name}"))
    };
    Ok(StringExpression::Replace {
        input: parse_required("input")?,
        find: parse_required("find")?,
        replacement: parse_required("replacement")?,
        all: operator == "$replaceAll",
    })
}
