use super::super::codepages::{
    CP437_UPPER, CP850_UPPER, CP852_UPPER, CP866_UPPER, CP1250_UPPER, CP1251_UPPER, CP1253_UPPER,
    CP1254_UPPER, CP1255_UPPER, CP1256_UPPER, encode_codepage, encode_windows_1252,
};
use super::super::{DbfError, FieldDescriptor, binary_value};
use super::cjk::{encode as encode_cjk, encoding_name};
use super::temporal::{currency_i64, foxpro_datetime_bytes};
use serde_json::Value;

pub fn encode_field(
    field: &FieldDescriptor,
    value: &Value,
    language_driver: u8,
) -> Result<Vec<u8>, DbfError> {
    let length = usize::from(field.length);
    match field.field_type.to_ascii_uppercase() {
        b'Q' | b'V' => encode_variable_field(value, field, language_driver),
        b'C' => {
            let bytes = if field.is_binary() {
                binary_value(value, field)?
            } else {
                encode_character(value, field, language_driver)?
            };
            if bytes.len() > length {
                return Err(DbfError::Invalid(format!(
                    "value for {} exceeds field width {}",
                    field.name, field.length
                )));
            }
            let mut output = vec![if field.is_binary() { 0 } else { b' ' }; length];
            output[..bytes.len()].copy_from_slice(&bytes);
            Ok(output)
        }
        b'B' if length == 8 => Ok(value_f64(value, field)?.to_le_bytes().to_vec()),
        b'Y' => {
            if length != 8 {
                return Err(DbfError::Invalid(format!(
                    "currency field {} must be eight bytes",
                    field.name
                )));
            }
            Ok(currency_i64(value, field)?.to_le_bytes().to_vec())
        }
        b'B' | b'G' | b'M' | b'P' | b'W' if length == 4 => {
            Ok(value_u32(value, field)?.to_le_bytes().to_vec())
        }
        b'D' => {
            if length != 8 {
                return Err(DbfError::Invalid(format!(
                    "date field {} must be eight bytes",
                    field.name
                )));
            }
            let text = value_text(value, field)?;
            if text.is_empty() {
                return Ok(vec![b' '; length]);
            }
            if text.len() != length || !text.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(DbfError::Invalid(format!(
                    "date field {} requires YYYYMMDD or null",
                    field.name
                )));
            }
            Ok(text.into_bytes())
        }
        b'B' | b'G' | b'M' => {
            let text = value_text(value, field)?;
            if text.len() > length {
                return Err(DbfError::Invalid(format!(
                    "value for {} exceeds field width {}",
                    field.name, field.length
                )));
            }
            let mut bytes = vec![b' '; length];
            bytes[..text.len()].copy_from_slice(text.as_bytes());
            Ok(bytes)
        }
        b'@' | b'T' => {
            if value.is_null() {
                return Ok(if field.field_type.eq_ignore_ascii_case(&b'T') {
                    vec![0; length]
                } else {
                    vec![b' '; length]
                });
            }
            if field.field_type.eq_ignore_ascii_case(&b'T') && length == 8 {
                if let Some(text) = value.as_str() {
                    if let Some(bytes) = foxpro_datetime_bytes(text)? {
                        return Ok(bytes.to_vec());
                    }
                }
            }
            let bytes = binary_value(value, field)?;
            if bytes.len() != length {
                return Err(DbfError::Invalid(format!(
                    "timestamp field {} requires exactly {length} bytes",
                    field.name
                )));
            }
            Ok(bytes)
        }
        b'N' | b'F' => {
            let text = numeric_field_text(value, field)?;
            if text.len() > length {
                return Err(DbfError::Invalid(format!(
                    "value for {} exceeds field width {}",
                    field.name, field.length
                )));
            }
            let mut bytes = vec![b' '; length];
            let start = length - text.len();
            bytes[start..].copy_from_slice(text.as_bytes());
            Ok(bytes)
        }
        b'L' => {
            let marker = match value {
                Value::Null => b' ',
                Value::Bool(true) => b'T',
                Value::Bool(false) => b'F',
                Value::String(text) if text.len() == 1 => text.as_bytes()[0].to_ascii_uppercase(),
                _ => {
                    return Err(DbfError::Invalid(format!(
                        "logical field {} requires boolean, one-byte text, or null",
                        field.name
                    )));
                }
            };
            if !matches!(marker, b' ' | b'T' | b'F' | b'Y' | b'N') {
                return Err(DbfError::Invalid(format!(
                    "invalid logical marker for {}",
                    field.name
                )));
            }
            Ok(vec![marker; length])
        }
        b'I' | b'+' => {
            if length != 4 {
                return Err(DbfError::Invalid(format!(
                    "integer field {} must be four bytes",
                    field.name
                )));
            }
            let integer = value_i64(value, field)?;
            let integer = i32::try_from(integer).map_err(|_| {
                DbfError::Invalid(format!("integer value for {} is out of range", field.name))
            })?;
            Ok(integer.to_le_bytes().to_vec())
        }
        b'O' => {
            if length != 8 {
                return Err(DbfError::Invalid(format!(
                    "double field {} must be eight bytes",
                    field.name
                )));
            }
            let number = value_f64(value, field)?;
            Ok(number.to_le_bytes().to_vec())
        }
        field_type => Err(DbfError::Invalid(format!(
            "writing field type 0x{field_type:02x} is unsupported"
        ))),
    }
}

