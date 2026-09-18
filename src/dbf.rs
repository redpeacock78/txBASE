use serde_json::{Map, Number, Value};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::Path;

const CLASSIC_HEADER_SIZE: usize = 32;
const CLASSIC_DESCRIPTOR_SIZE: usize = 32;
const LEVEL7_HEADER_SIZE: usize = 68;
const LEVEL7_DESCRIPTOR_SIZE: usize = 48;
const FIELD_TERMINATOR: u8 = 0x0d;
const ACTIVE_RECORD: u8 = 0x20;
const DELETED_RECORD: u8 = 0x2a;

#[derive(Debug)]
pub enum DbfError {
    Io(std::io::Error),
    Invalid(String),
}

impl Display for DbfError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::Invalid(message) => write!(f, "invalid DBF: {message}"),
        }
    }
}

impl Error for DbfError {}

impl From<std::io::Error> for DbfError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbfHeader {
    pub version: u8,
    pub last_update: [u8; 3],
    pub record_count: u32,
    pub header_length: u16,
    pub record_length: u16,
    pub language_driver: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDescriptor {
    pub name: String,
    pub field_type: u8,
    pub length: u8,
    pub decimal_count: u8,
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DbfRecord {
    pub number: usize,
    pub deleted: bool,
    pub values: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DbfTable {
    pub header: DbfHeader,
    pub fields: Vec<FieldDescriptor>,
    records: Vec<DbfRecord>,
}

impl DbfTable {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, DbfError> {
        Self::from_bytes(&fs::read(path)?)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DbfError> {
        if bytes.len() < CLASSIC_HEADER_SIZE {
            return Err(DbfError::Invalid("header is truncated".into()));
        }

        let header = DbfHeader {
            version: bytes[0],
            last_update: [bytes[1], bytes[2], bytes[3]],
            record_count: read_u32(bytes, 4)?,
            header_length: read_u16(bytes, 8)?,
            record_length: read_u16(bytes, 10)?,
            language_driver: bytes[29],
        };
        let header_length = usize::from(header.header_length);
        let record_length = usize::from(header.record_length);

        if header_length < CLASSIC_HEADER_SIZE + 1 || header_length > bytes.len() {
            return Err(DbfError::Invalid(format!(
                "header length {} is outside the file",
                header.header_length
            )));
        }
        if record_length == 0 {
            return Err(DbfError::Invalid("record length is zero".into()));
        }

        let (descriptor_start, descriptor_size) = if header.version & 0x07 == 4 {
            (LEVEL7_HEADER_SIZE, LEVEL7_DESCRIPTOR_SIZE)
        } else {
            (CLASSIC_HEADER_SIZE, CLASSIC_DESCRIPTOR_SIZE)
        };
        if descriptor_start >= header_length {
            return Err(DbfError::Invalid("field descriptor area is missing".into()));
        }

        let terminator = (descriptor_start..header_length)
            .step_by(descriptor_size)
            .find(|offset| bytes[*offset] == FIELD_TERMINATOR)
            .ok_or_else(|| DbfError::Invalid("field descriptor terminator is missing".into()))?;

        let fields = parse_fields(bytes, descriptor_start, terminator, descriptor_size)?;
        let expected_record_length = fields
            .iter()
            .map(|field| field.length as usize)
            .sum::<usize>()
            .checked_add(1)
            .ok_or_else(|| DbfError::Invalid("record length overflows usize".into()))?;
        if expected_record_length != record_length {
            return Err(DbfError::Invalid(format!(
                "record length {} does not match fields ({expected_record_length})",
                header.record_length
            )));
        }

        let record_start = header_length;
        let record_bytes = usize::try_from(header.record_count)
            .ok()
            .and_then(|count| count.checked_mul(record_length))
            .ok_or_else(|| DbfError::Invalid("record area overflows usize".into()))?;
        let record_end = record_start
            .checked_add(record_bytes)
            .ok_or_else(|| DbfError::Invalid("record area overflows usize".into()))?;
        if record_end > bytes.len() {
            return Err(DbfError::Invalid("record area is truncated".into()));
        }

        let record_count = usize::try_from(header.record_count)
            .map_err(|_| DbfError::Invalid("record count overflows usize".into()))?;
        let mut records = Vec::with_capacity(record_count);
        for number in 1..=record_count {
            let start = record_start + (number - 1) * record_length;
            let record = &bytes[start..start + record_length];
            let deleted = match record[0] {
                ACTIVE_RECORD => false,
                DELETED_RECORD => true,
                marker => {
                    return Err(DbfError::Invalid(format!(
                        "record {number} has unknown deletion marker 0x{marker:02x}"
                    )));
                }
            };

            let mut values = Map::new();
            for field in &fields {
                let start = field.offset;
                let end = start + field.length as usize;
                values.insert(
                    field.name.clone(),
                    decode_field(field.field_type, &record[start..end]),
                );
            }
            records.push(DbfRecord {
                number,
                deleted,
                values,
            });
        }

        Ok(Self {
            header,
            fields,
            records,
        })
    }

    pub fn records(&self) -> &[DbfRecord] {
        &self.records
    }

    pub fn active_records(&self) -> impl Iterator<Item = &DbfRecord> {
        self.records.iter().filter(|record| !record.deleted)
    }

    pub fn active_json(&self) -> Vec<Value> {
        self.active_records()
            .map(|record| Value::Object(record.values.clone()))
            .collect()
    }

    pub fn active_record(&self, number: usize) -> Option<&DbfRecord> {
        number
            .checked_sub(1)
            .and_then(|index| self.records.get(index).filter(|record| !record.deleted))
    }
}

fn parse_fields(
    bytes: &[u8],
    start: usize,
    end: usize,
    descriptor_size: usize,
) -> Result<Vec<FieldDescriptor>, DbfError> {
    if (end - start) % descriptor_size != 0 {
        return Err(DbfError::Invalid(
            "field descriptor area is misaligned".into(),
        ));
    }

    let (name_size, type_offset, length_offset, decimal_offset) =
        if descriptor_size == CLASSIC_DESCRIPTOR_SIZE {
            (11, 11, 16, 17)
        } else {
            (32, 32, 33, 34)
        };
    let mut names = BTreeSet::new();
    let mut offset = 1;
    let mut fields = Vec::with_capacity((end - start) / descriptor_size);
    for descriptor in bytes[start..end].chunks_exact(descriptor_size) {
        let name_end = descriptor[..name_size]
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(name_size);
        let name = String::from_utf8_lossy(&descriptor[..name_end])
            .trim()
            .to_owned();
        if name.is_empty() || !names.insert(name.clone()) {
            return Err(DbfError::Invalid(
                "field names must be non-empty and unique".into(),
            ));
        }
        let length = descriptor[length_offset];
        if length == 0 {
            return Err(DbfError::Invalid(format!("field {name} has zero width")));
        }
        fields.push(FieldDescriptor {
            name,
            field_type: descriptor[type_offset],
            length,
            decimal_count: descriptor[decimal_offset],
            offset,
        });
        offset = offset
            .checked_add(length as usize)
            .ok_or_else(|| DbfError::Invalid("field offsets overflow usize".into()))?;
    }
    Ok(fields)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, DbfError> {
    let bytes = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| DbfError::Invalid("header is truncated".into()))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, DbfError> {
    let bytes = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| DbfError::Invalid("header is truncated".into()))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn decode_field(field_type: u8, bytes: &[u8]) -> Value {
    match field_type.to_ascii_uppercase() {
        b'C' | b'D' | b'B' | b'G' | b'M' => Value::String(text(bytes)),
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
        _ => Value::String(text(bytes)),
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_end_matches([' ', '\0'])
        .to_owned()
}

fn numeric(bytes: &[u8]) -> Value {
    let value = text(bytes).trim().to_owned();
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

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        include_str!("../tests/fixtures/users.dbf.hex")
            .split_whitespace()
            .map(|token| u8::from_str_radix(token, 16).unwrap())
            .collect()
    }

    #[test]
    fn reads_header_fields_and_active_records() {
        let table = DbfTable::from_bytes(&fixture()).unwrap();

        assert_eq!(table.header.record_count, 2);
        assert_eq!(table.header.record_length, 18);
        assert_eq!(table.fields[0].name, "ID");
        assert_eq!(table.fields[1].offset, 4);
        assert!(table.records()[1].deleted);
        assert_eq!(table.active_json().len(), 1);
        assert_eq!(table.active_json()[0]["NAME"], "Alice");
        assert_eq!(table.active_json()[0]["AGE"], 29);
    }

    #[test]
    fn rejects_truncated_records() {
        let mut bytes = fixture();
        bytes.pop();
        bytes.pop();
        assert!(matches!(
            DbfTable::from_bytes(&bytes),
            Err(DbfError::Invalid(message)) if message == "record area is truncated"
        ));
    }

    #[test]
    fn rejects_unknown_deletion_markers() {
        let mut bytes = fixture();
        bytes[161] = b'!';
        assert!(matches!(
            DbfTable::from_bytes(&bytes),
            Err(DbfError::Invalid(message)) if message.contains("unknown deletion marker")
        ));
    }

    #[test]
    fn reads_level7_descriptor_layout() {
        let mut bytes = vec![0; 122];
        bytes[0] = 0x04;
        bytes[4] = 1;
        bytes[8] = 117;
        bytes[10] = 4;
        bytes[68] = b'V';
        bytes[100] = b'C';
        bytes[101] = 3;
        bytes[116] = FIELD_TERMINATOR;
        bytes[117..121].copy_from_slice(b" yes");
        bytes[121] = 0x1a;

        let table = DbfTable::from_bytes(&bytes).unwrap();
        assert_eq!(table.fields[0].name, "V");
        assert_eq!(table.active_json()[0]["V"], "yes");
    }

    #[test]
    fn decodes_float_fields_as_numbers() {
        assert_eq!(decode_field(b'F', b" 1.5"), serde_json::json!(1.5));
    }
}
