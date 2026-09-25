use super::{XbfError, XbfField, XbfType, XbfValue};
use serde_json::{Map, Value, json};
use std::fmt::Write;

pub(crate) fn record_values(
    fields: &[XbfField],
    values: &[XbfValue],
) -> Result<Map<String, Value>, XbfError> {
    if fields.len() != values.len() {
        return Err(XbfError::Invalid(
            "XBF record value count does not match schema".into(),
        ));
    }

    let mut result = Map::new();
    for (field, value) in fields.iter().zip(values) {
        if result.contains_key(&field.name) {
            return Err(XbfError::Invalid(format!(
                "XBF field name {:?} is duplicated",
                field.name
            )));
        }
        result.insert(field.name.clone(), query_value(field, value)?);
    }
    Ok(result)
}

fn query_value(field: &XbfField, value: &XbfValue) -> Result<Value, XbfError> {
    match super::export::value_to_dbf(field, value) {
        Ok(value) => Ok(value),
        Err(error) => match (field.ty, value) {
            (XbfType::Signed64, XbfValue::Signed32(value)) => Ok((*value).into()),
            (XbfType::Unsigned64, XbfValue::Unsigned64(value)) => {
                Ok(Value::Number((*value).into()))
            }
            (XbfType::Float32, XbfValue::Float32(value)) if !value.is_finite() => {
                Ok(json!({"$txbaseFloat32Bits": format!("{:08x}", value.to_bits())}))
            }
            (XbfType::Float64, XbfValue::Float64(value)) if !value.is_finite() => {
                Ok(json!({"$txbaseFloat64Bits": format!("{:016x}", value.to_bits())}))
            }
            (XbfType::Date, XbfValue::Date(days)) => Ok((*days).into()),
            (XbfType::Timestamp, XbfValue::Timestamp(milliseconds)) => Ok((*milliseconds).into()),
            (XbfType::Uuid, XbfValue::Uuid(bytes)) => Ok(Value::String(uuid_text(bytes))),
            (XbfType::Json, XbfValue::Json(value)) => Ok(value.clone()),
            _ => Err(error),
        },
    }
}

fn uuid_text(bytes: &[u8; 16]) -> String {
    let mut text = String::with_capacity(36);
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            text.push('-');
        }
        write!(&mut text, "{byte:02x}").expect("writing to a String cannot fail");
    }
    text
}