pub fn encode_variable_field(
    value: &Value,
    field: &FieldDescriptor,
    language_driver: u8,
) -> Result<Vec<u8>, DbfError> {
    let length = usize::from(field.length);
    if length == 0 {
        return Err(DbfError::Invalid(format!(
            "variable field {} must not be empty",
            field.name
        )));
    }
    if value.is_null() {
        return Ok(vec![0; length]);
    }

    let data = if field.is_binary() {
        binary_value(value, field)?
    } else {
        encode_character(value, field, language_driver)?
    };
    let max_length = length - 1;
    if data.len() > max_length {
        return Err(DbfError::Invalid(format!(
            "value for {} exceeds variable field width {}",
            field.name, field.length
        )));
    }
    let mut output = vec![if field.is_binary() { 0 } else { b' ' }; length];
    output[..data.len()].copy_from_slice(&data);
    output[max_length] = u8::try_from(data.len()).map_err(|_| {
        DbfError::Invalid(format!(
            "value for {} exceeds variable field width {}",
            field.name, field.length
        ))
    })?;
    Ok(output)
}

pub fn encode_character(
    value: &Value,
    field: &FieldDescriptor,
    language_driver: u8,
) -> Result<Vec<u8>, DbfError> {
    let text = value_text(value, field)?;
    let encoded = match language_driver {
        0x01 => encode_codepage(&text, CP437_UPPER),
        0x02 => encode_codepage(&text, CP850_UPPER),
        0x1f | 0x22 | 0x23 | 0x40 | 0x64 | 0x87 => encode_codepage(&text, CP852_UPPER),
        0x26 | 0x65 => encode_codepage(&text, CP866_UPPER),
        0xc8 => encode_codepage(&text, CP1250_UPPER),
        0xc9 => encode_codepage(&text, CP1251_UPPER),
        0xca => encode_codepage(&text, CP1254_UPPER),
        0xcb => encode_codepage(&text, CP1253_UPPER),
        0x7d => encode_codepage(&text, CP1255_UPPER),
        0x7e => encode_codepage(&text, CP1256_UPPER),
        0x03 | 0x57 => encode_windows_1252(&text),
        0x78..=0x7b => encode_cjk(&text, language_driver)
            .and_then(|(bytes, had_errors)| (!had_errors).then_some(bytes)),
        _ => Some(text.into_bytes()),
    };
    encoded.ok_or_else(|| {
        let code_page = encoding_name(language_driver).unwrap_or(match language_driver {
            0x01 => "CP437",
            0x02 => "CP850",
            0x1f | 0x22 | 0x23 | 0x40 | 0x64 | 0x87 => "CP852",
            0x26 | 0x65 => "CP866",
            0xc8 => "Windows-1250",
            0xc9 => "Windows-1251",
            0xca => "Windows-1254",
            0xcb => "Windows-1253",
            0x7d => "Windows-1255",
            0x7e => "Windows-1256",
            0x03 | 0x57 => "Windows-1252",
            _ => "the declared code page",
        });
        DbfError::Invalid(format!(
            "value for {} contains a character outside {code_page}",
            field.name
        ))
    })
}

pub fn value_text(value: &Value, field: &FieldDescriptor) -> Result<String, DbfError> {
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

pub fn numeric_field_text(value: &Value, field: &FieldDescriptor) -> Result<String, DbfError> {
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

pub fn value_i64(value: &Value, field: &FieldDescriptor) -> Result<i64, DbfError> {
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

pub fn value_u32(value: &Value, field: &FieldDescriptor) -> Result<u32, DbfError> {
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

pub fn value_f64(value: &Value, field: &FieldDescriptor) -> Result<f64, DbfError> {
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
