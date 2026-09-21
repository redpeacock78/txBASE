use super::super::{DbfError, FieldDescriptor};
use serde_json::Value;

pub(in crate::dbf) fn value_text(
    value: &Value,
    field: &FieldDescriptor,
) -> Result<String, DbfError> {
    match value {
        Value::Null => Ok(String::new()),
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        Value::Bool(boolean) => Ok(boolean.to_string()),
        _ => Err(DbfError::Invalid(format!(
            "field {} requires a scalar value",
            field.name
        ))),
    }
}

pub(super) fn numeric_field_text(
    value: &Value,
    field: &FieldDescriptor,
) -> Result<String, DbfError> {
    let text = value_text(value, field)?;
    if text.is_empty() {
        return Ok(text);
    }
    let text = text.trim();
    let number = text.parse::<f64>().map_err(|_| {
        DbfError::Invalid(format!(
            "numeric field {} requires a finite number or null",
            field.name
        ))
    })?;
    if !number.is_finite() {
        return Err(DbfError::Invalid(format!(
            "numeric field {} requires a finite number or null",
            field.name
        )));
    }
    Ok(text.to_owned())
}

pub(super) fn value_i64(value: &Value, field: &FieldDescriptor) -> Result<i64, DbfError> {
    match value {
        Value::Null => Ok(0),
        Value::Number(number) => number
            .as_i64()
            .ok_or_else(|| DbfError::Invalid(format!("field {} requires an integer", field.name))),
        Value::String(text) => text
            .trim()
            .parse::<i64>()
            .map_err(|_| DbfError::Invalid(format!("field {} requires an integer", field.name))),
        _ => Err(DbfError::Invalid(format!(
            "field {} requires an integer",
            field.name
        ))),
    }
}

pub(super) fn value_u32(value: &Value, field: &FieldDescriptor) -> Result<u32, DbfError> {
    match value {
        Value::Null => Ok(0),
        Value::Number(number) => number
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| DbfError::Invalid(format!("field {} requires a uint32", field.name))),
        Value::String(text) => text
            .trim()
            .parse::<u32>()
            .map_err(|_| DbfError::Invalid(format!("field {} requires a uint32", field.name))),
        _ => Err(DbfError::Invalid(format!(
            "field {} requires a uint32",
            field.name
        ))),
    }
}

pub(super) fn value_f64(value: &Value, field: &FieldDescriptor) -> Result<f64, DbfError> {
    let number = match value {
        Value::Null => 0.0,
        Value::Number(number) => number.as_f64().ok_or_else(|| {
            DbfError::Invalid(format!("field {} requires a finite number", field.name))
        })?,
        Value::String(text) => text.trim().parse::<f64>().map_err(|_| {
            DbfError::Invalid(format!("field {} requires a finite number", field.name))
        })?,
        _ => {
            return Err(DbfError::Invalid(format!(
                "field {} requires a number",
                field.name
            )));
        }
    };
    if number.is_finite() {
        Ok(number)
    } else {
        Err(DbfError::Invalid(format!(
            "field {} requires a finite number",
            field.name
        )))
    }
}
