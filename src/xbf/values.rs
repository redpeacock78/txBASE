use super::codec::{push_u32, take, take_u8, take_u32, usize_from_u32};
use super::{XbfError, XbfField, XbfLimits, XbfRecord, XbfType, XbfValue};

pub(super) fn encode_record(
    fields: &[XbfField],
    record: &XbfRecord,
    limits: &XbfLimits,
) -> Result<Vec<u8>, XbfError> {
    if record.values.len() != fields.len() {
        return Err(XbfError::Invalid(
            "XBF record value count does not match schema".into(),
        ));
    }
    let mut bytes = Vec::new();
    for (field, value) in fields.iter().zip(&record.values) {
        let (tag, payload) = encode_value(field, value)?;
        if payload.len() > limits.max_value_size {
            return Err(XbfError::Invalid(format!(
                "value for field {} exceeds the configured limit",
                field.name
            )));
        }
        let payload_length = u32::try_from(payload.len())
            .map_err(|_| XbfError::Invalid("XBF value length overflows u32".into()))?;
        bytes.push(tag);
        push_u32(&mut bytes, payload_length);
        bytes.extend_from_slice(&payload);
    }
    if bytes.len() > limits.max_record_size {
        return Err(XbfError::Invalid(
            "record payload exceeds the configured limit".into(),
        ));
    }
    Ok(bytes)
}

pub(super) fn decode_record(
    bytes: &[u8],
    fields: &[XbfField],
    limits: &XbfLimits,
) -> Result<Vec<XbfValue>, XbfError> {
    if bytes.len() > limits.max_record_size {
        return Err(XbfError::Invalid(
            "record payload exceeds the configured limit".into(),
        ));
    }
    let mut cursor = 0;
    let mut values = Vec::with_capacity(fields.len());
    for field in fields {
        let tag = take_u8(bytes, &mut cursor)?;
        let length = usize_from_u32(take_u32(bytes, &mut cursor)?, "value length")?;
        if length > limits.max_value_size {
            return Err(XbfError::Invalid(format!(
                "value for field {} exceeds the configured limit",
                field.name
            )));
        }
        let payload = take(bytes, &mut cursor, length)?;
        values.push(decode_value(field, tag, payload)?);
    }
    if cursor != bytes.len() {
        return Err(XbfError::Invalid("XBF record has trailing bytes".into()));
    }
    Ok(values)
}

fn encode_value(field: &XbfField, value: &XbfValue) -> Result<(u8, Vec<u8>), XbfError> {
    let result = match (field.ty, value) {
        (_, XbfValue::Null) => {
            if !field.nullable && !field.primary_key {
                return Err(XbfError::Invalid(format!(
                    "non-nullable field {} contains NULL",
                    field.name
                )));
            }
            (0, Vec::new())
        }
        (XbfType::Boolean, XbfValue::Boolean(value)) => (0x01, vec![u8::from(*value)]),
        (XbfType::Signed32, XbfValue::Signed32(value)) => (0x10, value.to_le_bytes().to_vec()),
        (XbfType::Signed32, XbfValue::Signed64(value)) => {
            let value = i32::try_from(*value).map_err(|_| invalid_type(field))?;
            (0x10, value.to_le_bytes().to_vec())
        }
        (XbfType::Signed64, XbfValue::Signed32(value)) => (0x10, value.to_le_bytes().to_vec()),
        (XbfType::Signed64, XbfValue::Signed64(value)) => (0x11, value.to_le_bytes().to_vec()),
        (XbfType::Unsigned64, XbfValue::Unsigned64(value)) => (0x12, value.to_le_bytes().to_vec()),
        (XbfType::Float32, XbfValue::Float32(value)) => (0x20, value.to_le_bytes().to_vec()),
        (XbfType::Float64, XbfValue::Float64(value)) => (0x21, value.to_le_bytes().to_vec()),
        (XbfType::String, XbfValue::String(value)) => (0x30, value.as_bytes().to_vec()),
        (XbfType::Bytes, XbfValue::Bytes(value)) => (0x31, value.clone()),
        (XbfType::Date, XbfValue::Date(value)) => (0x40, value.to_le_bytes().to_vec()),
        (XbfType::Timestamp, XbfValue::Timestamp(value)) => (0x41, value.to_le_bytes().to_vec()),
        (XbfType::Uuid, XbfValue::Uuid(value)) => (0x60, value.to_vec()),
        (XbfType::Json, XbfValue::Json(value)) => (
            0x70,
            serde_json::to_vec(value).map_err(|error| {
                XbfError::Invalid(format!("JSON value cannot be encoded: {error}"))
            })?,
        ),
        _ => return Err(invalid_type(field)),
    };
    Ok(result)
}

