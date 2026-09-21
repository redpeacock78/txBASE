use super::super::DbfError;
use serde_json::{Map, Number, Value};
use std::collections::BTreeSet;

pub(super) fn expand_update(
    current: &Map<String, Value>,
    update: Map<String, Value>,
) -> Result<(Map<String, Value>, BTreeSet<String>), DbfError> {
    let has_operator = update.keys().any(|key| key.starts_with('$'));
    if !has_operator {
        let changed_fields = update.keys().cloned().collect();
        let mut values = current.clone();
        values.extend(update);
        return Ok((values, changed_fields));
    }
    if update.keys().any(|key| !key.starts_with('$')) {
        return Err(DbfError::Invalid(
            "update cannot mix operators and fields".into(),
        ));
    }

    let mut values = current.clone();
    let mut changed_fields = BTreeSet::new();
    for (operator, operand) in update {
        match operator.as_str() {
            "$set" | "$unset" | "$inc" => {}
            _ => {
                return Err(DbfError::Invalid(format!(
                    "unsupported update operator {operator}"
                )));
            }
        }
        let fields = operand
            .as_object()
            .ok_or_else(|| DbfError::Invalid(format!("{operator} requires an object")))?;
        for field in fields.keys() {
            if !changed_fields.insert(field.clone()) {
                return Err(DbfError::Invalid(format!(
                    "field {field} appears in multiple update operators"
                )));
            }
        }
        match operator.as_str() {
            "$set" => values.extend(fields.clone()),
            "$unset" => {
                for field in fields.keys() {
                    values.insert(field.clone(), Value::Null);
                }
            }
            "$inc" => {
                for (field, increment) in fields {
                    let value = increment_value(values.get(field), increment, field)?;
                    values.insert(field.clone(), value);
                }
            }
            _ => unreachable!("update operator was validated above"),
        }
    }
    Ok((values, changed_fields))
}

fn increment_value(
    current: Option<&Value>,
    increment: &Value,
    field: &str,
) -> Result<Value, DbfError> {
    let Some(Value::Number(current)) = current else {
        return Err(DbfError::Invalid(format!(
            "$inc requires a numeric value in field {field}"
        )));
    };
    let Value::Number(increment) = increment else {
        return Err(DbfError::Invalid(format!(
            "$inc value for {field} must be a JSON number"
        )));
    };
    if let (Some(current), Some(increment)) = (current.as_i64(), increment.as_i64()) {
        let value = current
            .checked_add(increment)
            .ok_or_else(|| DbfError::Invalid(format!("$inc overflows integer field {field}")))?;
        return Ok(Value::Number(value.into()));
    }
    let value = current
        .as_f64()
        .and_then(|current| increment.as_f64().map(|increment| current + increment))
        .and_then(Number::from_f64)
        .ok_or_else(|| DbfError::Invalid(format!("$inc result for {field} is not finite")))?;
    Ok(Value::Number(value))
}
