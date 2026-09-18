use super::codepages::{
    CP437_UPPER, CP850_UPPER, CP852_UPPER, CP866_UPPER, CP1250_UPPER, CP1251_UPPER, CP1253_UPPER,
    CP1254_UPPER, CP1255_UPPER, CP1256_UPPER, decode_codepage, decode_windows_1252,
    encode_codepage, encode_windows_1252,
};
use super::{DbfError, FieldDescriptor, MemoFormat, NullFlagBits, binary_value};
use serde_json::{Number, Value};

#[path = "codec_fields.rs"]
mod fields;
#[path = "codec_temporal.rs"]
mod temporal;

pub(super) use fields::{
    encode_null_flags, flag_is_set, null_flag_layout, parse_fields, read_u16, read_u32,
    system_field_index, update_null_flags,
};
pub(super) use temporal::{
    currency_i64, currency_text, foxpro_datetime_bytes, foxpro_datetime_text,
};

pub(super) fn encode_field(
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

pub(super) fn encode_variable_field(
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

pub(super) fn encode_character(
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
        _ => Some(text.into_bytes()),
    };
    encoded.ok_or_else(|| {
        let code_page = match language_driver {
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
        };
        DbfError::Invalid(format!(
            "value for {} contains a character outside {code_page}",
            field.name
        ))
    })
}

pub(super) fn value_text(value: &Value, field: &FieldDescriptor) -> Result<String, DbfError> {
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

pub(super) fn decode_record_field(
    field: &FieldDescriptor,
    bytes: &[u8],
    language_driver: u8,
    null_flags: Option<&[u8]>,
    flag_bits: Option<NullFlagBits>,
) -> Value {
    let is_null = flag_bits
        .and_then(|bits| bits.nullable)
        .is_some_and(|bit| null_flags.is_some_and(|flags| flag_is_set(flags, bit)));
    if is_null {
        return Value::Null;
    }

    let data = match flag_bits.and_then(|bits| bits.varlength) {
        Some(bit) if null_flags.is_some_and(|flags| flag_is_set(flags, bit)) => {
            match bytes.last().map(|length| usize::from(*length)) {
                Some(length) if length <= bytes.len().saturating_sub(1) => &bytes[..length],
                _ => bytes,
            }
        }
        _ => bytes,
    };
    if field.is_binary() {
        Value::String(hex(data))
    } else {
        Value::String(text(data, language_driver))
    }
}

pub(super) fn decode_field(
    field_type: u8,
    bytes: &[u8],
    language_driver: u8,
    memo_format: Option<MemoFormat>,
) -> Value {
    match field_type.to_ascii_uppercase() {
        b'C' => Value::String(text(bytes, language_driver)),
        b'D' if bytes.iter().all(|byte| matches!(*byte, b' ' | 0)) => Value::Null,
        b'T' if memo_format == Some(MemoFormat::FoxPro) && bytes.len() >= 8 => {
            let day = u32::from_le_bytes(bytes[..4].try_into().unwrap());
            if day == 0 {
                Value::Null
            } else if let Some(value) = foxpro_datetime_text(bytes) {
                Value::String(value)
            } else {
                Value::String(hex(bytes))
            }
        }
        b'Y' if bytes.len() >= 8 => Value::String(currency_text(bytes)),
        b'B' if memo_format.is_some_and(|format| format != MemoFormat::FoxPro)
            && bytes.len() != 4 =>
        {
            numeric(bytes)
        }
        b'B' if bytes.len() >= 8 => {
            let value = f64::from_le_bytes(bytes[..8].try_into().unwrap());
            if value.is_finite() {
                Number::from_f64(value)
                    .map(Value::Number)
                    .unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
        b'B' | b'G' | b'M' | b'P' | b'W' if bytes.len() == 4 => {
            let block = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            Value::Number(block.into())
        }
        b'D' | b'B' | b'G' | b'M' | b'P' | b'W' => Value::String(text(bytes, 0)),
        b'F' | b'N' => numeric(bytes),
        b'L' => match bytes.first().map(|byte| byte.to_ascii_uppercase()) {
            Some(b'T') | Some(b'Y') => Value::Bool(true),
            Some(b'F') | Some(b'N') => Value::Bool(false),
            _ => Value::Null,
        },
        b'I' | b'+' if bytes.len() >= 4 => {
            Value::Number(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]).into())
        }
        b'O' if bytes.len() >= 8 => {
            let value = f64::from_le_bytes(bytes[..8].try_into().unwrap());
            if value.is_finite() {
                Number::from_f64(value)
                    .map(Value::Number)
                    .unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
        b'@' | b'T' => Value::String(hex(bytes)),
        _ => Value::String(text(bytes, language_driver)),
    }
}

pub(super) fn text(bytes: &[u8], language_driver: u8) -> String {
    let end = bytes
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\0'))
        .map_or(0, |index| index + 1);
    let bytes = &bytes[..end];
    match language_driver {
        0x01 => decode_codepage(bytes, CP437_UPPER),
        0x02 => decode_codepage(bytes, CP850_UPPER),
        0x1f | 0x22 | 0x23 | 0x40 | 0x64 | 0x87 => decode_codepage(bytes, CP852_UPPER),
        0x26 | 0x65 => decode_codepage(bytes, CP866_UPPER),
        0xc8 => decode_codepage(bytes, CP1250_UPPER),
        0xc9 => decode_codepage(bytes, CP1251_UPPER),
        0xca => decode_codepage(bytes, CP1254_UPPER),
        0xcb => decode_codepage(bytes, CP1253_UPPER),
        0x7d => decode_codepage(bytes, CP1255_UPPER),
        0x7e => decode_codepage(bytes, CP1256_UPPER),
        0x03 | 0x57 => decode_windows_1252(bytes),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

pub(super) fn numeric(bytes: &[u8]) -> Value {
    let value = text(bytes, 0).trim().to_owned();
    if value.is_empty() {
        return Value::Null;
    }
    if let Ok(integer) = value.parse::<i64>() {
        return Value::Number(integer.into());
    }
    if let Ok(float) = value.parse::<f64>() {
        if let Some(number) = Number::from_f64(float) {
            return Value::Number(number);
        }
    }
    Value::String(value)
}

pub(super) fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}
