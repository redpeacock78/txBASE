use super::codec::{
    check_section_size, push_u16, push_u32, take_u8, take_u16, take_u32, usize_from_u32,
};
use super::{XbfError, XbfField, XbfLimits, XbfRecord, XbfType, XbfValue};
use std::collections::BTreeSet;

pub(super) fn encode_schema(fields: &[XbfField], limits: &XbfLimits) -> Result<Vec<u8>, XbfError> {
    let count = u32::try_from(fields.len())
        .map_err(|_| XbfError::Invalid("field count overflows u32".into()))?;
    let mut bytes = Vec::new();
    push_u32(&mut bytes, count);
    let mut names = BTreeSet::new();
    let mut primary_key_count = 0;
    for field in fields {
        let name = field.name.as_bytes();
        if name.is_empty() || name.len() > 255 || name.len() > limits.max_field_name {
            return Err(XbfError::Invalid(format!(
                "field name {} is outside the supported length",
                field.name
            )));
        }
        if !names.insert(field.name.clone()) {
            return Err(XbfError::Invalid(format!(
                "duplicate XBF field name {}",
                field.name
            )));
        }
        if field.primary_key {
            primary_key_count += 1;
            if field.nullable {
                return Err(XbfError::Invalid(format!(
                    "primary-key field {} cannot be nullable",
                    field.name
                )));
            }
        }
        let flags = (if field.nullable { 0x01 } else { 0 })
            | (if field.primary_key { 0x02 } else { 0 })
            | (if field.unique || field.primary_key {
                0x04
            } else {
                0
            });
        push_u16(&mut bytes, name.len() as u16);
        bytes.extend_from_slice(name);
        bytes.push(field.ty.tag());
        bytes.push(flags);
        push_u16(&mut bytes, 0);
    }
    if primary_key_count > 1 {
        return Err(XbfError::Invalid(
            "XBF v1 supports at most one primary-key field".into(),
        ));
    }
    check_section_size(bytes.len(), limits, "schema")?;
    Ok(bytes)
}

pub(super) fn decode_schema(bytes: &[u8], limits: &XbfLimits) -> Result<Vec<XbfField>, XbfError> {
    let mut cursor = 0;
    let count = usize_from_u32(take_u32(bytes, &mut cursor)?, "field count")?;
    if count > limits.max_fields {
        return Err(XbfError::Invalid(
            "field count exceeds the configured limit".into(),
        ));
    }
    let mut names = BTreeSet::new();
    let mut fields = Vec::with_capacity(count);
    let mut primary_key_count = 0;
    for _ in 0..count {
        let name_length = usize::from(take_u16(bytes, &mut cursor)?);
        if name_length == 0 || name_length > 255 || name_length > limits.max_field_name {
            return Err(XbfError::Invalid(
                "XBF field name length is outside 1..=255".into(),
            ));
        }
        let name_bytes = super::codec::take(bytes, &mut cursor, name_length)?;
        let name = String::from_utf8(name_bytes.to_vec())
            .map_err(|_| XbfError::Invalid("XBF field name is not UTF-8".into()))?;
        if !names.insert(name.clone()) {
            return Err(XbfError::Invalid(format!(
                "duplicate XBF field name {name}"
            )));
        }
        let ty = XbfType::from_tag(take_u8(bytes, &mut cursor)?)?;
        let flags = take_u8(bytes, &mut cursor)?;
        if flags & !0x07 != 0 {
            return Err(XbfError::Invalid(
                "XBF schema contains unknown constraint flags".into(),
            ));
        }
        if take_u16(bytes, &mut cursor)? != 0 {
            return Err(XbfError::Invalid(
                "XBF schema reserved field is non-zero".into(),
            ));
        }
        let nullable = flags & 0x01 != 0;
        let primary_key = flags & 0x02 != 0;
        let unique = flags & 0x04 != 0 || primary_key;
        if primary_key {
            primary_key_count += 1;
            if nullable {
                return Err(XbfError::Invalid(
                    "XBF primary-key field must be non-null".into(),
                ));
            }
        }
        fields.push(XbfField {
            name,
            ty,
            nullable,
            primary_key,
            unique,
        });
    }
    if primary_key_count > 1 {
        return Err(XbfError::Invalid(
            "XBF v1 supports at most one primary-key field".into(),
        ));
    }
    if cursor != bytes.len() {
        return Err(XbfError::Invalid("XBF schema has trailing bytes".into()));
    }
    Ok(fields)
}

