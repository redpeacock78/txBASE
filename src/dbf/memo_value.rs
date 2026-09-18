use super::super::{DbfError, EOF_MARKER, FieldDescriptor, MemoFormat, MemoUpdate, value_text};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn find_memo_path(path: &Path) -> Option<PathBuf> {
    let stem = path.file_stem()?;
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    for extension in ["fpt", "dbt"] {
        let candidate = path.with_extension(extension);
        if candidate.is_file() {
            return Some(candidate);
        }
        let candidate = path.with_extension(extension.to_ascii_uppercase());
        if candidate.is_file() {
            return Some(candidate);
        }
        let entries = fs::read_dir(directory).ok()?;
        if let Some(candidate) = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|candidate| {
                candidate.file_stem() == Some(stem)
                    && candidate
                        .extension()
                        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
            })
        {
            return Some(candidate);
        }
    }
    None
}

pub(crate) fn memo_index(bytes: &[u8], _format: MemoFormat) -> Result<Option<u32>, DbfError> {
    if bytes.iter().all(|byte| matches!(byte, b' ' | b'\0')) {
        return Ok(None);
    }
    if bytes.len() == 4 {
        let block = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        return Ok(Some(block));
    }
    let end = bytes
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\0'))
        .map_or(0, |index| index + 1);
    if end == 0 {
        return Ok(None);
    }
    let text = std::str::from_utf8(&bytes[..end])
        .map_err(|_| DbfError::Invalid("memo block number is not ASCII".into()))?
        .trim_matches([' ', '\0']);
    text.parse::<u32>()
        .map(Some)
        .map_err(|_| DbfError::Invalid("memo block number is not an integer".into()))
}

fn is_memo_field(field_type: u8) -> bool {
    field_type.eq_ignore_ascii_case(&b'M')
}

pub(crate) fn is_sidecar_field(field: &FieldDescriptor) -> bool {
    is_memo_field(field.field_type)
        || matches!(field.field_type.to_ascii_uppercase(), b'G' | b'P' | b'W')
        || (field.field_type.eq_ignore_ascii_case(&b'B') && field.length != 8)
}

pub(crate) fn storage_value_without_sidecar(
    value: &Value,
    field: &FieldDescriptor,
) -> Result<Value, DbfError> {
    let empty = if field.is_binary() {
        binary_value(value, field)?.is_empty()
    } else {
        value_text(value, field)?.is_empty()
    };
    if !empty {
        return Err(DbfError::Invalid(format!(
            "memo sidecar is missing for non-empty field {}",
            field.name
        )));
    }
    Ok(empty_memo_value(field))
}

pub(crate) fn sidecar_update(
    value: &Value,
    field: &FieldDescriptor,
    memo_format: MemoFormat,
) -> Result<Option<MemoUpdate>, DbfError> {
    if !field.is_binary() {
        let text = value_text(value, field)?;
        return Ok((!text.is_empty()).then_some(MemoUpdate::Text(text)));
    }

    let bytes = binary_value(value, field)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    if memo_format == MemoFormat::Dbase3
        && (bytes.last() == Some(&EOF_MARKER)
            || bytes
                .windows(2)
                .any(|pair| pair == [EOF_MARKER, EOF_MARKER]))
    {
        return Err(DbfError::Invalid(
            "dBASE III binary data cannot contain its 0x1a1a terminator".into(),
        ));
    }
    Ok(Some(MemoUpdate::Binary(bytes)))
}

pub(crate) fn binary_value(value: &Value, field: &FieldDescriptor) -> Result<Vec<u8>, DbfError> {
    let Some(text) = value.as_str() else {
        if value.is_null() {
            return Ok(Vec::new());
        }
        return Err(DbfError::Invalid(format!(
            "binary field {} requires an even-length hexadecimal string or null",
            field.name
        )));
    };
    let bytes = text.as_bytes();
    if bytes.len() % 2 != 0 {
        return Err(DbfError::Invalid(format!(
            "binary field {} requires an even-length hexadecimal string",
            field.name
        )));
    }
    bytes
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_digit(pair[0]);
            let low = hex_digit(pair[1]);
            match (high, low) {
                (Some(high), Some(low)) => Ok(high << 4 | low),
                _ => Err(DbfError::Invalid(format!(
                    "binary field {} contains a non-hexadecimal character",
                    field.name
                ))),
            }
        })
        .collect()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn empty_memo_value(field: &FieldDescriptor) -> Value {
    if field.length == 4 {
        Value::Number(0.into())
    } else {
        Value::String(String::new())
    }
}

pub(crate) fn encode_memo_pointer(
    field: &FieldDescriptor,
    block: u32,
    _format: MemoFormat,
) -> Result<Vec<u8>, DbfError> {
    if field.length == 4 {
        return Ok(block.to_le_bytes().to_vec());
    }
    let text = block.to_string();
    let length = usize::from(field.length);
    if text.len() > length {
        return Err(DbfError::Invalid(format!(
            "memo block number for {} exceeds field width {}",
            field.name, field.length
        )));
    }
    let mut pointer = vec![b' '; length];
    let start = length - text.len();
    pointer[start..].copy_from_slice(text.as_bytes());
    Ok(pointer)
}
