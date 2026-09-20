use super::{HEADER_SIZE, XbfError, XbfLimits};

mod decode;
mod encode;

pub use decode::{decode, decode_with_limits};
pub use encode::{encode, encode_with_limits};

const VERSION_MAJOR: u16 = 1;
const VERSION_MINOR: u16 = 0;
const HEADER_FLAGS: u32 = 0;
const DIRECTORY_ENTRY_SIZE: usize = 24;

const HEADER_LENGTH_OFFSET: usize = 12;
const SCHEMA_OFFSET: usize = 16;
const SCHEMA_LENGTH: usize = 24;
const SCHEMA_CRC: usize = 32;
const DIRECTORY_OFFSET: usize = 36;
const DIRECTORY_LENGTH: usize = 44;
const DIRECTORY_CRC: usize = 52;
const DATA_OFFSET: usize = 56;
const DATA_LENGTH: usize = 64;
const DATA_CRC: usize = 72;
const RECORD_COUNT: usize = 76;
const GENERATION: usize = 84;
const HEADER_CRC: usize = 92;
const RESERVED: usize = 96;

fn validate_limits(limits: &XbfLimits) -> Result<(), XbfError> {
    if limits.max_file_size < HEADER_SIZE
        || limits.max_section_size == 0
        || limits.max_record_size == 0
        || limits.max_value_size == 0
        || limits.max_field_name == 0
        || limits.max_fields == 0
        || limits.max_records == 0
    {
        return Err(XbfError::Invalid(
            "XBF limits contain a zero or undersized bound".into(),
        ));
    }
    Ok(())
}

pub(super) fn check_section_size(
    length: usize,
    limits: &XbfLimits,
    label: &str,
) -> Result<(), XbfError> {
    if length > limits.max_section_size {
        return Err(XbfError::Invalid(format!(
            "{label} section exceeds the configured limit"
        )));
    }
    Ok(())
}

pub(super) fn usize_from_u64(value: u64, label: &str) -> Result<usize, XbfError> {
    usize::try_from(value).map_err(|_| XbfError::Invalid(format!("{label} overflows usize")))
}

pub(super) fn usize_from_u32(value: u32, label: &str) -> Result<usize, XbfError> {
    usize::try_from(value).map_err(|_| XbfError::Invalid(format!("{label} overflows usize")))
}

pub(super) fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

pub(super) fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

pub(super) fn take<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
    length: usize,
) -> Result<&'a [u8], XbfError> {
    let end = cursor
        .checked_add(length)
        .ok_or_else(|| XbfError::Invalid("XBF payload length overflows".into()))?;
    let value = bytes
        .get(*cursor..end)
        .ok_or_else(|| XbfError::Invalid("XBF payload is truncated".into()))?;
    *cursor = end;
    Ok(value)
}

pub(super) fn take_u8(bytes: &[u8], cursor: &mut usize) -> Result<u8, XbfError> {
    Ok(take(bytes, cursor, 1)?[0])
}

pub(super) fn take_u16(bytes: &[u8], cursor: &mut usize) -> Result<u16, XbfError> {
    Ok(u16::from_le_bytes(
        take(bytes, cursor, 2)?.try_into().expect("checked length"),
    ))
}

pub(super) fn take_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, XbfError> {
    Ok(u32::from_le_bytes(
        take(bytes, cursor, 4)?.try_into().expect("checked length"),
    ))
}
