use super::super::checksum::crc32c;
use super::super::schema::{encode_schema, validate_constraints};
use super::super::values::encode_record;
use super::super::{HEADER_SIZE, MAGIC, XbfError, XbfLimits, XbfTable};
use super::{
    DATA_CRC, DATA_LENGTH, DATA_OFFSET, DIRECTORY_CRC, DIRECTORY_ENTRY_SIZE, DIRECTORY_LENGTH,
    DIRECTORY_OFFSET, GENERATION, HEADER_CRC, HEADER_FLAGS, HEADER_LENGTH_OFFSET, RECORD_COUNT,
    RESERVED, SCHEMA_CRC, SCHEMA_LENGTH, SCHEMA_OFFSET, VERSION_MAJOR, VERSION_MINOR,
    check_section_size, push_u32, validate_limits,
};

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
    check_section_size(directory_length, limits, "record directory")?;
    let schema_offset = HEADER_SIZE;
    let directory_offset = checked_add(schema_offset, schema.len(), "schema section")?;
    let data_offset = checked_add(directory_offset, directory_length, "record directory")?;
    if data_offset > limits.max_file_size {
        return Err(XbfError::Invalid(
            "encoded XBF file exceeds the configured limit".into(),
        ));
    }
    let mut directory = Vec::with_capacity(directory_length);
    let mut data = Vec::new();

    for record in &table.records {
        let payload = encode_record(&table.fields, record, limits)?;
        let next_data_length = checked_add(data.len(), payload.len(), "record data")?;
        check_section_size(next_data_length, limits, "record data")?;
        let next_file_size = checked_add(data_offset, next_data_length, "XBF file")?;
        if next_file_size > limits.max_file_size {
            return Err(XbfError::Invalid(
                "encoded XBF file exceeds the configured limit".into(),
            ));
        }
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

fn checked_add(left: usize, right: usize, label: &str) -> Result<usize, XbfError> {
    left.checked_add(right)
        .ok_or_else(|| XbfError::Invalid(format!("{label} length overflows")))
}

fn u64_from_usize(value: usize, label: &str) -> Result<u64, XbfError> {
    u64::try_from(value).map_err(|_| XbfError::Invalid(format!("{label} overflows u64")))
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

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