pub(super) fn validate_constraints(
    fields: &[XbfField],
    records: &[XbfRecord],
) -> Result<(), XbfError> {
    let mut primary_key_count = 0;
    for (field_index, field) in fields.iter().enumerate() {
        if field.primary_key {
            primary_key_count += 1;
            if field.nullable {
                return Err(XbfError::Invalid(format!(
                    "primary-key field {} cannot be nullable",
                    field.name
                )));
            }
        }
        if !field.unique && !field.primary_key {
            continue;
        }
        let mut keys = BTreeSet::new();
        for record in records {
            let value = record.values.get(field_index).ok_or_else(|| {
                XbfError::Invalid("XBF record value count does not match schema".into())
            })?;
            let Some(key) = unique_key(field, value)? else {
                if field.primary_key {
                    return Err(XbfError::Invalid(format!(
                        "primary-key field {} contains NULL",
                        field.name
                    )));
                }
                continue;
            };
            if !keys.insert(key) {
                return Err(XbfError::Invalid(format!(
                    "unique field {} contains duplicate values",
                    field.name
                )));
            }
        }
    }
    if primary_key_count > 1 {
        return Err(XbfError::Invalid(
            "XBF v1 supports at most one primary-key field".into(),
        ));
    }
    Ok(())
}

fn unique_key(field: &XbfField, value: &XbfValue) -> Result<Option<Vec<u8>>, XbfError> {
    if matches!(value, XbfValue::Null) {
        return Ok(None);
    }
    let key = match (field.ty, value) {
        (XbfType::Boolean, XbfValue::Boolean(value)) => vec![u8::from(*value)],
        (XbfType::Signed32, XbfValue::Signed32(value)) => value.to_le_bytes().to_vec(),
        (XbfType::Signed32, XbfValue::Signed64(value)) => i32::try_from(*value)
            .map_err(|_| invalid_type(field))?
            .to_le_bytes()
            .to_vec(),
        (XbfType::Signed64, XbfValue::Signed32(value)) => i64::from(*value).to_le_bytes().to_vec(),
        (XbfType::Signed64, XbfValue::Signed64(value)) => value.to_le_bytes().to_vec(),
        (XbfType::Unsigned64, XbfValue::Unsigned64(value)) => value.to_le_bytes().to_vec(),
        (XbfType::Float32, XbfValue::Float32(value)) => value.to_bits().to_le_bytes().to_vec(),
        (XbfType::Float64, XbfValue::Float64(value)) => value.to_bits().to_le_bytes().to_vec(),
        (XbfType::String, XbfValue::String(value)) => value.as_bytes().to_vec(),
        (XbfType::Bytes, XbfValue::Bytes(value)) => value.clone(),
        (XbfType::Date, XbfValue::Date(value)) => value.to_le_bytes().to_vec(),
        (XbfType::Timestamp, XbfValue::Timestamp(value)) => value.to_le_bytes().to_vec(),
        (XbfType::Uuid, XbfValue::Uuid(value)) => value.to_vec(),
        (XbfType::Json, XbfValue::Json(value)) => serde_json::to_vec(value).map_err(|error| {
            XbfError::Invalid(format!("JSON value cannot be compared: {error}"))
        })?,
        _ => return Err(invalid_type(field)),
    };
    Ok(Some(key))
}

fn invalid_type(field: &XbfField) -> XbfError {
    XbfError::Invalid(format!("value does not match XBF field {}", field.name))
}