fn decode_value(field: &XbfField, tag: u8, payload: &[u8]) -> Result<XbfValue, XbfError> {
    if tag == 0 {
        if !payload.is_empty() {
            return Err(XbfError::Invalid(format!(
                "NULL value for field {} has a payload",
                field.name
            )));
        }
        if !field.nullable && !field.primary_key {
            return Err(XbfError::Invalid(format!(
                "non-nullable field {} contains NULL",
                field.name
            )));
        }
        return Ok(XbfValue::Null);
    }
    let value = match (field.ty, tag) {
        (XbfType::Boolean, 0x01) if payload.len() == 1 => match payload[0] {
            0 => XbfValue::Boolean(false),
            1 => XbfValue::Boolean(true),
            _ => {
                return Err(XbfError::Invalid(format!(
                    "boolean value for field {} is not 0 or 1",
                    field.name
                )));
            }
        },
        (XbfType::Signed32, 0x10) if payload.len() == 4 => XbfValue::Signed32(i32::from_le_bytes(
            payload.try_into().expect("checked length"),
        )),
        (XbfType::Signed64, 0x10) if payload.len() == 4 => XbfValue::Signed32(i32::from_le_bytes(
            payload.try_into().expect("checked length"),
        )),
        (XbfType::Signed64, 0x11) if payload.len() == 8 => XbfValue::Signed64(i64::from_le_bytes(
            payload.try_into().expect("checked length"),
        )),
        (XbfType::Unsigned64, 0x12) if payload.len() == 8 => XbfValue::Unsigned64(
            u64::from_le_bytes(payload.try_into().expect("checked length")),
        ),
        (XbfType::Float32, 0x20) if payload.len() == 4 => XbfValue::Float32(f32::from_le_bytes(
            payload.try_into().expect("checked length"),
        )),
        (XbfType::Float64, 0x21) if payload.len() == 8 => XbfValue::Float64(f64::from_le_bytes(
            payload.try_into().expect("checked length"),
        )),
        (XbfType::String, 0x30) => {
            XbfValue::String(String::from_utf8(payload.to_vec()).map_err(|_| {
                XbfError::Invalid(format!("string field {} is not UTF-8", field.name))
            })?)
        }
        (XbfType::Bytes, 0x31) => XbfValue::Bytes(payload.to_vec()),
        (XbfType::Date, 0x40) if payload.len() == 4 => XbfValue::Date(i32::from_le_bytes(
            payload.try_into().expect("checked length"),
        )),
        (XbfType::Timestamp, 0x41) if payload.len() == 8 => XbfValue::Timestamp(
            i64::from_le_bytes(payload.try_into().expect("checked length")),
        ),
        (XbfType::Uuid, 0x60) if payload.len() == 16 => {
            XbfValue::Uuid(payload.try_into().expect("checked length"))
        }
        (XbfType::Json, 0x70) => {
            XbfValue::Json(serde_json::from_slice(payload).map_err(|error| {
                XbfError::Invalid(format!("JSON field {} is invalid: {error}", field.name))
            })?)
        }
        _ => return Err(invalid_type(field)),
    };
    Ok(value)
}

fn invalid_type(field: &XbfField) -> XbfError {
    XbfError::Invalid(format!("value does not match XBF field {}", field.name))
}
