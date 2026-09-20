use super::super::checksum::crc32c;
use super::super::schema::{decode_schema, validate_constraints};
use super::super::values::decode_record;
use super::super::{HEADER_SIZE, MAGIC, XbfError, XbfLimits, XbfRecord, XbfTable};
use super::{
    DATA_CRC, DATA_LENGTH, DATA_OFFSET, DIRECTORY_CRC, DIRECTORY_ENTRY_SIZE, DIRECTORY_LENGTH,
    DIRECTORY_OFFSET, GENERATION, HEADER_CRC, HEADER_FLAGS, HEADER_LENGTH_OFFSET, RECORD_COUNT,
    RESERVED, SCHEMA_CRC, SCHEMA_LENGTH, SCHEMA_OFFSET, VERSION_MAJOR, VERSION_MINOR,
    usize_from_u64, validate_limits,
};

pub fn decode(bytes: &[u8]) -> Result<XbfTable, XbfError> {
    decode_with_limits(bytes, &XbfLimits::default())
}

pub fn decode_with_limits(bytes: &[u8], limits: &XbfLimits) -> Result<XbfTable, XbfError> {
    validate_limits(limits)?;
    if bytes.len() > limits.max_file_size {
        return Err(XbfError::Invalid(
            "XBF file exceeds the configured limit".into(),
        ));
    }
    if bytes.len() < HEADER_SIZE {
        return Err(XbfError::Invalid("XBF header is truncated".into()));
    }
    let header = &bytes[..HEADER_SIZE];
    if header[..MAGIC.len()] != MAGIC {
        return Err(XbfError::Invalid("XBF magic is invalid".into()));
    }
    let major = get_u16(header, 4)?;
    if major != VERSION_MAJOR {
        return Err(XbfError::Invalid(format!(
            "unsupported XBF major version {major}"
        )));
    }
    let minor = get_u16(header, 6)?;
    if minor != VERSION_MINOR {
        return Err(XbfError::Invalid(format!(
            "unsupported XBF minor version {minor}"
        )));
    }
    if get_u32(header, 8)? != HEADER_FLAGS {
        return Err(XbfError::Invalid(
            "XBF header contains unknown feature flags".into(),
        ));
    }
    if get_u32(header, HEADER_LENGTH_OFFSET)? != HEADER_SIZE as u32 {
        return Err(XbfError::Invalid("XBF header length is not 100".into()));
    }
    if get_u32(header, RESERVED)? != 0 {
        return Err(XbfError::Invalid(
            "XBF header reserved field is non-zero".into(),
        ));
    }
    let mut header_for_checksum = [0; HEADER_SIZE];
    header_for_checksum.copy_from_slice(header);
    header_for_checksum[HEADER_CRC..HEADER_CRC + 4].fill(0);
    if get_u32(header, HEADER_CRC)? != crc32c(&header_for_checksum) {
        return Err(XbfError::Invalid(
            "XBF header checksum does not match".into(),
        ));
    }

    let schema_offset = get_u64(header, SCHEMA_OFFSET)?;
    let schema_length = get_u64(header, SCHEMA_LENGTH)?;
    let directory_offset = get_u64(header, DIRECTORY_OFFSET)?;
    let directory_length = get_u64(header, DIRECTORY_LENGTH)?;
    let data_offset = get_u64(header, DATA_OFFSET)?;
    let data_length = get_u64(header, DATA_LENGTH)?;
    let schema = section(bytes, schema_offset, schema_length, limits, "schema")?;
    let directory = section(
        bytes,
        directory_offset,
        directory_length,
        limits,
        "record directory",
    )?;
    let data = section(bytes, data_offset, data_length, limits, "record data")?;
    let schema_end = schema_offset
        .checked_add(schema_length)
        .ok_or_else(|| XbfError::Invalid("schema section bounds overflow".into()))?;
    let directory_end = directory_offset
        .checked_add(directory_length)
        .ok_or_else(|| XbfError::Invalid("record directory bounds overflow".into()))?;
    let data_end = data_offset
        .checked_add(data_length)
        .ok_or_else(|| XbfError::Invalid("record data bounds overflow".into()))?;
    if schema_offset < HEADER_SIZE as u64
        || schema_end > directory_offset
        || directory_offset < HEADER_SIZE as u64
        || directory_end > data_offset
        || data_offset < HEADER_SIZE as u64
        || data_end != bytes.len() as u64
    {
        return Err(XbfError::Invalid(
            "XBF sections are unordered, overlapping, or have trailing bytes".into(),
        ));
    }
    if get_u32(header, SCHEMA_CRC)? != crc32c(schema) {
        return Err(XbfError::Invalid(
            "XBF schema checksum does not match".into(),
        ));
    }
    if get_u32(header, DIRECTORY_CRC)? != crc32c(directory) {
        return Err(XbfError::Invalid(
            "XBF record-directory checksum does not match".into(),
        ));
    }
    if get_u32(header, DATA_CRC)? != crc32c(data) {
        return Err(XbfError::Invalid(
            "XBF record-data checksum does not match".into(),
        ));
    }

    let fields = decode_schema(schema, limits)?;
    let record_count = usize_from_u64(get_u64(header, RECORD_COUNT)?, "record count")?;
    if record_count > limits.max_records {
        return Err(XbfError::Invalid(
            "record count exceeds the configured limit".into(),
        ));
    }
    let expected_directory_length = record_count
        .checked_mul(DIRECTORY_ENTRY_SIZE)
        .ok_or_else(|| XbfError::Invalid("record directory length overflows".into()))?;
    if directory.len() != expected_directory_length {
        return Err(XbfError::Invalid(
            "record directory length does not match record count".into(),
        ));
    }

    let mut entries = Vec::with_capacity(record_count);
    let mut cursor = 0;
    for _ in 0..record_count {
        let offset = get_u64(directory, cursor)?;
        let length = get_u64(directory, cursor + 8)?;
        let flags = get_u32(directory, cursor + 16)?;
        if flags & !0x01 != 0 || get_u32(directory, cursor + 20)? != 0 {
            return Err(XbfError::Invalid(
                "XBF record directory contains unknown flags or reserved data".into(),
            ));
        }
        let end = offset
            .checked_add(length)
            .ok_or_else(|| XbfError::Invalid("record directory entry overflows".into()))?;
        if offset < data_offset || end > data_end {
            return Err(XbfError::Invalid(
                "record directory entry is outside record data".into(),
            ));
        }
        if length > limits.max_record_size as u64 {
            return Err(XbfError::Invalid(
                "record payload exceeds the configured limit".into(),
            ));
        }
        entries.push((offset, length, flags & 0x01 != 0));
        cursor += DIRECTORY_ENTRY_SIZE;
    }
    let mut ordered_entries = entries.clone();
    ordered_entries.sort_by_key(|(offset, _, _)| *offset);
    for pair in ordered_entries.windows(2) {
        if pair[0]
            .0
            .checked_add(pair[0].1)
            .is_some_and(|end| end > pair[1].0)
        {
            return Err(XbfError::Invalid(
                "XBF record directory entries overlap".into(),
            ));
        }
    }

    let mut records = Vec::with_capacity(record_count);
    for (offset, length, deleted) in entries {
        let start = usize_from_u64(offset, "record offset")?;
        let end = usize_from_u64(
            offset
                .checked_add(length)
                .ok_or_else(|| XbfError::Invalid("record range overflows".into()))?,
            "record end",
        )?;
        let payload = bytes
            .get(start..end)
            .ok_or_else(|| XbfError::Invalid("record range is outside the XBF file".into()))?;
        records.push(XbfRecord {
            deleted,
            values: decode_record(payload, &fields, limits)?,
        });
    }
    validate_constraints(&fields, &records)?;
    Ok(XbfTable {
        generation: get_u64(header, GENERATION)?,
        fields,
        records,
    })
}

fn section<'a>(
    bytes: &'a [u8],
    offset: u64,
    length: u64,
    limits: &XbfLimits,
    label: &str,
) -> Result<&'a [u8], XbfError> {
    if length > limits.max_section_size as u64 {
        return Err(XbfError::Invalid(format!(
            "{label} section exceeds the configured limit"
        )));
    }
    let start = usize_from_u64(offset, &format!("{label} offset"))?;
    let length = usize_from_u64(length, &format!("{label} length"))?;
    let end = start
        .checked_add(length)
        .ok_or_else(|| XbfError::Invalid(format!("{label} section bounds overflow")))?;
    bytes
        .get(start..end)
        .ok_or_else(|| XbfError::Invalid(format!("{label} section is outside the XBF file")))
}

fn get_u16(bytes: &[u8], offset: usize) -> Result<u16, XbfError> {
    let raw = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| XbfError::Invalid("XBF integer is truncated".into()))?;
    Ok(u16::from_le_bytes(raw.try_into().expect("checked length")))
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
