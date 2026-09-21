use super::{
    CLASSIC_DESCRIPTOR_SIZE, CLASSIC_HEADER_SIZE, DbfError, DbfTable, EOF_MARKER, FIELD_TERMINATOR,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbfFieldSpec {
    pub name: String,
    pub field_type: u8,
    pub length: u8,
    pub decimal_count: u8,
}

impl DbfFieldSpec {
    pub fn new(name: impl Into<String>, field_type: u8, length: u8, decimal_count: u8) -> Self {
        Self {
            name: name.into(),
            field_type,
            length,
            decimal_count,
        }
    }
}

impl DbfTable {
    pub fn empty(fields: &[DbfFieldSpec]) -> Result<Self, DbfError> {
        if fields.is_empty() {
            return Err(DbfError::Invalid(
                "an empty DBF needs at least one field".into(),
            ));
        }
        let header_length = CLASSIC_HEADER_SIZE
            .checked_add(
                fields
                    .len()
                    .checked_mul(CLASSIC_DESCRIPTOR_SIZE)
                    .ok_or_else(|| DbfError::Invalid("DBF header length overflows".into()))?,
            )
            .and_then(|length| length.checked_add(1))
            .ok_or_else(|| DbfError::Invalid("DBF header length overflows".into()))?;
        for field in fields {
            validate_field(field)?;
        }
        let record_length = fields
            .iter()
            .try_fold(1usize, |length, field| {
                length.checked_add(usize::from(field.length))
            })
            .ok_or_else(|| DbfError::Invalid("DBF record length overflows".into()))?;
        let header_length = u16::try_from(header_length)
            .map_err(|_| DbfError::Invalid("DBF header length exceeds u16".into()))?;
        let record_length = u16::try_from(record_length)
            .map_err(|_| DbfError::Invalid("DBF record length exceeds u16".into()))?;

        let mut bytes = vec![0; usize::from(header_length)];
        bytes[0] = 0x03;
        bytes[1..4].copy_from_slice(&[0x7e, 0x09, 0x14]);
        bytes[8..10].copy_from_slice(&header_length.to_le_bytes());
        bytes[10..12].copy_from_slice(&record_length.to_le_bytes());
        let mut offset = CLASSIC_HEADER_SIZE;
        for field in fields {
            bytes[offset..offset + field.name.len()].copy_from_slice(field.name.as_bytes());
            bytes[offset + 11] = field.field_type;
            bytes[offset + 16] = field.length;
            bytes[offset + 17] = field.decimal_count;
            offset += CLASSIC_DESCRIPTOR_SIZE;
        }
        bytes[usize::from(header_length) - 1] = FIELD_TERMINATOR;
        bytes.push(EOF_MARKER);
        Self::from_bytes(&bytes)
    }
}

fn validate_field(field: &DbfFieldSpec) -> Result<(), DbfError> {
    if field.name.is_empty() || field.name.len() > 10 || !field.name.is_ascii() {
        return Err(DbfError::Invalid(format!(
            "DBF field name must be 1-10 ASCII bytes: {}",
            field.name
        )));
    }
    if field.length == 0 {
        return Err(DbfError::Invalid(format!(
            "DBF field {} has zero width",
            field.name
        )));
    }
    if field.decimal_count > field.length {
        return Err(DbfError::Invalid(format!(
            "DBF field {} has too many decimal places",
            field.name
        )));
    }
    if !matches!(
        field.field_type.to_ascii_uppercase(),
        b'C' | b'D' | b'F' | b'I' | b'L' | b'N'
    ) {
        return Err(DbfError::Invalid(format!(
            "unsupported initialized DBF field type: {}",
            field.field_type as char
        )));
    }
    Ok(())
}
