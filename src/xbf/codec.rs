use super::checksum::crc32c;
use super::schema::{decode_schema, encode_schema, validate_constraints};
use super::values::{decode_record, encode_record};
use super::{HEADER_SIZE, MAGIC, XbfError, XbfLimits, XbfRecord, XbfTable};

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

pub fn encode(table: &XbfTable) -> Result<Vec<u8>, XbfError> {
    encode_with_limits(table, &XbfLimits::default())
}

pub fn encode_with_limits(table: &XbfTable, limits: &XbfLimits) -> Result<Vec<u8>, XbfError> {
    validate_limits(limits)?;
    if table.fields.len() > limits.max_fields {
        return Err(XbfError::Invalid(
            "field count exceeds the configured limit".into(),
        ));
    }
    if table.records.len() > limits.max_records {
        return Err(XbfError::Invalid(
            "record count exceeds the configured limit".into(),
        ));
    }

    let schema = encode_schema(&table.fields, limits)?;
    let directory_length = table
        .records
        .len()
        .checked_mul(DIRECTORY_ENTRY_SIZE)
        .ok_or_else(|| XbfError::Invalid("record directory length overflows".into()))?;
    let mut directory = Vec::with_capacity(directory_length);
    let mut data = Vec::new();
    let schema_offset = HEADER_SIZE;
    let directory_offset = checked_add(schema_offset, schema.len(), "schema section")?;
    let data_offset = checked_add(directory_offset, directory_length, "record directory")?;

    for record in &table.records {
        let payload = encode_record(&table.fields, record, limits)?;
        let offset = checked_add(data_offset, data.len(), "record data")?;
        push_u64(&mut directory, u64_from_usize(offset, "record offset")?);
        push_u64(
            &mut directory,
            u64_from_usize(payload.len(), "record payload length")?,
        );
        push_u32(&mut directory, if record.deleted { 0x01 } else { 0 });
        push_u32(&mut directory, 0);
        data.extend_from_slice(&payload);
    }

    validate_constraints(&table.fields, &table.records)?;
    check_section_size(schema.len(), limits, "schema")?;
    check_section_size(directory.len(), limits, "record directory")?;
    check_section_size(data.len(), limits, "record data")?;

    let mut header = [0; HEADER_SIZE];
    header[..MAGIC.len()].copy_from_slice(&MAGIC);
    put_u16(&mut header, 4, VERSION_MAJOR);
    put_u16(&mut header, 6, VERSION_MINOR);
    put_u32(&mut header, 8, HEADER_FLAGS);
    put_u32(&mut header, HEADER_LENGTH_OFFSET, HEADER_SIZE as u32);
    put_u64(
        &mut header,
        SCHEMA_OFFSET,
        u64_from_usize(schema_offset, "schema offset")?,
    );
    put_u64(
        &mut header,
        SCHEMA_LENGTH,
        u64_from_usize(schema.len(), "schema length")?,
    );
    put_u32(&mut header, SCHEMA_CRC, crc32c(&schema));
    put_u64(
        &mut header,
        DIRECTORY_OFFSET,
        u64_from_usize(directory_offset, "record directory offset")?,
    );
    put_u64(
        &mut header,
        DIRECTORY_LENGTH,
        u64_from_usize(directory.len(), "record directory length")?,
    );
    put_u32(&mut header, DIRECTORY_CRC, crc32c(&directory));
    put_u64(
        &mut header,
        DATA_OFFSET,
        u64_from_usize(data_offset, "record data offset")?,
    );
    put_u64(
        &mut header,
        DATA_LENGTH,
        u64_from_usize(data.len(), "record data length")?,
    );
    put_u32(&mut header, DATA_CRC, crc32c(&data));
    put_u64(
        &mut header,
        RECORD_COUNT,
        u64_from_usize(table.records.len(), "record count")?,
    );
    put_u64(&mut header, GENERATION, table.generation);
    put_u32(&mut header, HEADER_CRC, 0);
    put_u32(&mut header, RESERVED, 0);
    let header_crc = crc32c(&header);
    put_u32(&mut header, HEADER_CRC, header_crc);

    let total_size = checked_add(
        checked_add(
            checked_add(HEADER_SIZE, schema.len(), "XBF file")?,
            directory.len(),
            "XBF file",
        )?,
        data.len(),
        "XBF file",
    )?;
    if total_size > limits.max_file_size {
        return Err(XbfError::Invalid(
            "encoded XBF file exceeds the configured limit".into(),
        ));
    }
    let mut bytes = Vec::with_capacity(total_size);
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(&schema);
    bytes.extend_from_slice(&directory);
    bytes.extend_from_slice(&data);
    Ok(bytes)
}

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

fn checked_add(left: usize, right: usize, label: &str) -> Result<usize, XbfError> {
    left.checked_add(right)
        .ok_or_else(|| XbfError::Invalid(format!("{label} length overflows")))
}

fn u64_from_usize(value: usize, label: &str) -> Result<u64, XbfError> {
    u64::try_from(value).map_err(|_| XbfError::Invalid(format!("{label} overflows u64")))
}

pub(super) fn usize_from_u64(value: u64, label: &str) -> Result<usize, XbfError> {
    usize::try_from(value).map_err(|_| XbfError::Invalid(format!("{label} overflows usize")))
}

pub(super) fn usize_from_u32(value: u32, label: &str) -> Result<usize, XbfError> {
    usize::try_from(value).map_err(|_| XbfError::Invalid(format!("{label} overflows usize")))
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

pub(super) fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
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
