use super::super::codepages::{
    CP437_UPPER, CP850_UPPER, CP852_UPPER, CP866_UPPER, CP1250_UPPER, CP1251_UPPER, CP1253_UPPER,
    CP1254_UPPER, CP1255_UPPER, CP1256_UPPER, decode_codepage, decode_windows_1252,
};
use super::super::{FieldDescriptor, MemoFormat, NullFlagBits};
use super::cjk::decode as decode_cjk;
use super::fields::flag_is_set;
use super::temporal::{currency_text, foxpro_datetime_text};
use serde_json::{Number, Value};

pub fn decode_record_field(
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

pub fn decode_field(
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

pub fn text(bytes: &[u8], language_driver: u8) -> String {
    let end = bytes
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\0'))
        .map_or(0, |index| index + 1);
    let bytes = &bytes[..end];
    if let Some(text) = decode_cjk(bytes, language_driver) {
        return text;
    }
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

pub fn numeric(bytes: &[u8]) -> Value {
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

pub fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}
