use super::super::checksum::crc32c;
use super::super::{HEADER_SIZE, XbfError, XbfLimits};

pub(super) const ENTRY_SIZE: usize = 28;
const OPTIONAL_FLAG: u32 = 0x01;

pub(super) fn validate_extensions(
    bytes: &[u8],
    header: &[u8],
    data_end: u64,
    limits: &XbfLimits,
) -> Result<(), XbfError> {
    let directory = &header[HEADER_SIZE..];
    let mut previous_type = 0;
    let mut expected_offset = data_end;
    for entry in directory.chunks_exact(ENTRY_SIZE) {
        let section_type = get_u32(entry, 0)?;
        if section_type == 0 || section_type <= previous_type {
            return Err(XbfError::Invalid(
                "XBF extension types are zero, duplicated, or unordered".into(),
            ));
        }
        previous_type = section_type;

        let flags = get_u32(entry, 4)?;
        if flags & !OPTIONAL_FLAG != 0 {
            return Err(XbfError::Invalid(
                "XBF extension contains unsupported flags".into(),
            ));
        }
        if flags & OPTIONAL_FLAG == 0 {
            return Err(XbfError::Invalid(
                "XBF contains an unknown required extension section".into(),
            ));
        }

        let offset = get_u64(entry, 8)?;
        let length = get_u64(entry, 16)?;
        if length > limits.max_section_size as u64 {
            return Err(XbfError::Invalid(
                "XBF extension section exceeds the configured limit".into(),
            ));
        }
        if offset != expected_offset {
            return Err(XbfError::Invalid(
                "XBF extension sections are unordered, overlapping, or have gaps".into(),
            ));
        }
        let end = offset
            .checked_add(length)
            .ok_or_else(|| XbfError::Invalid("XBF extension bounds overflow".into()))?;
        let start = usize::try_from(offset)
            .map_err(|_| XbfError::Invalid("XBF extension offset overflows usize".into()))?;
        let end_usize = usize::try_from(end)
            .map_err(|_| XbfError::Invalid("XBF extension end overflows usize".into()))?;
        let payload = bytes
            .get(start..end_usize)
            .ok_or_else(|| XbfError::Invalid("XBF extension is outside the file".into()))?;
        if get_u32(entry, 24)? != crc32c(payload) {
            return Err(XbfError::Invalid(
                "XBF extension checksum does not match".into(),
            ));
        }
        expected_offset = end;
    }

    if expected_offset != bytes.len() as u64 {
        return Err(XbfError::Invalid(
            "XBF sections are unordered, overlapping, or have trailing bytes".into(),
        ));
    }
    Ok(())
}

fn get_u32(bytes: &[u8], offset: usize) -> Result<u32, XbfError> {
    let raw = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| XbfError::Invalid("XBF integer is truncated".into()))?;
    Ok(u32::from_le_bytes(raw.try_into().expect("checked length")))
}

fn get_u64(bytes: &[u8], offset: usize) -> Result<u64, XbfError> {
    let raw = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| XbfError::Invalid("XBF integer is truncated".into()))?;
    Ok(u64::from_le_bytes(raw.try_into().expect("checked length")))
}
